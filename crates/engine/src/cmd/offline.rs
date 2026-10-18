//! Mail to keep offline (#143): mail of a Microsoft account older than its window goes back to
//! its headers and preview, and the content files it kept on disk go with it.
//!
//! - **What goes back** is mail outside its account's window ([`OfflineWindow`]) that is here
//!   whole: never mail the server lacks (a local folder's, a draft, the Outbox's, a held
//!   message), never one being shown, and not one the owner had fetched this session
//!   ([`Session::kept_for_now`]), which goes back the next time the mailbox is opened.
//! - **What stays** is everything but its content: where it is, read, flag, categories,
//!   importance, the server's state of it and the changes waiting to go up, its headers and a
//!   preview of its text. It is listed, sorted, threaded and searched by those as before.
//! - **Not an undo step:** content that went is fetched again when the message is opened, and
//!   an Undo of something else never brings it back.
//! - **On disk** its content files go once the mailbox is saved, at once rather than with the
//!   sweep hours later ([`crate::store::forget_when_saved`]), unless something else names them.

use std::collections::HashSet;

use mailcraft_model::{Id, Message, OfflineWindow};

use super::graph_down;
use crate::{Part, Session};

/// How long a preview is: as long as Microsoft Graph's own (`bodyPreview`).
const PREVIEW: usize = 255;

/// What thinning did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Thinned {
    /// Messages that went back to their headers and preview, and the bytes of content they let go of.
    pub messages: usize,
    pub bytes: u64,
    /// Messages outside the window still whole (past `most`): for the next call.
    pub left: usize,
}

/// The messages of `account` (and of the shared mailboxes it opens; every account for `None`)
/// that are whole and outside their window at the session's clock, and may go back.
fn outside(s: &Session, account: Option<Id>) -> Vec<Id> {
    let mb = &s.mailbox;
    let window = OfflineWindow::of(mb, s.now());
    if window.keeps_all() {
        return Vec::new();
    }
    let accounts: Option<HashSet<Id>> =
        account.map(|a| mb.accounts.iter().filter(|x| x.id == a || mb.owner_of(x.id).is_some_and(|o| o.id == a)).map(|x| x.id).collect());
    mb.messages
        .values()
        .filter(|m| !m.unfetched && accounts.as_ref().is_none_or(|set| set.contains(&m.account)))
        .filter(|m| !window.within(m) && !s.kept_for_now.contains(&m.id) && !s.sel.messages.contains(&m.id))
        .map(|m| m.id)
        .collect()
}

/// The bytes of content message `m` has, in memory and kept on disk.
pub(crate) fn content_bytes(m: &Message) -> u64 {
    let body = m.body.text.len() as u64 + m.body.html.as_ref().map_or(0, |h| h.len() as u64);
    let kept = m.kept.as_ref().map_or(0, |k| k.len());
    let files: u64 = m.attachments.iter().map(|a| a.size() as u64).sum();
    body.saturating_add(kept).saturating_add(files)
}

/// The preview message `id` keeps: the first words of its text (read from disk when only that
/// is kept there).
fn preview_of(s: &Session, id: Id) -> String {
    let Some(m) = s.mailbox.message(id) else { return String::new() };
    if !m.body.text.trim().is_empty() || m.body.html.is_some() {
        return crate::list::preview(m, PREVIEW);
    }
    let kept = m.kept.as_deref().filter(|k| !k.failed);
    if kept.is_some_and(|k| k.text.is_some())
        && let Ok(bytes) = s.content(id, Part::Text)
    {
        return mailcraft_html::text_preview(&String::from_utf8_lossy(&bytes), PREVIEW);
    }
    if kept.is_some_and(|k| k.html.is_some())
        && let Ok(bytes) = s.content(id, Part::Html)
    {
        return mailcraft_html::preview(&String::from_utf8_lossy(&bytes), PREVIEW);
    }
    String::new()
}

/// Mail of `account` (every account for `None`) outside its window goes back to its headers and
/// preview, `most` messages at most. Not an undo step.
pub fn thin(s: &mut Session, account: Option<Id>, most: usize) -> Thinned {
    let waiting = outside(s, account);
    let mut done = Thinned { left: waiting.len().saturating_sub(most), ..Default::default() };
    let mut thinned = HashSet::new();
    let mut refs: Vec<mailcraft_model::ContentRef> = Vec::new();
    for id in waiting.into_iter().take(most) {
        let preview = preview_of(s, id);
        let Some(m) = s.mailbox.messages.get(&id) else { continue };
        done.bytes = done.bytes.saturating_add(content_bytes(m));
        #[cfg(not(target_arch = "wasm32"))]
        if s.path.is_some() {
            refs.extend(crate::store::content_names(id, m));
        }
        if let Ok(m) = s.mailbox.message_mut(id) {
            graph_down::thin_message(m, preview);
            thinned.insert(id);
        }
    }
    done.messages = thinned.len();
    if done.messages == 0 {
        return done;
    }
    s.let_go(&thinned);
    s.mailbox_changed();
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(path) = s.path.clone() {
        crate::store::forget_when_saved(&path, s.rev(), refs);
    }
    #[cfg(target_arch = "wasm32")]
    drop(refs);
    log::info!("{} message(s) outside their mail to keep offline went back to their headers and preview ({} bytes)", done.messages, done.bytes);
    done
}

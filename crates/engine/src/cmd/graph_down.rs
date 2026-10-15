//! Mail down through Microsoft Graph (R3.2; R3 spec S5, S9–S11, S15).
//!
//! A folder's mail arrives as pages of its message delta, each taken in by itself with the link
//! that goes on from it, so that the mailbox file never holds a page without its link, nor a link
//! past what it has (S10). Everything is matched by server id (S5): a page that comes twice
//! changes nothing.
//!
//! - **A new message** comes as its headers and preview, not **fetched** (`CONTEXT.md`).
//! - **A message the server has changed** takes the server's read, flag, categories and
//!   importance, but for a property changed here since the server last said (ADR 0003): that
//!   goes up with R3.3, and nothing changed here is overwritten by coming down.
//! - **A message the server says left a folder** (`@removed`: deleted, or moved out) is marked
//!   gone and kept. Seen in another folder, it has moved there, with what was fetched of it. Only
//!   once every folder's delta reached its end in a run is what is still gone removed here.
//! - **A folder listed afresh** (its link was no longer taken, S11) marks all its messages gone
//!   first: what the complete new listing brings is kept, and the rest goes at the end of the run.

use std::collections::HashMap;

use mailcraft_model::sync::{GraphMessage, GraphProgress, MessagePage, ServerMessage};
use mailcraft_model::{Body, Id, Mailbox, Message};

/// Where each message a server knows is, by its server id: kept between pages, as finding one
/// otherwise means a walk through every message of the mailbox.
#[derive(Debug, Default)]
pub struct ServerIndex {
    ids: HashMap<String, Id>,
    /// How many messages the mailbox had when this was last right: anything else that added or
    /// removed one makes it walk again.
    count: usize,
}

impl ServerIndex {
    fn rebuild(&mut self, mb: &Mailbox) {
        self.ids = mb.messages.values().filter_map(|m| m.server.as_ref().map(|s| (s.id.clone(), m.id))).collect();
        self.count = mb.messages.len();
    }

    /// The message of `account` the server calls `id`.
    fn find(&mut self, mb: &Mailbox, account: Id, id: &str) -> Option<Id> {
        let is = |m: Id| mb.message(m).is_some_and(|m| m.account == account && m.server.as_ref().is_some_and(|s| s.id == id));
        if self.count != mb.messages.len() {
            self.rebuild(mb);
        }
        match self.ids.get(id).copied() {
            Some(m) if is(m) => Some(m),
            // Out of date (a message was changed by other means): look again, once.
            Some(_) => {
                self.rebuild(mb);
                self.ids.get(id).copied().filter(|m| is(*m))
            }
            None => None,
        }
    }

    fn note(&mut self, mb: &Mailbox, id: &str, m: Id) {
        self.ids.insert(id.to_string(), m);
        self.count = mb.messages.len();
    }
}

/// What taking in pages did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Down {
    pub new: usize,
    pub changed: usize,
    pub gone: usize,
    pub removed: usize,
}

/// Take in one page of a folder's delta, with its link. A page for a folder this mailbox doesn't
/// have (yet) is passed over, and so is its link: it comes again.
pub fn take_page(mb: &mut Mailbox, index: &mut ServerIndex, account: Id, page: &MessagePage, down: &mut Down) {
    let Some(folder) = mb.folder_by_server(account, &page.folder) else { return };
    if page.afresh {
        for m in mb.messages.values_mut() {
            if m.account == account
                && m.server.as_ref().is_some_and(|s| s.folder == page.folder && !s.gone)
                && let Some(s) = std::sync::Arc::make_mut(m).server.as_mut()
            {
                s.gone = true;
            }
        }
    }
    for id in &page.removed {
        let Some(m) = index.find(mb, account, id) else { continue };
        if let Ok(m) = mb.message_mut(m)
            && let Some(s) = m.server.as_mut().filter(|s| s.folder == page.folder && !s.gone)
        {
            s.gone = true;
            down.gone += 1;
        }
    }
    for there in &page.messages {
        if there.server.id.is_empty() {
            continue;
        }
        match index.find(mb, account, &there.server.id) {
            Some(m) => {
                if update(mb, account, m, folder, &page.folder, there) {
                    down.changed += 1;
                }
            }
            None => {
                if let Ok(m) = mb.add_message(new_message(folder, &page.folder, there)) {
                    index.note(mb, &there.server.id, m);
                    down.new += 1;
                }
            }
        }
    }
    if !page.link.is_empty()
        && let Ok(f) = mb.folder_mut(folder)
        && let Some(s) = f.server.as_mut()
    {
        s.delta = page.link.clone();
        s.caught_up = page.caught_up;
    }
}

/// A message the server has and this mailbox hasn't: its headers and preview.
fn new_message(folder: Id, server_folder: &str, there: &GraphMessage) -> Message {
    let server = ServerMessage { folder: server_folder.to_string(), gone: false, ..there.server.clone() };
    Message {
        folder,
        message_id: there.message_id.trim().trim_start_matches('<').trim_end_matches('>').to_string(),
        from: there.from.clone(),
        to: there.to.clone(),
        cc: there.cc.clone(),
        subject: there.subject.clone(),
        date: there.sent,
        received: there.received,
        body: Body { text: there.preview.clone(), html: None },
        read: server.read,
        flag: server.flag,
        importance: server.importance,
        categories: server.categories.clone(),
        server: Some(server),
        unfetched: true,
        ..Default::default()
    }
}

/// Bring a message this mailbox has in step with what the server says of it now. Whether
/// anything changed.
fn update(mb: &mut Mailbox, account: Id, id: Id, folder: Id, server_folder: &str, there: &GraphMessage) -> bool {
    let Some(m) = mb.message(id) else { return false };
    let before = m.clone();
    let last = m.server.clone().unwrap_or_default();
    // Where the server last had it, as this mailbox has that folder: a message still there here
    // follows the server; one moved here since stays where it was put (that move goes up, R3.3).
    let was_here = mb.folder_by_server(account, &last.folder);
    let follows = last.gone || was_here.is_none_or(|was| m.folder == was);
    let Ok(m) = mb.message_mut(id) else { return false };
    let now = &there.server;
    if m.read == last.read {
        m.read = now.read;
    }
    if m.flag == last.flag {
        m.flag = now.flag;
    }
    if m.categories == last.categories {
        m.categories = now.categories.clone();
    }
    if m.importance == last.importance {
        m.importance = now.importance;
    }
    if follows {
        m.folder = folder;
    }
    m.server = Some(ServerMessage { folder: server_folder.to_string(), gone: false, ..now.clone() });
    *m != before
}

/// Every folder's delta reached its end: what is still gone from every folder is gone here too.
pub fn sweep(mb: &mut Mailbox, account: Id, down: &mut Down) {
    let before = mb.messages.len();
    mb.messages.retain(|_, m| !(m.account == account && m.server.as_ref().is_some_and(|s| s.gone)));
    down.removed += before - mb.messages.len();
}

/// What the status bar says while a run is under way.
pub fn progress_words(mb: &Mailbox, account: Id, progress: &GraphProgress) -> String {
    let name = mb.folder_by_server(account, &progress.folder).and_then(|f| mb.folder(f)).map(|f| f.name.clone()).unwrap_or_default();
    let name: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
    if progress.total > 0 {
        format!("Receiving {name}: {} of {} · folder {} of {}", progress.read.min(progress.total), progress.total, progress.number, progress.folders)
    } else {
        format!("Receiving {name} · folder {} of {}", progress.number, progress.folders)
    }
}

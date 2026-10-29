// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mail down from an Exchange server account (#252, E6): what a run asks about, its pieces held
//! to sizes, and its folders and pages taken in.
//!
//! A result is handed over by a host and may say anything, so before anything of it is taken in
//! ([`bounded`], after [`mailcraft_model::sync::EwsMailResult::check`] has refused what is larger
//! than anything real):
//!
//! - **Ids** (a folder's, an item's) are held to the characters of base64 and the length the
//!   model allows; a folder or message with any other id is passed over.
//! - **Text** is put on one line and cut to what is shown (a subject and a `Message-ID` to a line of
//!   RFC 5322, a preview to 256 characters, a name to 256, an address to 320).
//! - **What an Exchange server doesn't say** is none of it: `answered` and `forwarded` are not
//!   properties of an item here, so they stay as they are.
//! - **A cut removes nothing.** A piece that had to be cut is not the end of a run that went well,
//!   and no read of a folder in it counts as a complete one.
//!
//! **Taking in** goes the way of a Graph run, by server id and with the same pieces
//! ([`super::graph_down::take_page`]): a page of a folder's changes is taken in together with the
//! sync state that goes on from it, so that the mailbox file never holds a page without its state
//! nor a state past what it has. The folder list is a delta: the folders created or updated, and
//! those deleted; read afresh it is every folder, and a folder it lacks is gone, as a complete
//! listing says ([`super::graph_mail::take_listing`]).

use std::collections::{HashMap, HashSet};

use mailcraft_model::sync::{
    EwsFolderDeltas, EwsMailResult, EwsMessagePage, FolderListing, GraphFetchResult, GraphFolder, GraphMessage, MAX_LISTED_FOLDERS, MessagePage,
};
use mailcraft_model::{FolderKind, Id, Mailbox};

use super::graph_down::{self, Down, ServerIndex};
use super::jmap_down::{MAX_BATCH, MAX_BUSY, MAX_FETCHED, MAX_MESSAGE, MAX_PREVIEW, MAX_TEXT, line, lines, people, person};

/// Longest name of a folder kept, in characters.
const MAX_NAME: usize = 255;
/// Most pages one piece brings, and most messages of the batch fetched whole.
const MAX_PAGES: usize = 1_000;

/// Whether `id` may be an Exchange server id (of a folder, an item or a change key): the characters
/// of base64 and its URL variant, at most [`mailcraft_model::sync::MAX_EWS_ID`] of them.
pub(crate) fn is_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= mailcraft_model::sync::MAX_EWS_ID && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_'))
}

/// A message of a page: whether it can be taken in at all (its id is one), and if so with its text
/// held to its sizes and what an Exchange server doesn't say taken out.
fn bounded_message(m: &mut GraphMessage, folder: &str) -> bool {
    if !is_id(&m.server.id) {
        return false;
    }
    let server = &mut m.server;
    server.folder = folder.to_owned();
    server.gone = false;
    (server.answered, server.forwarded) = (false, false);
    if !server.change_key.is_empty() && !is_id(&server.change_key) {
        server.change_key.clear();
    }
    server.categories.truncate(50);
    server.categories = server.categories.iter().map(|c| line(c, 255)).filter(|c| !c.is_empty()).collect();
    m.message_id = line(&m.message_id, MAX_TEXT);
    m.subject = line(&m.subject, MAX_TEXT);
    m.preview = line(&m.preview, MAX_PREVIEW);
    m.from = person(&m.from).unwrap_or_default();
    m.to = people(&m.to);
    m.cc = people(&m.cc);
    true
}

/// A page held to its sizes: whether it came whole.
fn bounded_page(page: &mut EwsMessagePage) -> bool {
    let folder = page.folder.clone();
    let before = page.messages.len() + page.removed.len() + page.read.len();
    page.messages.retain_mut(|m| bounded_message(m, &folder));
    page.removed.retain(|id| is_id(id));
    page.read.retain(|r| is_id(&r.id));
    before == page.messages.len() + page.removed.len() + page.read.len()
}

/// A folder of the folder list: whether it can be taken in, and if so as an Exchange server can
/// say it. A folder that lies in itself, or in a folder that is no id, is at the top; the server's
/// Outbox is a folder of its own, not the one mail is sent from here; and its counts pass no
/// folder over (Exchange runs have no such shortcut).
fn bounded_folder(f: &mut GraphFolder) -> bool {
    if !is_id(&f.id) {
        return false;
    }
    if f.parent == f.id || !is_id(&f.parent) {
        f.parent.clear();
    }
    if f.kind == FolderKind::Outbox {
        f.kind = FolderKind::Custom;
    }
    f.name = line(&f.name, MAX_NAME);
    f.counted = false;
    !f.name.is_empty()
}

/// The folder list of a result held to its sizes. Whether it came whole: a delta that had to be
/// cut is no complete listing, and neither is a read afresh.
fn bounded_deltas(deltas: &mut EwsFolderDeltas) -> bool {
    let mut whole = deltas.changed.len() <= MAX_LISTED_FOLDERS && deltas.deleted.len() <= MAX_LISTED_FOLDERS;
    deltas.changed.truncate(MAX_LISTED_FOLDERS);
    deltas.deleted.truncate(MAX_LISTED_FOLDERS);
    let before = deltas.changed.len();
    deltas.changed.retain_mut(bounded_folder);
    whole &= deltas.changed.len() == before;
    deltas.deleted.retain(|id| is_id(id));
    // (An afresh read that lists no folder is not a listing: an account always has an Inbox.)
    deltas.afresh &= whole && !deltas.changed.is_empty();
    whole
}

/// A piece of a run held to its sizes, as the module documentation says. What became of changes,
/// drafts, mail sent and mail placed is taken out: none of it goes up yet, so it answers nothing
/// that was asked. A piece is the end of a run that went well (`whole`) only if it is the last,
/// came whole, and tells of no trouble.
pub(crate) fn bounded(mut result: EwsMailResult) -> EwsMailResult {
    result.changes.clear();
    result.messages.clear();
    result.drafts.clear();
    result.sent.clear();
    result.placed.clear();
    let mut cut = false;
    // The folders whose pages came cut: where they go on from is not kept, so that the next run
    // reads them again from where it was.
    let mut cut_folders: HashSet<String> = HashSet::new();
    if let Some(deltas) = result.folders.as_mut() {
        let whole = bounded_deltas(deltas);
        cut |= !whole;
        // (Nor the folder list's state: it would go past folders that were left out.)
        if !whole {
            result.hierarchy.clear();
        }
    }
    cut |= result.pages.len() > MAX_PAGES;
    result.pages.truncate(MAX_PAGES);
    let before = result.pages.len();
    result.pages.retain(|page| is_id(&page.folder));
    cut |= result.pages.len() != before;
    for page in &mut result.pages {
        if !bounded_page(page) {
            cut = true;
            cut_folders.insert(page.folder.clone());
        }
    }
    // A page that came cut is no part of a fresh read that ends: nothing is removed for it.
    if cut {
        result.pages.iter_mut().for_each(|page| page.afresh = false);
    }
    result.states.retain(|s| is_id(&s.folder) && !cut_folders.contains(&s.folder));
    result.errors = lines(&result.errors);
    result.busy_for = result.busy_for.map(|seconds| seconds.min(MAX_BUSY));
    if let Some(progress) = result.progress.as_mut().filter(|progress| !is_id(&progress.folder)) {
        progress.folder.clear();
    }
    let trouble = !result.errors.is_empty() || result.stopped || result.auth_refused || result.busy_for.is_some() || !result.untrusted.is_empty();
    result.whole &= !cut && !trouble && !result.partial;
    result
}

/// What a batch of messages fetched whole brought, held to its sizes (as a JMAP batch is,
/// [`super::jmap_down::bounded_fetch`]), with Exchange's ids.
pub(crate) fn bounded_fetch(mut result: GraphFetchResult) -> GraphFetchResult {
    let mut refused = Vec::new();
    let (mut brought, mut full) = (0usize, false);
    result.fetched.truncate(MAX_FETCHED);
    result.fetched.retain(|f| {
        if !is_id(&f.id) || full {
            return false;
        }
        if f.mime.is_empty() || f.mime.len() > MAX_MESSAGE {
            refused.push(f.id.clone());
            return false;
        }
        full = brought > 0 && brought.saturating_add(f.mime.len()) > MAX_BATCH;
        brought = brought.saturating_add(f.mime.len());
        !full
    });
    for ids in [&mut result.missing, &mut result.failed] {
        ids.retain(|id| is_id(id));
        ids.truncate(MAX_FETCHED);
    }
    result.failed.extend(refused);
    result.errors = lines(&result.errors);
    result.busy_for = result.busy_for.map(|seconds| seconds.min(MAX_BUSY));
    result
}

/// Folders listed deepest last, so that a folder is made after the one it lies in.
fn parents_first(folders: &mut [GraphFolder]) {
    let parent_of: HashMap<&str, &str> = folders.iter().map(|f| (f.id.as_str(), f.parent.as_str())).collect();
    let depth = |f: &GraphFolder| {
        let (mut at, mut n) = (f.parent.as_str(), 0usize);
        while n < 64 && !at.is_empty() {
            match parent_of.get(at) {
                Some(up) => (at, n) = (up, n + 1),
                None => break,
            }
        }
        n
    };
    let depths: HashMap<String, usize> = folders.iter().map(|f| (f.id.clone(), depth(f))).collect();
    folders.sort_by_key(|f| depths.get(&f.id).copied().unwrap_or(0));
}

/// Take in the folder list of a run: the folders created or updated, and those deleted, by server
/// id. The server's folders this mailbox has that the delta doesn't mention are listed as they
/// were, so that the listing is a complete one (a folder that isn't in it is gone, with the mail in
/// it, but for the folders and mail that are only here: [`super::jmap_down::keep_what_is_only_here`]).
/// Where each folder goes on from is kept ([`super::graph_mail::take_listing`] leaves it).
pub(crate) fn take_folders(mb: &mut Mailbox, account: Id, deltas: &EwsFolderDeltas, taken: &mut super::graph_mail::Taken) -> usize {
    let mut folders: Vec<GraphFolder> = deltas.changed.clone();
    if !deltas.afresh {
        let named: HashSet<&str> = deltas.changed.iter().map(|f| f.id.as_str()).chain(deltas.deleted.iter().map(String::as_str)).collect();
        let stubs: Vec<GraphFolder> = mb
            .folders
            .iter()
            .filter(|f| f.account == account)
            .filter_map(|f| f.server.as_ref().map(|server| (f.kind, server)))
            .filter(|(_, server)| !named.contains(server.id.as_str()))
            .map(|(kind, server)| GraphFolder { id: server.id.clone(), parent: server.parent.clone(), name: server.name.clone(), kind, ..Default::default() })
            .collect();
        folders.extend(stubs);
    }
    parents_first(&mut folders);
    let listing = FolderListing { folders, complete: true };
    let removed = super::jmap_down::keep_what_is_only_here(mb, account, &listing);
    super::graph_mail::take_listing(mb, account, &listing, taken);
    removed
}

/// Take in one page of a folder's changes with the sync state that goes on from it. A page for a
/// folder this mailbox doesn't have (yet) is passed over, and so is its state: it comes again.
pub(crate) fn take_page(mb: &mut Mailbox, index: &mut ServerIndex, account: Id, page: &EwsMessagePage, state: Option<&str>, down: &mut Down) {
    let Some(folder) = mb.folder_by_server(account, &page.folder) else { return };
    // Whether the folder's first round had ended before this page ([`graph_down::take_page`] sorts
    // what arrives by it): the same for a link of Graph's and a sync state of Exchange's.
    let sorting = !page.afresh && mb.folder(folder).and_then(|f| f.server.as_ref()).is_some_and(|s| s.round_ended || s.caught_up);
    let as_page = MessagePage {
        folder: page.folder.clone(),
        messages: page.messages.clone(),
        removed: page.removed.clone(),
        read: page.read.clone(),
        link: String::new(),
        caught_up: false,
        afresh: page.afresh,
        counts: None,
    };
    graph_down::take_page(mb, index, account, &as_page, down);
    if let Some(state) = state
        && let Ok(f) = mb.folder_mut(folder)
        && let Some(s) = f.server.as_mut()
    {
        s.sync_state = state.to_owned();
        s.caught_up = page.last;
        s.round_ended = sorting || page.last;
        s.counts = None;
    }
}

/// The sync state a run's result gives for `folder`: the last it gives.
pub(crate) fn state_of<'r>(result: &'r EwsMailResult, folder: &str) -> Option<&'r str> {
    result.states.iter().rev().find(|s| s.folder == folder).map(|s| s.state.as_str())
}

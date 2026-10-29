// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A mailbox kept in step through Exchange Web Services: what a run is asked to do and what it
//! brings back (#252).
//!
//! **Shapes only for now**: nothing speaks the protocol yet, and a run that is asked for answers
//! that it is not built. The shapes are here so that the pieces can be built against them one at a
//! time, as an account's kind and settings ([`crate::AccountKind::Exchange`],
//! [`crate::EwsServer`]) are.
//!
//! The counterpart of [`JmapMailRequest`](super::JmapMailRequest) and
//! [`GraphMailRequest`](super::GraphMailRequest), beside them rather than bent into them. Plain
//! data: the engine builds an [`EwsMailRequest`] and takes in an [`EwsMailResult`] (both pure), and
//! a host with sockets does the talking in between, usually as a task that hands the answer over in
//! pieces. An Exchange server account's folders and messages are known to its server by the ids it
//! hands out ([`crate::AccountKind::server_ids`]); what carries no Graph meaning is used again
//! instead of being copied: [`FolderChange`], [`MessageChange`], [`DraftUp`], [`SendUp`],
//! [`PlaceUp`], [`FolderListing`], [`MessagePage`], [`GraphProgress`] and, for messages fetched
//! whole, what that brings ([`GraphFetchResult`](super::GraphFetchResult)).
//!
//! Where Graph and JMAP continue a folder from a link or a window, Exchange continues it from the
//! **sync state** the server gave at the end of the last read (`SyncFolderHierarchy`,
//! `SyncFolderItems`): an opaque string, kept by the host beside the folder and handed back
//! ([`EwsFolderState`]). A server that no longer knows one makes the host read the folder afresh.
//!
//! No type shows the server's address, the user name, the account's address, a sync state or a
//! message in debug output: counts only. The password that opens the server is handed to the host
//! beside the request and is in neither.

use serde::{Deserialize, Serialize};

use super::{
    DraftDone, DraftUp, FolderChange, FolderChangeDone, FolderListing, GraphProgress, MessageChange, MessageChangeDone, MessagePage, PlaceDone,
    PlaceUp, SendDone, SendUp, is_false,
};
use crate::Id;

/// Most folders whose sync state a request or an answer carries.
pub const MAX_EWS_STATES: usize = 10_000;

/// Longest sync state, in bytes, that is kept or handed back: the server's own are short, so a
/// longer one is not one.
pub const MAX_EWS_STATE: usize = 8192;

/// Which folders a run reads.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EwsScope {
    /// The Inbox and the folders named (the server ids of those on screen): the cheap run that
    /// keeps what the owner is looking at fresh.
    Light {
        #[serde(default)]
        also: Vec<String>,
    },
    /// Every folder of the listing.
    #[default]
    All,
}

impl EwsScope {
    /// A word for it that names no folder.
    fn describe(&self) -> String {
        match self {
            EwsScope::All => "all".into(),
            EwsScope::Light { also } => format!("light+{}", also.len()),
        }
    }
}

/// Where a folder's mail goes on from: the sync state the server gave at the end of the last read
/// of the folder (`SyncFolderItems`), by the folder's server id.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EwsFolderState {
    /// The folder's server id.
    pub folder: String,
    /// The sync state: opaque.
    pub state: String,
}

/// What a run for a [`crate::AccountKind::Exchange`] account is asked to do: the changes made here
/// go up, what waits in the Outbox is sent, the server's folders are listed if they should be, and
/// the folders in `scope` are read from their sync states.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EwsMailRequest {
    pub account: Id,
    /// The account's address: what the answer is checked against, and the mailbox whose requests
    /// are counted together.
    pub email: String,
    /// The address of the Exchange Web Services endpoint ([`crate::EwsServer::url`]). Never logged.
    pub url: String,
    /// The name the password is sent with ([`crate::EwsServer::username`]); empty means `email`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub username: String,
    /// The fingerprint of the one certificate the owner trusts for this account besides the
    /// system's ([`crate::EwsServer::trusted`]); empty when none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub trusted: String,
    /// List the server's folders in this run (`SyncFolderHierarchy` from `hierarchy`): the first
    /// run, a run that reads every folder, and the run after one that brought no listing.
    #[serde(skip_serializing_if = "is_false")]
    pub list_folders: bool,
    /// Where the folder hierarchy goes on from; empty reads it from its start.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hierarchy: String,
    /// Which folders are read.
    pub scope: EwsScope,
    /// Where each folder's mail goes on from; a folder that isn't here is read from its start.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<EwsFolderState>,
    /// The changes of folders made here, first.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FolderChange>,
    /// The changes of messages made here, after the folder changes and before the listing.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<MessageChange>,
    /// The drafts saved here that the server hasn't all of.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drafts: Vec<DraftUp>,
    /// What waits in the Outbox, sent after the drafts.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sends: Vec<SendUp>,
    /// Messages to make in its folders, moved or copied here from another mailbox of the same
    /// owner's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<PlaceUp>,
    /// The Inbox pass: the run is only the Inbox's changes from its sync state, and nothing else.
    #[serde(skip_serializing_if = "is_false")]
    pub inbox_pass: bool,
    /// A full sweep: every folder is read, none is passed over for having the same counts as
    /// before.
    #[serde(skip_serializing_if = "is_false")]
    pub full: bool,
    /// Nobody is waiting for this run: it gets the share of requests background work has.
    #[serde(skip_serializing_if = "is_false")]
    pub background: bool,
}

/// Messages of an Exchange server account to fetch whole (their MIME content), by server id, in
/// this order: the counterpart of [`GraphFetchRequest`](super::GraphFetchRequest).
///
/// It says where the server is, as an [`EwsMailRequest`] does, because a host that has to connect
/// for it has no other way to know. What it brings back is the same
/// [`GraphFetchResult`](super::GraphFetchResult) a Microsoft account's fetch brings. The password
/// goes beside the request and is not in it.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EwsFetchRequest {
    pub account: Id,
    /// The account's address: what the answer is checked against.
    pub email: String,
    /// The address of the Exchange Web Services endpoint. Never logged.
    pub url: String,
    /// The name the password is sent with; empty means `email`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub username: String,
    /// The fingerprint of the one certificate the owner trusts for this account; empty when none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub trusted: String,
    /// The server ids of the messages to fetch, in the order they are wanted.
    pub ids: Vec<String>,
    /// Nobody is waiting: it gets the share of requests background work has.
    #[serde(skip_serializing_if = "is_false")]
    pub background: bool,
}

/// What a run for an Exchange server account brought back, or a piece of it.
///
/// A run arrives in pieces, in the order things were done (`partial` on all but the last), each
/// whole in itself: taking the pieces in one after the other, as far as they come, leaves the
/// mailbox as far along as the run got.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct EwsMailResult {
    pub account: Id,
    /// The address of the account the run was for: an answer is not taken in by another account
    /// that happens to have the id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    /// The address of the endpoint the run read (its request's): an answer is not taken in once
    /// the account names another server. Never logged.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub url: String,
    /// What became of the changes of folders sent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FolderChangeDone>,
    /// What became of the changes of messages sent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<MessageChangeDone>,
    /// What became of the drafts taken up.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub drafts: Vec<DraftDone>,
    /// What became of the messages sent.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub sent: Vec<SendDone>,
    /// What became of the messages put into its folders from another mailbox of the same owner's.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub placed: Vec<PlaceDone>,
    /// Every folder of the server, when it could be listed at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listing: Option<FolderListing>,
    /// Where the folder hierarchy goes on from now.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub hierarchy: String,
    /// The pages of messages read, in the order they came: each is taken in by itself.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<MessagePage>,
    /// Where each folder read goes on from now.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub states: Vec<EwsFolderState>,
    /// Problems worth telling the owner; empty when all went well.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// The server said it was busy and to stay away this many seconds (`ErrorServerBusy`): all
    /// work for the account waits that long.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_for: Option<u32>,
    /// More of this run comes after this piece.
    #[serde(skip_serializing_if = "is_false")]
    pub partial: bool,
    /// Nothing went wrong in the run and it was not stopped: every folder it meant to read was
    /// read to its end.
    #[serde(skip_serializing_if = "is_false")]
    pub whole: bool,
    /// The owner stopped the run (Cancel) after the piece in hand: the next one goes on from there.
    #[serde(skip_serializing_if = "is_false")]
    pub stopped: bool,
    /// How far the run has got, for the status bar.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<GraphProgress>,
    /// The server turned the password down (a 401): the owner is asked for it again, and the run
    /// is not repeated by itself.
    #[serde(skip_serializing_if = "is_false")]
    pub auth_refused: bool,
    /// The SHA-256 fingerprint (64 hexadecimal digits) of a certificate the system doesn't trust
    /// and the account doesn't either: the run stopped there, and the owner is asked whether to
    /// trust it for this account.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub untrusted: String,
}

impl EwsMailResult {
    /// Take in the next piece of the same run, for a caller that takes a run in at once: the pages
    /// and the rest in the order they came, and what the last piece says of the end.
    pub fn absorb(&mut self, piece: EwsMailResult) {
        self.account = piece.account;
        if !piece.email.is_empty() {
            self.email = piece.email;
        }
        if !piece.url.is_empty() {
            self.url = piece.url;
        }
        self.changes.extend(piece.changes);
        self.messages.extend(piece.messages);
        self.drafts.extend(piece.drafts);
        self.sent.extend(piece.sent);
        self.placed.extend(piece.placed);
        if piece.listing.is_some() {
            self.listing = piece.listing;
        }
        if !piece.hierarchy.is_empty() {
            self.hierarchy = piece.hierarchy;
        }
        self.pages.extend(piece.pages);
        self.states.extend(piece.states);
        self.errors.extend(piece.errors);
        self.busy_for = piece.busy_for.or(self.busy_for);
        (self.partial, self.whole, self.stopped) = (piece.partial, piece.whole, piece.stopped);
        if piece.progress.is_some() {
            self.progress = piece.progress;
        }
        self.auth_refused |= piece.auth_refused;
        if !piece.untrusted.is_empty() {
            self.untrusted = piece.untrusted;
        }
    }
}

/// A folder's sync state is the server's to say and never ours to show.
impl std::fmt::Debug for EwsFolderState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EwsFolderState").field("state", &!self.state.is_empty()).finish()
    }
}

/// Counts only: the server's address, the user name, the account's address, every id and sync
/// state, and every word of a message stay out of debug output.
impl std::fmt::Debug for EwsMailRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EwsMailRequest")
            .field("account", &self.account)
            .field("trusted", &!self.trusted.is_empty())
            .field("list_folders", &self.list_folders)
            .field("scope", &self.scope.describe())
            .field("states", &self.states.len())
            .field("changes", &self.changes.len())
            .field("messages", &self.messages.len())
            .field("drafts", &self.drafts.len())
            .field("sends", &self.sends.len())
            .field("places", &self.places.len())
            .field("inbox_pass", &self.inbox_pass)
            .field("full", &self.full)
            .field("background", &self.background)
            .finish()
    }
}

/// Counts only, as a run's request: no address, and no id of a message.
impl std::fmt::Debug for EwsFetchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EwsFetchRequest")
            .field("account", &self.account)
            .field("trusted", &!self.trusted.is_empty())
            .field("ids", &self.ids.len())
            .field("background", &self.background)
            .finish()
    }
}

/// Counts only, as the request's.
impl std::fmt::Debug for EwsMailResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let messages: usize = self.pages.iter().map(|page| page.messages.len()).sum();
        f.debug_struct("EwsMailResult")
            .field("account", &self.account)
            .field("changes", &self.changes.len())
            .field("messages_changed", &self.messages.len())
            .field("drafts", &self.drafts.len())
            .field("sent", &self.sent.len())
            .field("placed", &self.placed.len())
            .field("listed", &self.listing.as_ref().map(|listing| listing.folders.len()))
            .field("pages", &self.pages.len())
            .field("messages", &messages)
            .field("states", &self.states.len())
            .field("errors", &self.errors.len())
            .field("busy_for", &self.busy_for)
            .field("partial", &self.partial)
            .field("whole", &self.whole)
            .field("stopped", &self.stopped)
            .field("progress", &self.progress.is_some())
            .field("auth_refused", &self.auth_refused)
            .field("untrusted", &!self.untrusted.is_empty())
            .finish()
    }
}

#[cfg(test)]
#[path = "ews_tests.rs"]
mod tests;

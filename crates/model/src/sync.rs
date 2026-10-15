//! What a mail-server synchronisation is asked to do and what it brings back.
//!
//! Plain data, so the pieces stay apart: the engine builds a [`SyncRequest`] and applies a
//! [`SyncResult`] (both pure), and a host with sockets (`mailcraft-net`, on the desktop and in the
//! CLI) does the talking in between, usually on a background thread.

use serde::{Deserialize, Serialize};

use crate::{Account, Address, FlagStatus, FolderKind, Id, Importance, Message};
use mailcraft_time::Timestamp;

/// A folder as the mailbox knows it, for the server side to continue from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FolderState {
    pub id: Id,
    /// Its path on the server; empty for a folder that only exists locally.
    pub remote: String,
    pub kind: FolderKind,
    pub uid_validity: u32,
    /// The highest UID already downloaded.
    pub uid_seen: u32,
    /// UIDs of the messages the mailbox has in this folder (to notice deletions and flag changes).
    pub known_uids: Vec<u32>,
}

/// A change made here for the server to take on. Server mailboxes are named by their path
/// ([`FolderState::remote`]), messages by their UID there.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RemoteOp {
    /// Set or clear flags of a message. Only the flags that were changed here are named; `None`
    /// leaves that flag as the server has it (another program may have changed it meanwhile).
    Flags {
        folder: String,
        uid: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        read: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        flagged: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        answered: Option<bool>,
    },
    /// The message that is `uid` in `from` belongs in local folder `to_folder` now. With `create`
    /// empty, `to` is that folder's server path. A folder that only exists here is made on the
    /// server first: `create` holds its names below `to`, the server path of the nearest folder
    /// above it that the server has (empty: the top level).
    #[serde(rename_all = "camelCase")]
    Move {
        message: Id,
        from: String,
        uid: u32,
        to: String,
        to_folder: Id,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        create: Vec<String>,
    },
    /// The message that is `uid` in `from` was deleted for good.
    Delete { from: String, uid: u32 },
}

impl RemoteOp {
    /// The server mailbox the change is made in.
    pub fn mailbox(&self) -> &str {
        match self {
            RemoteOp::Flags { folder, .. } => folder,
            RemoteOp::Move { from, .. } | RemoteOp::Delete { from, .. } => from,
        }
    }
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncRequest {
    pub account: Account,
    /// Never written to disk or logs.
    #[serde(skip)]
    pub password: String,
    /// The OAuth access token of an account that signs in that way ([`Account::oauth`]), in place
    /// of a password. Never written to disk or logs.
    #[serde(skip)]
    pub access_token: String,
    pub folders: Vec<FolderState>,
    /// Messages waiting in the Outbox, to send first.
    pub outbox: Vec<Message>,
    /// Most messages to download per folder in one run (the newest win); 0 means no limit.
    pub max_per_folder: u32,
    /// Only send what waits and tell the server what changed here; bring nothing down. For a
    /// change that should reach the server at once, without reading every folder for it.
    #[serde(skip_serializing_if = "is_false")]
    pub push_only: bool,
    /// Changes made here since the server last heard, to push before anything is downloaded.
    /// A change is only ever made in a mailbox whose `UIDVALIDITY` is still the one `folders`
    /// has for it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ops: Vec<RemoteOp>,
}

impl SyncRequest {
    /// What the servers are shown to prove who is asking: the access token of an OAuth account,
    /// the password of any other.
    pub fn secret(&self) -> &str {
        if self.account.oauth.is_some() { &self.access_token } else { &self.password }
    }
}

/// The password and the token stay out of debug output.
impl std::fmt::Debug for SyncRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hidden = |secret: &str| if secret.is_empty() { "" } else { "…" };
        f.debug_struct("SyncRequest")
            .field("account", &self.account.email)
            .field("password", &hidden(&self.password))
            .field("access_token", &hidden(&self.access_token))
            .field("folders", &self.folders.len())
            .field("outbox", &self.outbox.len())
            .field("ops", &self.ops.len())
            .finish()
    }
}

/// What an OAuth sign-in brought back. Kept in memory only: it has no way to be written out, and
/// it doesn't show in debug output.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Tokens {
    /// Shown to the mail servers; good for about an hour.
    pub access: String,
    /// Trades for a new access token without asking the owner again; may be empty.
    pub refresh: String,
    /// When the access token stops working (Unix seconds).
    pub expires_at: i64,
    /// Who signed in, as the provider names them (may be empty): for the account's display name.
    pub name: String,
    /// The address the provider knows them by (may be empty).
    pub email: String,
    /// The directory they signed in to, as the provider names it (may be empty): Microsoft's
    /// identity token says it, and it tells an organisation's account from a personal one
    /// ([`crate::Directory::of_tenant`]). An identifier, not a secret; kept in memory like the rest.
    pub tenant: String,
}

impl Tokens {
    /// Whether the access token is still good at `now` (Unix seconds), with a minute to spare.
    pub fn fresh(&self, now: i64) -> bool {
        !self.access.is_empty() && self.expires_at.saturating_sub(60) > now
    }
}

impl std::fmt::Debug for Tokens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let hidden = |secret: &str| if secret.is_empty() { "" } else { "…" };
        f.debug_struct("Tokens")
            .field("access", &hidden(&self.access))
            .field("refresh", &hidden(&self.refresh))
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A message's flags on the server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemoteFlags {
    pub uid: u32,
    pub read: bool,
    pub flagged: bool,
    pub answered: bool,
    pub draft: bool,
}

impl RemoteFlags {
    /// What `message` shows now, as a server would hold it under `uid`. A follow-up flag that
    /// is complete is no flag there.
    pub fn of(message: &Message, uid: u32) -> RemoteFlags {
        RemoteFlags { uid, read: message.read, flagged: message.flag == FlagStatus::Flagged, answered: message.answered, draft: message.draft }
    }

    /// The same flags under another UID (the message moved).
    pub fn at(self, uid: u32) -> RemoteFlags {
        RemoteFlags { uid, ..self }
    }
}

/// A message as a Microsoft account's server knows it (ADR 0003): its server id (Graph's
/// immutable id, which stays the same when the message moves within the mailbox) and what the
/// server last said of the properties a change made here is worked out from. The difference
/// between these and the message as it is now is what a run sends (R3.3).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ServerMessage {
    pub id: String,
    /// The server id of the folder it is in.
    pub folder: String,
    pub read: bool,
    pub flag: FlagStatus,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub importance: Importance,
    /// The server said it left `folder` (deleted, or moved out: Graph's `@removed`). It is kept
    /// until every folder's delta has ended in the run, in case it turns up in another folder;
    /// still gone from every folder then, it is removed here (R3 spec S9).
    #[serde(skip_serializing_if = "is_false")]
    pub gone: bool,
}

/// A value from a mailbox file, or `None` when what the file holds in its place isn't one (a
/// file from elsewhere, or from a later version): the mailbox still opens.
pub(crate) fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(match Maybe::<T>::deserialize(deserializer)? {
        Maybe::Is(value) => Some(value),
        Maybe::Not(_) => None,
    })
}

/// [`lenient`] for a value that has a "nothing was said" of its own: that, when what the file
/// holds in its place isn't one.
pub(crate) fn lenient_or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    lenient(deserializer).map(Option::unwrap_or_default)
}

/// A list from a mailbox file without the entries that aren't what they should be.
pub(crate) fn lenient_list<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum List<T> {
        Is(Vec<Maybe<T>>),
        Not(serde::de::IgnoredAny),
    }
    Ok(match List::<T>::deserialize(deserializer)? {
        List::Is(entries) => entries
            .into_iter()
            .filter_map(|entry| match entry {
                Maybe::Is(value) => Some(value),
                Maybe::Not(_) => None,
            })
            .collect(),
        List::Not(_) => Vec::new(),
    })
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Maybe<T> {
    Is(T),
    Not(serde::de::IgnoredAny),
}

/// What changed in one server folder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FolderDelta {
    /// The local folder, or `None` for a server folder the mailbox doesn't have yet.
    pub id: Option<Id>,
    pub remote: String,
    /// What the server says the folder is for (`\Sent`, `\Trash`…), [`FolderKind::Custom`] otherwise.
    pub kind: FolderKind,
    pub uid_validity: u32,
    pub uid_seen: u32,
    /// Newly downloaded messages (`uid` set; `folder` and `account` are filled in when applied).
    pub new: Vec<Message>,
    /// Current flags of messages the mailbox already had.
    pub flags: Vec<RemoteFlags>,
    /// UIDs the mailbox has that the server no longer does.
    pub removed: Vec<u32>,
}

/// A folder of mail the server has, as its list names it ([`SyncResult::listed`]).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ListedFolder {
    /// Its path on the server.
    pub remote: String,
    pub kind: FolderKind,
    /// The path of the folder it lies in; empty for one at the top.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent: String,
    /// The last part of its name, as people read it.
    pub name: String,
}

/// A flag change the server took on ([`RemoteOp::Flags`]): the flags that were set or cleared.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlagsPushed {
    pub folder: String,
    pub uid: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flagged: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered: Option<bool>,
}

/// A move that is finished ([`RemoteOp::Move`]): the server carried it out, or the message was
/// no longer where it was to be moved from, which leaves nothing to do.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Moved {
    pub message: Id,
    pub from: String,
    pub uid: u32,
    pub to_folder: Id,
    /// The message's UID in its new mailbox, when the server said (`COPYUID`); 0 otherwise.
    #[serde(skip_serializing_if = "is_zero")]
    pub new_uid: u32,
    /// `UIDVALIDITY` of the mailbox `new_uid` belongs to; 0 when the server didn't say.
    #[serde(skip_serializing_if = "is_zero")]
    pub uid_validity: u32,
}

/// A deletion that is finished ([`RemoteOp::Delete`]): the message is gone from the server, or
/// marked as deleted there, or was gone already.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Deleted {
    pub from: String,
    pub uid: u32,
}

/// A local folder that has a mailbox on the server now ([`RemoteOp::Move`] with `create`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Created {
    pub folder: Id,
    /// The new mailbox's path on the server.
    pub remote: String,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// What a synchronisation brought back, or a piece of it.
///
/// A host that wants to show mail as it arrives is handed a run's answer in pieces, in the order
/// things were done (`partial` on all but the last). Every piece is whole in itself: taking the
/// pieces in one after the other, as far as they come, leaves the mailbox as far along as the
/// run got, and what a later piece would have brought is asked for again by the next run.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SyncResult {
    pub account: Id,
    /// The address of the account the run was for: an answer is not taken in by another account
    /// that happens to have the id (in a mailbox opened while the run was under way).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    pub folders: Vec<FolderDelta>,
    /// Every folder of mail the server has, parents and children, said once near the start of a
    /// run: the tree is there to see before its mail is.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub listed: Vec<ListedFolder>,
    /// Ids of Outbox messages that were sent.
    pub sent: Vec<Id>,
    /// Those of `sent` that have a copy in the server's mailbox for sent mail, or will have: put
    /// there in this run, or filed by a server that keeps what it sends. The copy comes down with
    /// that mailbox, now or later.
    ///
    /// In an answer that comes in pieces this is said with `sent`, before the copy is put there,
    /// so that a mailbox which only hears that much knows the copy for what it is when it shows.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filed: Vec<Id>,
    /// Messages an earlier piece said were `filed` whose copy didn't get to the server after all
    /// (it wouldn't take it, or has no mailbox for sent mail): the copy here is the one there is.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub unfiled: Vec<Id>,
    /// What became of [`SyncRequest::ops`]. An op that isn't here failed (see `errors`) or wasn't
    /// tried, and is still to be pushed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub flagged: Vec<FlagsPushed>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub moved: Vec<Moved>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deleted: Vec<Deleted>,
    /// Mailboxes made on the server, whether or not the move they were made for went through.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub created: Vec<Created>,
    /// Problems worth telling the user (one per failed step); empty when all went well. In a
    /// piece of an answer: the problems since the piece before.
    pub errors: Vec<String>,
    /// This is a piece of an answer, and more follows. The last piece doesn't have it.
    #[serde(skip_serializing_if = "is_false")]
    pub partial: bool,
    /// One short line on how far the run has got (`Inbox: 150 of 500`), for a status bar. Empty
    /// in the last piece, whose summary says how it went.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub progress: String,
    /// The run was called off before it had done all there was to do (in the last piece).
    #[serde(skip_serializing_if = "is_false")]
    pub stopped: bool,
}

impl SyncResult {
    pub fn new_count(&self) -> usize {
        self.folders.iter().map(|f| f.new.len()).sum()
    }

    /// How many changes made here the server took on.
    pub fn pushed_count(&self) -> usize {
        self.flagged.len() + self.moved.len() + self.deleted.len() + self.created.len()
    }

    /// Whether there is anything in it for a mailbox to take in. A piece with only a line of
    /// progress, or only problems to tell, has nothing.
    pub fn has_news(&self) -> bool {
        !(self.folders.is_empty()
            && self.listed.is_empty()
            && self.sent.is_empty()
            && self.filed.is_empty()
            && self.unfiled.is_empty()
            && self.pushed_count() == 0)
    }

    /// Add the piece that came after this one, so that the two are one answer: taking it in is
    /// the same as taking in the one and then the other. What a mailbox's pieces say about it
    /// comes together under its name, its new messages in the order they came.
    pub fn absorb(&mut self, later: SyncResult) {
        (self.account, self.email) = (later.account, later.email);
        (self.partial, self.progress, self.stopped) = (later.partial, later.progress, later.stopped);
        if !later.listed.is_empty() {
            self.listed = later.listed;
        }
        self.sent.extend(later.sent);
        // A copy that didn't get there after all is one that was never said to.
        self.filed.retain(|id| !later.unfiled.contains(id));
        self.filed.extend(later.filed);
        self.unfiled.extend(later.unfiled);
        self.flagged.extend(later.flagged);
        self.moved.extend(later.moved);
        self.deleted.extend(later.deleted);
        self.created.extend(later.created);
        self.errors.extend(later.errors);
        for delta in later.folders {
            match self.folders.iter_mut().find(|f| f.remote == delta.remote && f.uid_validity == delta.uid_validity) {
                Some(folder) => {
                    folder.id = folder.id.or(delta.id);
                    // The mailbox is read as far as the piece that got furthest says.
                    folder.uid_seen = folder.uid_seen.max(delta.uid_seen);
                    folder.new.extend(delta.new);
                    folder.flags.extend(delta.flags);
                    folder.removed.extend(delta.removed);
                }
                None => self.folders.push(delta),
            }
        }
    }

    /// `3 new messages, 1 sent`, `Up to date`, or the first error.
    ///
    /// Before the result is applied, "new" is everything that came down, including the server's
    /// copy of a message that is already here (one that was just sent, or moved on a server that
    /// doesn't say where it put it). What applying it answers is exact.
    pub fn summary(&self) -> String {
        summary_line(self.errors.first().map(String::as_str), self.new_count(), self.sent.len())
    }
}

/// `3 new messages, 1 sent`, `Up to date`, or the error.
pub fn summary_line(error: Option<&str>, new: usize, sent: usize) -> String {
    if let Some(e) = error {
        return e.to_string();
    }
    let plural = |n: usize| if n == 1 { "" } else { "s" };
    match (new, sent) {
        (0, 0) => "Up to date".into(),
        (n, 0) => format!("{n} new message{}", plural(n)),
        (0, s) => format!("{s} message{} sent", plural(s)),
        (n, s) => format!("{n} new message{}, {s} sent", plural(n)),
    }
}

// ---- Microsoft Graph (R3) ----
//
// A Microsoft account is kept in step through Graph alone (ADR 0002), and its folders and
// messages are known by their server ids (ADR 0003). These are its own request and answer,
// beside the IMAP ones above rather than bent into them: no UIDs, no paths.

/// Most changes to folders one Graph run is asked to make; more go over several runs.
pub const MAX_FOLDER_CHANGES: usize = 500;
/// Most folders a listing holds; a mailbox with more is listed in part (and nothing is removed
/// for being missing from it).
pub const MAX_LISTED_FOLDERS: usize = 10_000;

/// A change made here to a folder of a Microsoft account, for its server to take on: worked out
/// from the difference between the folder and what the server last said of it
/// ([`crate::ServerFolder`]), or waiting in [`crate::Mailbox::queued`]. Folders are named by
/// their server ids; a parent that is empty is the top of the mailbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FolderChange {
    /// The folder `folder` only exists here: the server makes it, called `name`, under the
    /// folder `parent`; or, when `parent_folder` names a folder of this mailbox that is made in
    /// the same run, under that one once it is.
    #[serde(rename_all = "camelCase")]
    Create {
        folder: Id,
        name: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        parent: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_folder: Option<Id>,
    },
    /// The folder `folder`, which the server calls `id`, is called `name` now.
    Rename { folder: Id, id: String, name: String },
    /// The folder `folder`, which the server calls `id`, lies in `parent` now. (Delete Folder is
    /// one of these: into Deleted Items, as Outlook does.)
    Move {
        folder: Id,
        id: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        parent: String,
    },
    /// The folder the server calls `id` was deleted for good here.
    Delete { id: String },
}

impl FolderChange {
    /// The folder of this mailbox the change is about; `None` for one that is gone here.
    pub fn folder(&self) -> Option<Id> {
        match self {
            FolderChange::Create { folder, .. } | FolderChange::Rename { folder, .. } | FolderChange::Move { folder, .. } => Some(*folder),
            FolderChange::Delete { .. } => None,
        }
    }
}

/// What a Graph run for a Microsoft account is asked to do: the folder changes made here go up,
/// then every folder of the mailbox is listed. The access token is handed to the host's Graph
/// code beside this, never in it: nothing here is a secret, and it may be logged by count.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphMailRequest {
    pub account: Id,
    /// The account's address: the mailbox whose requests are counted together (Microsoft allows
    /// a few at once per mailbox), and what the answer is checked against.
    pub email: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FolderChange>,
    /// Nobody is waiting for this run: it gets the share of requests background work has.
    #[serde(skip_serializing_if = "is_false")]
    pub background: bool,
    /// Where each folder's mail goes on from (the folder's server id and its link, and whether
    /// the link is a delta link); a folder that isn't here is read from its start (R3 spec S10).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<GraphLink>,
}

/// Where a folder's message delta goes on from.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphLink {
    pub folder: String,
    pub link: String,
    /// It is a delta link (the folder's round has ended once), not the next page of a round.
    #[serde(skip_serializing_if = "is_false")]
    pub caught_up: bool,
}

/// A message as a folder's delta gives it: what the message list needs (R3 spec S9).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphMessage {
    /// The server's state of it: its immutable id, the folder, read, flag, categories, importance.
    pub server: ServerMessage,
    /// `Message-ID` without angle brackets.
    pub message_id: String,
    pub subject: String,
    pub from: Address,
    pub to: Vec<Address>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<Address>,
    /// When it was sent, and when it arrived.
    pub sent: Timestamp,
    pub received: Timestamp,
    pub preview: String,
    #[serde(skip_serializing_if = "is_false")]
    pub has_attachments: bool,
}

/// One page of a folder's message delta, taken in whole with the link that goes on from it, so
/// that a mailbox file never holds the one without the other (R3 spec S10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MessagePage {
    /// The folder's server id.
    pub folder: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<GraphMessage>,
    /// The server ids of the messages that left the folder (`@removed`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<String>,
    /// Where the folder goes on from after this page.
    pub link: String,
    /// `link` is a delta link: this page ended the round.
    #[serde(skip_serializing_if = "is_false")]
    pub caught_up: bool,
    /// The server no longer took the folder's link (`410 Gone`, `syncStateNotFound`): this page
    /// begins a fresh listing, and what the complete listing doesn't have is removed when it ends
    /// (R3 spec S11).
    #[serde(skip_serializing_if = "is_false")]
    pub afresh: bool,
}

/// One folder of a Microsoft account's mailbox, as the server lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphFolder {
    /// The server's id.
    pub id: String,
    /// The server id of the folder it lies in; empty for one at the top of the mailbox.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub parent: String,
    /// Its name, whole: it may hold `/` or `\`, and is never split into a path.
    pub name: String,
    /// Which special folder it is, when the server says it is one of them.
    pub kind: FolderKind,
    /// How many messages it has, and how many of them are unread, as the server counts them:
    /// for progress (R3 spec S15).
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub total: u32,
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub unread: u32,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// Every folder the server has, parents before what lies in them. Hidden folders and search
/// folders are not among them.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct FolderListing {
    pub folders: Vec<GraphFolder>,
    /// The listing went to its end: a folder that isn't in it is one the server doesn't have.
    /// A listing that was cut short (a page that didn't come, a limit reached) only adds and
    /// updates; nothing is removed for being missing from it (R3 spec S8).
    pub complete: bool,
}

/// What the server made of a [`FolderChange`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeOutcome {
    /// Done: the folder as the server has it now (for a deletion, empty).
    Done { folder: GraphFolder },
    /// The server no longer has the folder: there is nothing to do (the listing removes it).
    Gone,
    /// The server won't (403, 400, a name that is taken there): the folder is put back to what
    /// the server has, and this says why, in plain words.
    Refused { why: String },
}

/// A change and what became of it. A change that isn't answered (the server was busy, the
/// connection went) is not here: it is worked out again for the next run.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderChangeDone {
    pub change: FolderChange,
    pub outcome: ChangeOutcome,
}

/// What a Graph run for a Microsoft account brought back.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GraphMailResult {
    pub account: Id,
    /// The address of the account the run was for: an answer is not taken in by another account
    /// that happens to have the id.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub email: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FolderChangeDone>,
    /// Every folder of the mailbox, when it could be listed at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub listing: Option<FolderListing>,
    /// Problems worth telling the owner; empty when all went well.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// The server said it was busy and to stay away this many seconds: all Graph work for the
    /// account waits that long (R3 spec S14).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy_for: Option<u32>,
    /// Pages of message delta, in the order they came: each is taken in by itself (R3 spec S10).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pages: Vec<MessagePage>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R3.2: what a folder's delta brings is kept with the message and the folder, and a file
    /// from before reads as whole messages, no link, and the default download limit.
    #[test]
    fn delta_state_reads_from_older_files_and_round_trips() {
        let old: Message = serde_json::from_str(r#"{"id": 4, "subject": "Hi", "server": {"id": "AAkA", "folder": "F1"}}"#).unwrap();
        assert!(!old.unfetched && !old.server.as_ref().unwrap().gone);
        let account: Account = serde_json::from_str(r#"{"email": "ada@example.org", "downloadInFull": "everything"}"#).unwrap();
        assert_eq!((account.download_in_full, account.download_in_full.limit()), (crate::DownloadInFull::Everything, None));
        let account: Account = serde_json::from_str(r#"{"email": "ada@example.org", "downloadInFull": "upTo999Mb"}"#).unwrap();
        assert_eq!(account.download_in_full.limit(), Some(10 << 20), "unknown: the default");
        assert!(!serde_json::to_string(&account).unwrap().contains("downloadInFull"));
        let folder: crate::ServerFolder = serde_json::from_str(r#"{"id": "F1", "name": "Inbox", "delta": "https://x/d"}"#).unwrap();
        assert!(!folder.caught_up);

        let mut m = old.clone();
        m.unfetched = true;
        if let Some(server) = m.server.as_mut() {
            server.gone = true;
        }
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains(r#""gone":true"#) && json.contains(r#""unfetched":true"#), "{json}");
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), m);
        let page =
            MessagePage { folder: "F1".into(), removed: vec!["AAkB".into()], link: "https://x/d".into(), caught_up: true, ..Default::default() };
        let result = GraphMailResult { pages: vec![page], ..Default::default() };
        assert_eq!(serde_json::from_str::<GraphMailResult>(&serde_json::to_string(&result).unwrap()).unwrap(), result);
    }

    #[test]
    fn the_password_never_shows() {
        let req = SyncRequest {
            password: "hunter2".into(),
            account: Account { email: "ada@example.org".into(), ..Default::default() },
            ..Default::default()
        };
        assert!(!format!("{req:?}").contains("hunter2"));
        assert!(!serde_json::to_string(&req).unwrap().contains("hunter2"));
        assert_eq!(req.secret(), "hunter2");
    }

    #[test]
    fn a_token_never_shows_and_stands_in_for_the_password() {
        let account = Account { email: "ada@example.org".into(), oauth: Some(crate::OAuth::default()), ..Default::default() };
        let req = SyncRequest { account, password: "hunter2".into(), access_token: "tok-en-123".into(), ..Default::default() };
        assert_eq!(req.secret(), "tok-en-123");
        assert!(!format!("{req:?}").contains("tok-en-123"));
        assert!(!serde_json::to_string(&req).unwrap().contains("tok-en-123"));
        let tokens = Tokens { access: "tok-en-123".into(), refresh: "re-fresh-456".into(), expires_at: 1_000, ..Default::default() };
        let shown = format!("{tokens:?}");
        assert!(!shown.contains("tok-en-123") && !shown.contains("re-fresh-456"));
        assert!(tokens.fresh(900) && !tokens.fresh(950) && !Tokens::default().fresh(0));
    }

    #[test]
    fn an_account_without_oauth_reads_and_writes_as_before() {
        let plain: Account = serde_json::from_str(r#"{"email":"ada@example.org"}"#).unwrap();
        assert_eq!(plain.oauth, None);
        assert!(!serde_json::to_string(&plain).unwrap().contains("oauth"));
        let signed: Account = serde_json::from_str(r#"{"email":"ada@example.org","oauth":{"provider":"microsoft","tenant":"consumers"}}"#).unwrap();
        assert_eq!(signed.oauth.as_ref().map(|o| o.tenant.as_str()), Some("consumers"));
        assert_eq!(signed.login(), "ada@example.org");
    }

    #[test]
    fn changes_for_the_server_read_and_write_as_json() {
        let ops = vec![
            RemoteOp::Flags { folder: "INBOX".into(), uid: 7, read: Some(true), flagged: None, answered: Some(false) },
            RemoteOp::Move { message: 12, from: "INBOX".into(), uid: 7, to: "Archive".into(), to_folder: 4, create: vec![] },
            RemoteOp::Move { message: 13, from: "INBOX".into(), uid: 8, to: String::new(), to_folder: 5, create: vec!["Receipts".into()] },
            RemoteOp::Delete { from: "Trash".into(), uid: 2 },
        ];
        assert_eq!(ops.iter().map(RemoteOp::mailbox).collect::<Vec<_>>(), ["INBOX", "INBOX", "INBOX", "Trash"]);
        let req = SyncRequest { password: "hunter2".into(), ops, ..Default::default() };
        let json = serde_json::to_string(&req).unwrap();
        // A flag that wasn't changed here isn't named at all.
        assert!(json.contains(r#"{"flags":{"folder":"INBOX","uid":7,"read":true,"answered":false}}"#), "{json}");
        assert!(json.contains(r#"{"move":{"message":12,"from":"INBOX","uid":7,"to":"Archive","toFolder":4}}"#), "{json}");
        assert!(json.contains(r#""toFolder":5,"create":["Receipts"]"#) && json.contains(r#"{"delete":{"from":"Trash","uid":2}}"#), "{json}");
        let back: SyncRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ops, req.ops);
        assert!(format!("{req:?}").contains("ops: 4") && !format!("{req:?}").contains("hunter2"));
        assert!(!serde_json::to_string(&SyncRequest::default()).unwrap().contains("ops"));

        let result = SyncResult {
            flagged: vec![FlagsPushed { folder: "INBOX".into(), uid: 7, read: Some(true), ..Default::default() }],
            moved: vec![Moved { message: 12, from: "INBOX".into(), uid: 7, to_folder: 4, new_uid: 31, uid_validity: 9 }, Moved::default()],
            deleted: vec![Deleted { from: "Trash".into(), uid: 2 }],
            created: vec![Created { folder: 5, remote: "Receipts".into() }],
            filed: vec![3],
            ..Default::default()
        };
        assert_eq!(result.pushed_count(), 5);
        let json = serde_json::to_string(&result).unwrap();
        assert!(json.contains(r#""toFolder":4,"newUid":31,"uidValidity":9"#), "{json}");
        assert_eq!(serde_json::from_str::<SyncResult>(&json).unwrap(), result);
        // A result from before any of this was there reads as one that pushed nothing.
        let old: SyncResult = serde_json::from_str(r#"{"account": 3, "sent": [1], "errors": []}"#).unwrap();
        assert_eq!((old.pushed_count(), old.filed.len()), (0, 0));
        let plain = serde_json::to_string(&old).unwrap();
        assert!(!plain.contains("moved") && !plain.contains("filed"), "{plain}");
    }

    #[test]
    fn the_pieces_of_an_answer_come_together() {
        let m = |uid: u32| Message { uid, ..Default::default() };
        let delta =
            |remote: &str, uid_seen, new: Vec<Message>| FolderDelta { remote: remote.into(), uid_validity: 7, uid_seen, new, ..Default::default() };
        let pieces = [
            SyncResult { account: 3, partial: true, progress: "Sent 1 of 1".into(), sent: vec![9], filed: vec![9], ..Default::default() },
            // A line of progress and a problem to tell: nothing for a mailbox to take in.
            SyncResult { account: 3, partial: true, progress: "Connecting".into(), errors: vec!["One".into()], ..Default::default() },
            SyncResult {
                account: 3,
                partial: true,
                progress: "Inbox: 2 of 4".into(),
                unfiled: vec![9],
                flagged: vec![FlagsPushed::default()],
                folders: vec![FolderDelta {
                    flags: vec![RemoteFlags { uid: 1, ..Default::default() }],
                    removed: vec![2],
                    ..delta("INBOX", 4, vec![m(7), m(8)])
                }],
                ..Default::default()
            },
            SyncResult {
                account: 3,
                email: "ada@example.org".into(),
                stopped: true,
                errors: vec!["Two".into()],
                folders: vec![FolderDelta { id: Some(5), ..delta("INBOX", 8, vec![m(5), m(6)]) }, delta("Sent", 1, vec![m(1)])],
                ..Default::default()
            },
        ];
        assert_eq!(pieces.iter().map(SyncResult::has_news).collect::<Vec<_>>(), [true, false, true, true]);
        let mut whole = SyncResult::default();
        assert!(!whole.has_news());
        pieces.into_iter().for_each(|piece| whole.absorb(piece));
        // It is the last piece that says whose answer it is and how the run ended.
        assert_eq!(
            (whole.account, whole.email.as_str(), whole.partial, whole.progress.as_str(), whole.stopped),
            (3, "ada@example.org", false, "", true)
        );
        assert_eq!((whole.sent.as_slice(), whole.flagged.len(), whole.errors.join(", ").as_str()), (&[9][..], 1, "One, Two"));
        // A copy that didn't get to the server after all was never there.
        assert_eq!((whole.filed.len(), whole.unfiled.as_slice()), (0, &[9][..]));
        // What was said of a mailbox comes together: read as far as the piece that got furthest,
        // its new messages in the order they came.
        let inbox = &whole.folders[0];
        assert_eq!((inbox.id, inbox.uid_seen, inbox.flags.len(), inbox.removed.as_slice()), (Some(5), 8, 1, &[2][..]));
        assert_eq!(inbox.new.iter().map(|m| m.uid).collect::<Vec<_>>(), [7, 8, 5, 6]);
        assert_eq!((whole.folders.len(), whole.new_count()), (2, 5));
        // A mailbox that was made anew meanwhile is another mailbox.
        whole.absorb(SyncResult { folders: vec![FolderDelta { uid_validity: 8, ..delta("INBOX", 1, vec![m(1)]) }], ..Default::default() });
        assert_eq!((whole.folders.len(), whole.folders[0].uid_seen, whole.folders[2].uid_seen), (3, 8, 1));

        // What a piece says of itself is written only when there is something to say, and an
        // answer from before there were pieces reads as a whole one.
        let piece = SyncResult { partial: true, progress: "Inbox: 1 of 2".into(), stopped: true, unfiled: vec![4], ..Default::default() };
        let json = serde_json::to_string(&piece).unwrap();
        assert!(json.contains(r#""partial":true,"progress":"Inbox: 1 of 2","stopped":true"#) && json.contains(r#""unfiled":[4]"#), "{json}");
        assert_eq!(serde_json::from_str::<SyncResult>(&json).unwrap(), piece);
        let plain = serde_json::to_string(&SyncResult::default()).unwrap();
        assert!(!["partial", "progress", "stopped", "unfiled"].iter().any(|word| plain.contains(word)), "{plain}");
        let old: SyncResult = serde_json::from_str(r#"{"account": 3, "sent": [1], "errors": []}"#).unwrap();
        assert!(!old.partial && !old.stopped && old.progress.is_empty() && old.unfiled.is_empty() && old.has_news());
    }

    #[test]
    fn the_flags_a_server_would_hold() {
        let m = Message { read: true, flag: FlagStatus::Complete, answered: true, ..Default::default() };
        // A follow-up that is done with is no flag on a server.
        assert_eq!(RemoteFlags::of(&m, 7), RemoteFlags { uid: 7, read: true, flagged: false, answered: true, draft: false });
        let flagged = Message { flag: FlagStatus::Flagged, draft: true, ..Default::default() };
        assert_eq!(RemoteFlags::of(&flagged, 0).at(9), RemoteFlags { uid: 9, read: false, flagged: true, answered: false, draft: true });
    }

    #[test]
    fn summaries() {
        assert_eq!(summary_line(None, 1, 2), "1 new message, 2 sent");
        assert_eq!(summary_line(Some("Couldn't sign in"), 1, 2), "Couldn't sign in");
        let mut r = SyncResult::default();
        assert_eq!(r.summary(), "Up to date");
        r.folders.push(FolderDelta { new: vec![Message::default(), Message::default()], ..Default::default() });
        assert_eq!(r.summary(), "2 new messages");
        r.sent.push(1);
        assert_eq!(r.summary(), "2 new messages, 1 sent");
        r.errors.push("Couldn't sign in".into());
        assert_eq!(r.summary(), "Couldn't sign in");
    }
}

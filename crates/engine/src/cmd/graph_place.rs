//! Mail moved or copied between two mailboxes of one owner's (#70): a shared mailbox and its
//! owner's, or two shared mailboxes of his.
//!
//! Microsoft moves and copies a message only within its own mailbox. What goes to another is
//! made there by that mailbox's own run, at its own address, from the whole message here
//! ([`PlaceUp`]): the message waits here, in its new folder, until it is
//! ([`Queued::PlaceMessage`]). Of a move, the original stays on its own server, **being moved**
//! ([`mailcraft_model::sync::MovingOut`]), and goes from there only once the copy is confirmed on
//! the destination's server (`graph_up`'s moved-out changes): a copy that fails never loses
//! the message.
//!
//! - **Made**: the message here takes the copy's server id, and is that server message from then on.
//! - **Refused** (403, 400, or a folder that is gone): a move goes back to where its original
//!   still is, as that server message; a copy is taken away here again (its source is where it
//!   was). Said once per run and kind, as other refusals are (R3 spec S20). A move whose original
//!   was gone already is kept in a folder on this computer: it is all there is of it.
//! - **No answer**, or a shared mailbox's 403 that isn't "no access": it waits for the next run,
//!   where the copy is looked for by its marker before it is made again.
//! - Moved on, deleted or undone meanwhile: a copy made all the same is deleted there again.

use mailcraft_model::sync::{FlagUp, MessageOutcome, PlaceDone, PlaceUp, ServerMessage};
use mailcraft_model::{FlagStatus, FolderKind, Id, Mailbox, Message, Queued};
use mailcraft_time::{DateTime, Timestamp};

use super::graph_up::{Kind, Refusals, Why, destination, folder_name};
use crate::Session;

/// The messages waiting to be made in `account`'s folders: at most `limit` spelled out (each
/// whole, its body and files read in when they are kept on disk), and how many there are.
pub fn place_changes(s: &Session, account: Id, limit: usize) -> (Vec<PlaceUp>, usize) {
    let mb = &s.mailbox;
    let today = s.zone().instant(DateTime::at_midnight(s.today()));
    let now = s.now();
    let outbox = mb.folder_of_kind(account, FolderKind::Outbox);
    let (mut out, mut count) = (Vec::new(), 0usize);
    for queued in &mb.queued {
        let Queued::PlaceMessage { account: a, message, marker, .. } = queued else { continue };
        if *a != account {
            continue;
        }
        let Some(m) = mb.message(*message).filter(|m| m.account == account && m.server.is_none() && Some(m.folder) != outbox) else { continue };
        let Some((to, to_folder)) = destination(mb, account, m.folder) else { continue };
        count += 1;
        if out.len() >= limit {
            continue;
        }
        match s.loaded_message(m.id) {
            Ok(m) => out.push(PlaceUp::of(&m, to, to_folder, marker.clone(), m.moving_out.is_some(), flag_of(&m, today, now))),
            Err(e) => log::warn!("message {} waits to go to another mailbox: {e}", m.id),
        }
    }
    (out, count)
}

/// The flag of `m` as it goes with the copy.
fn flag_of(m: &Message, today: Timestamp, now: Timestamp) -> Option<FlagUp> {
    match m.flag {
        FlagStatus::None => None,
        FlagStatus::Flagged => {
            let start = m.flag_due.map_or(today, |due| today.min(due));
            Some(FlagUp { status: FlagStatus::Flagged, start: Some(start), due: m.flag_due, completed: None })
        }
        FlagStatus::Complete => Some(FlagUp { status: FlagStatus::Complete, completed: Some(now), ..Default::default() }),
    }
}

/// Take in what became of one message made in `account` from another mailbox. Whether anything
/// changed here.
pub fn place_done(mb: &mut Mailbox, account: Id, done: &PlaceDone, refusals: &mut Refusals) -> bool {
    let change = &done.change;
    let id = change.message;
    let entry = mb.queued.iter().position(
        |q| matches!(q, Queued::PlaceMessage { account: a, message, marker, .. } if *a == account && *message == id && *marker == change.marker),
    );
    let made = match &done.outcome {
        MessageOutcome::Done { id: made, .. } => Some(made.clone()).filter(|made| !made.is_empty()),
        _ => None,
    };
    let Some(entry) = entry else {
        // Called off meanwhile (moved on, deleted, undone): a copy made all the same goes again.
        if let Some(made) = made {
            mb.queued.push(Queued::DeleteMessage { account, id: made, kept: None });
            return true;
        }
        return false;
    };
    if matches!(done.outcome, MessageOutcome::Done { .. }) && made.is_none() {
        // Made, but which one isn't said: found by its marker next time.
        return false;
    }
    let copy = matches!(mb.queued.get(entry), Some(Queued::PlaceMessage { copy: true, .. }));
    mb.queued.remove(entry);
    let to_name = folder_name(mb, account, &change.to, change.to_folder);
    match (&done.outcome, made) {
        (MessageOutcome::Done { folder, .. }, Some(made)) => {
            let folder = if folder.is_empty() { change.to.clone() } else { folder.clone() };
            // Come down meanwhile (its make's answer was lost, and the delta brought it): this is it.
            let twins: Vec<Id> = mb
                .messages
                .values()
                .filter(|o| o.account == account && o.id != id && o.server.as_ref().is_some_and(|s| s.id == made))
                .map(|o| o.id)
                .collect();
            for twin in twins {
                let _ = mb.forget_message(twin);
            }
            let Ok(m) = mb.message_mut(id) else { return true };
            let flag = change.flag.clone().unwrap_or_default();
            m.server = Some(ServerMessage {
                id: made,
                folder,
                read: change.read,
                flag: flag.status,
                flag_start: flag.start,
                flag_due: flag.due,
                flag_completed: flag.completed,
                categories: change.categories.clone(),
                importance: change.importance,
                ..Default::default()
            });
            true
        }
        (outcome, _) => {
            // (A copy isn't "gone": what the server no longer has is the folder it was to go to.)
            let why = if matches!(outcome, MessageOutcome::Refused { status: 403 }) { Why::Denied } else { Why::Invalid };
            let Some(m) = mb.message(id) else { return true };
            let subject = m.subject.clone();
            refusals.note(if copy { Kind::Copy } else { Kind::Move }, why, &subject, to_name);
            put_back(mb, id, copy);
            true
        }
    }
}

/// A message the destination wouldn't take: a move back to where its original still is, as that
/// server message; a copy taken away; a move whose original is gone kept on this computer.
fn put_back(mb: &mut Mailbox, id: Id, copy: bool) {
    let Some(m) = mb.message(id) else { return };
    let out = m.moving_out.clone().filter(|out| mb.is_graph(out.account) && !out.id.is_empty());
    match out {
        Some(out) => {
            let folder = mb.folder_by_server(out.account, &out.folder).or_else(|| mb.folder_of_kind(out.account, FolderKind::Inbox));
            let Some(folder) = folder else { return };
            let Ok(m) = mb.message_mut(id) else { return };
            let server = ServerMessage {
                id: out.id,
                folder: out.folder,
                read: m.read,
                flag: m.flag,
                flag_due: m.flag_due,
                categories: m.categories.clone(),
                importance: m.importance,
                ..Default::default()
            };
            (m.server, m.moving_out) = (Some(server), None);
            (m.folder, m.account, m.uid, m.synced) = (folder, out.account, 0, None);
        }
        None if copy => {
            let _ = mb.forget_message(id);
        }
        None => {
            if let Ok(folder) = mb.imported_folder(false)
                && let Some(account) = mb.folder(folder).map(|f| f.account)
                && let Ok(m) = mb.message_mut(id)
            {
                (m.folder, m.account, m.uid, m.synced) = (folder, account, 0, None);
            }
        }
    }
}

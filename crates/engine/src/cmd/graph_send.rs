//! Sending through Microsoft Graph (R3.4.2; R3 spec S27, S28).
//!
//! What waits in a Microsoft account's Outbox goes with its run: a message saved as a draft is
//! sent as that draft (brought up to date first), one never saved with `sendMail`. A message that
//! went leaves the Outbox; the server's copy in Sent Items comes down by delta, and nothing is
//! uploaded here. A **held** message ([`Message::held`]) is one whose answer was lost (it may have
//! gone) or that the server refused: it stays in the Outbox, says what happened and what to check,
//! and isn't sent again until the owner says so (Send Again).

use mailcraft_model::sync::{MAX_SENDS, SendDone, SendOutcome, SendUp};
use mailcraft_model::{FolderKind, Id, Mailbox};
use serde_json::{Value, json};

use super::{bad, fail};
use crate::{CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.sendAgain", "Send Again", "Mail", send_again)
            .params(r#"{"id"?: number} (default: the selected message) — a held message in the Outbox goes with the next Send/Receive"#)
            .selection(super::needs_message),
    ]
}

/// What `account` has waiting to be sent: every message of its Outbox that isn't held, whole.
/// What a message keeps on disk is read for it (R6.1); one whose content can't be read waits in
/// the Outbox for the next run, with a line in the log, rather than going without its files.
pub fn sends(s: &Session, account: Id) -> Vec<SendUp> {
    let mb = &s.mailbox;
    let Some(outbox) = mb.folder_of_kind(account, FolderKind::Outbox) else { return Vec::new() };
    let mut waiting: Vec<_> = mb.messages_in(outbox).filter(|m| m.held.is_empty()).map(|m| m.id).collect();
    waiting.sort_unstable();
    waiting
        .into_iter()
        .take(MAX_SENDS)
        .filter_map(|id| match s.loaded_message(id) {
            Ok(m) => Some(m),
            Err(e) => {
                log::warn!("message {id} stays in the Outbox this time: {e}");
                None
            }
        })
        .map(|m| match m.server_draft.as_ref().and(super::graph_drafts::draft_up(&m)) {
            // Sent as it is here: whatever was changed on the server meanwhile is the owner's to lose.
            Some(draft) => SendUp { message: m.id, draft: Some(mailcraft_model::sync::DraftUp { change_key: String::new(), ..draft }), mime: None },
            None => SendUp { message: m.id, draft: None, mime: Some(Box::new(m.as_ref().clone())) },
        })
        .collect()
}

/// How many messages of `account` wait in its Outbox to be sent (held ones aren't).
pub fn waiting(mb: &Mailbox, account: Id) -> usize {
    mb.folder_of_kind(account, FolderKind::Outbox).map_or(0, |outbox| mb.messages_in(outbox).filter(|m| m.held.is_empty()).count())
}

/// Take in what became of one message sent. Whether anything changed; a line for the owner when
/// it is held.
pub fn send_done(mb: &mut Mailbox, account: Id, done: &SendDone, said: &mut Vec<String>) -> bool {
    let mut changed = false;
    if let Some(draft) = &done.draft {
        changed |= super::graph_drafts::draft_done(mb, account, draft, said);
    }
    let Some(m) = mb.message(done.message).filter(|m| m.account == account) else { return changed };
    let subject: String = m.subject.chars().filter(|c| !c.is_control()).take(60).collect();
    let subject = if subject.trim().is_empty() { "(no subject)".to_string() } else { subject };
    match &done.outcome {
        SendOutcome::Waits => changed,
        // Gone: the server files it in Sent Items, and that copy comes down.
        SendOutcome::Sent => mb.forget_message(done.message).is_ok() || changed,
        SendOutcome::Held { lost, why } => {
            let held = if *lost {
                "It may have been sent: the answer from Microsoft was lost. Look in Sent Items before you send it again.".to_string()
            } else {
                format!("It wasn't sent: {why}. Change it, or send it again.")
            };
            said.push(if *lost {
                format!("'{subject}' may have been sent: look in Sent Items. It is held in the Outbox.")
            } else {
                format!("Couldn't send '{subject}': {why}. It is held in the Outbox.")
            });
            if let Ok(m) = mb.message_mut(done.message) {
                m.held = held;
            }
            true
        }
    }
}

fn send_again(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::id(v, "id").or(s.sel.focus()).ok_or_else(|| bad("select a message first"))?;
    let m = s.mailbox.message(id).ok_or_else(|| fail(format!("no message with id {id}")))?;
    if m.held.is_empty() {
        return Err(fail("that message isn't held"));
    }
    s.mailbox.message_mut(id)?.held.clear();
    s.mailbox_changed();
    Ok(json!({"id": id, "held": false}))
}

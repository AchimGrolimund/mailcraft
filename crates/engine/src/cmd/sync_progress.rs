//! Send/Receive Progress (#120): what the run under way is doing, or what the last one did,
//! account by account. Kept in the session, in memory only (never saved); `sync.apply` and
//! `sync.applyGraph` keep it up to date as the pieces of a run come in, and `sync.progress`
//! answers it.
//!
//! A run starts when a host says which accounts it covers ([`SyncProgress::begin`]), or else with
//! the first piece for an account when no run is under way. Problems may name a message by its
//! subject: that is message content.

use std::collections::HashMap;

use mailcraft_model::sync::{SyncPhase, SyncStage, SyncStep};
use mailcraft_model::{Id, Mailbox};
use serde_json::{Value, json};

use crate::{CmdResult, CommandSpec, Session};

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec::new("sync.progress", "Show Progress", "Send / Receive", progress).pure().params(
        r#"{} — answers {"running": bool, "accounts": [{"id", "name", "state": "waiting"|"running"|"done"|"stopped", "phase": "sending"|"folders"|"newMail"|"calendarAndContacts"|null, "folder", "done", "total", "new", "sent", "problems": [string], "kept": bool}]}; opens nothing"#,
    )]
}

/// Problems kept for an account, at most, and how long each may be.
const MAX_PROBLEMS: usize = 60;
const MAX_PROBLEM_CHARS: usize = 600;

/// Where an account is in a run.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RunState {
    /// Its turn hasn't come.
    #[default]
    Waiting,
    Running,
    Done,
    /// Its part of the run was called off before it was through.
    Stopped,
}

impl RunState {
    pub fn as_str(self) -> &'static str {
        match self {
            RunState::Waiting => "waiting",
            RunState::Running => "running",
            RunState::Done => "done",
            RunState::Stopped => "stopped",
        }
    }

    fn under_way(self) -> bool {
        matches!(self, RunState::Waiting | RunState::Running)
    }
}

/// One account's row: where it is in the run, how far it has got, and what it came to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AccountRun {
    pub account: Id,
    pub state: RunState,
    /// What it is doing (running), or did last; None before its first piece.
    pub phase: Option<SyncPhase>,
    /// The folder it is at, by its shown name; empty when it is at none.
    pub folder: String,
    /// How many of how many (messages sent, or a folder's new messages); 0 when not known.
    pub done: u32,
    pub total: u32,
    /// New messages and messages sent so far in the run.
    pub new: usize,
    pub sent: usize,
    /// What went wrong, in plain words, in the order it was said.
    pub problems: Vec<String>,
    /// The run changed nothing for it (no new mail, nothing sent, no problems): what it shows is
    /// what the run before came to.
    pub kept: bool,
}

/// What an account's part of a run came to, kept for the next run that changes nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Outcome {
    new: usize,
    sent: usize,
    problems: Vec<String>,
    stopped: bool,
}

/// The current run, or else the last one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncProgress {
    /// The accounts of the run, in the order they go.
    pub accounts: Vec<AccountRun>,
    /// What each account's last run came to (as last shown).
    last: HashMap<Id, Outcome>,
}

/// What a piece of a run says, whatever kind of account it is for.
#[derive(Default)]
pub(crate) struct Piece<'a> {
    /// The step it is at, when the piece says (else the row keeps the one before).
    pub step: Option<(SyncPhase, String, u32, u32)>,
    pub new: usize,
    pub sent: usize,
    pub problems: &'a [String],
    /// The last piece of the account's part of the run.
    pub last: bool,
    pub stopped: bool,
}

impl SyncProgress {
    /// Whether a run is under way: an account in it is running or waiting its turn.
    pub fn running(&self) -> bool {
        self.accounts.iter().any(|a| a.state.under_way())
    }

    /// A new run, of these accounts in this order, every one waiting: it takes the place of the
    /// one before. (For a host that knows which accounts a run covers; without it, a run is
    /// begun by its first piece.)
    pub fn begin(&mut self, accounts: &[Id]) {
        let mut rows: Vec<AccountRun> = Vec::new();
        for &account in accounts {
            if !rows.iter().any(|r| r.account == account) {
                rows.push(AccountRun { account, ..Default::default() });
            }
        }
        self.accounts = rows;
    }

    /// The account's row for a piece that came for it, running from now on. A piece for an
    /// account that isn't in the run under way joins it (one already done there carries on from
    /// what it had); when no run is under way, it begins one.
    fn row(&mut self, account: Id) -> Option<&mut AccountRun> {
        if !self.running() {
            self.begin(&[account]);
        } else if !self.accounts.iter().any(|r| r.account == account) {
            self.accounts.push(AccountRun { account, ..Default::default() });
        }
        let row = self.accounts.iter_mut().find(|r| r.account == account)?;
        if row.state != RunState::Running {
            row.state = RunState::Running;
            row.kept = false;
        }
        Some(row)
    }

    /// Take in a piece of the account's part of a run.
    pub(crate) fn take(&mut self, account: Id, piece: Piece) {
        let Some(row) = self.row(account) else { return };
        if let Some((phase, folder, done, total)) = piece.step {
            row.phase = Some(phase);
            row.folder = folder;
            (row.done, row.total) = (done, total);
        }
        row.new = row.new.saturating_add(piece.new);
        row.sent = row.sent.saturating_add(piece.sent);
        for problem in piece.problems {
            if row.problems.len() >= MAX_PROBLEMS {
                break;
            }
            let line: String = problem.chars().filter(|c| *c != '\r' && *c != '\0').take(MAX_PROBLEM_CHARS).collect();
            if !line.trim().is_empty() {
                row.problems.push(line);
            }
        }
        if !piece.last {
            return;
        }
        row.state = if piece.stopped { RunState::Stopped } else { RunState::Done };
        (row.folder, row.done, row.total) = (String::new(), 0, 0);
        let outcome = Outcome { new: row.new, sent: row.sent, problems: row.problems.clone(), stopped: piece.stopped };
        let nothing = outcome.new == 0 && outcome.sent == 0 && outcome.problems.is_empty() && !outcome.stopped;
        match self.last.get(&account) {
            // A run that changed nothing leaves the last result in place.
            Some(before) if nothing => {
                let before = before.clone();
                if let Some(row) = self.accounts.iter_mut().find(|r| r.account == account) {
                    (row.new, row.sent, row.problems) = (before.new, before.sent, before.problems);
                    row.state = if before.stopped { RunState::Stopped } else { RunState::Done };
                    row.kept = true;
                }
            }
            _ => {
                self.last.insert(account, outcome);
            }
        }
    }

    /// The state as JSON, for `sync.progress`: accounts the mailbox no longer has are left out.
    pub fn json(&self, mb: &Mailbox) -> Value {
        let accounts: Vec<Value> = self
            .accounts
            .iter()
            .filter_map(|r| {
                let name = mb.account(r.account)?.name.clone();
                Some(json!({
                    "id": r.account,
                    "name": name,
                    "state": r.state.as_str(),
                    "phase": r.phase.map(phase_id),
                    "folder": r.folder,
                    "done": r.done,
                    "total": r.total,
                    "new": r.new,
                    "sent": r.sent,
                    "problems": r.problems,
                    "kept": r.kept,
                }))
            })
            .collect();
        json!({"running": self.running(), "accounts": accounts})
    }
}

fn phase_id(phase: SyncPhase) -> &'static str {
    match phase {
        SyncPhase::Sending => "sending",
        SyncPhase::Folders => "folders",
        SyncPhase::NewMail => "newMail",
        SyncPhase::CalendarAndContacts => "calendarAndContacts",
    }
}

/// A mail server's step as a row shows it: a folder's new mail counted in messages, the folder
/// being looked at with no count yet, what was sent of how many.
pub(crate) fn imap_step(step: &SyncStep) -> (SyncPhase, String, u32, u32) {
    let folder = shown(&step.folder);
    match step.stage {
        SyncStage::Sending | SyncStage::Reading => (step.stage.phase(), folder, step.done, step.total),
        SyncStage::Checking | SyncStage::Updating => (step.stage.phase(), folder, 0, 0),
        SyncStage::Connecting | SyncStage::Listed | SyncStage::Filing => (step.stage.phase(), String::new(), 0, 0),
    }
}

/// A folder's name for a row: one line, not too long.
pub(crate) fn shown(name: &str) -> String {
    name.chars().filter(|c| !c.is_control()).take(80).collect()
}

fn progress(s: &mut Session, _: &Value) -> CmdResult {
    Ok(s.sync_progress.json(&s.mailbox))
}

#[cfg(test)]
mod tests {
    use mailcraft_model::sync::{FolderDelta, FolderListing, GraphFolder, GraphMailResult, GraphProgress, MessagePage, SyncResult};
    use mailcraft_model::{FolderKind, Message};
    use serde_json::json;

    use super::*;

    /// A session with an IMAP account and a Microsoft account kept in step through Graph.
    fn session() -> (Session, Id, Id) {
        let mut s = Session::empty();
        let servers = json!({"email": "avery@mail.example", "incoming": {"host": "imap.mail.example"}, "outgoing": {"host": "smtp.mail.example"}});
        let imap = s.run("account.add", &servers).unwrap()["id"].as_u64().unwrap();
        let graph = s.run("account.add", &json!({"email": "owner@example.org", "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
        (s, imap, graph)
    }

    fn progress(s: &mut Session) -> Value {
        s.run("sync.progress", &json!({})).unwrap()
    }

    fn row(s: &mut Session, account: Id) -> Value {
        let all = progress(s);
        all["accounts"].as_array().and_then(|rows| rows.iter().find(|r| r["id"] == json!(account)).cloned()).unwrap_or(Value::Null)
    }

    fn apply(s: &mut Session, piece: SyncResult) {
        s.run("sync.apply", &json!({"result": piece})).unwrap();
    }

    fn graph(s: &mut Session, piece: GraphMailResult) {
        s.run("sync.applyGraph", &json!({"result": piece})).unwrap();
    }

    fn step(stage: SyncStage, folder: &str, done: usize, total: usize) -> Option<SyncStep> {
        Some(SyncStep::new(stage, folder, done, total))
    }

    fn mail(uids: &[u32], uid_seen: u32) -> Vec<FolderDelta> {
        let new = uids
            .iter()
            .map(|uid| Message { uid: *uid, subject: format!("Mail {uid}"), message_id: format!("m{uid}@mail.example"), ..Default::default() })
            .collect();
        vec![FolderDelta { remote: "INBOX".into(), kind: FolderKind::Inbox, uid_validity: 7, uid_seen, new, ..Default::default() }]
    }

    #[test]
    fn a_run_is_followed_account_by_account() {
        let (mut s, imap, ms) = session();
        assert_eq!(progress(&mut s), json!({"running": false, "accounts": []}));
        // A host that knows the run's accounts says so: both wait their turn, in that order.
        s.sync_progress.begin(&[imap, ms, imap]);
        let all = progress(&mut s);
        assert_eq!(all["running"], json!(true));
        let states: Vec<(u64, &str)> =
            all["accounts"].as_array().unwrap().iter().map(|r| (r["id"].as_u64().unwrap(), r["state"].as_str().unwrap())).collect();
        assert_eq!(states, [(imap, "waiting"), (ms, "waiting")]);
        assert_eq!(all["accounts"][0]["name"], json!(s.mailbox.account(imap).unwrap().name));

        // The IMAP account's pieces: its phase, folder and counts as they go.
        let piece = |step: Option<SyncStep>| SyncResult { account: imap, partial: true, progress: String::new(), step, ..Default::default() };
        apply(&mut s, piece(step(SyncStage::Sending, "", 0, 2)));
        let r = row(&mut s, imap);
        assert_eq!(
            (r["state"].as_str(), r["phase"].as_str(), r["done"].as_u64(), r["total"].as_u64()),
            (Some("running"), Some("sending"), Some(0), Some(2))
        );
        // (A host that says only the numbers has its line made of them.)
        assert_eq!(s.sync_status, "Sending 2 messages");
        apply(
            &mut s,
            SyncResult { errors: vec!["Couldn't send \"Quarterly figures\": refused".into()], ..piece(step(SyncStage::Sending, "", 1, 2)) },
        );
        apply(&mut s, piece(step(SyncStage::Checking, "Inbox", 1, 3)));
        let r = row(&mut s, imap);
        assert_eq!((r["phase"].as_str(), r["folder"].as_str(), r["total"].as_u64()), (Some("newMail"), Some("Inbox"), Some(0)));
        apply(&mut s, SyncResult { folders: mail(&[5, 6], 0), ..piece(step(SyncStage::Reading, "Inbox", 2, 3)) });
        let r = row(&mut s, imap);
        assert_eq!((r["folder"].as_str(), r["done"].as_u64(), r["total"].as_u64(), r["new"].as_u64()), (Some("Inbox"), Some(2), Some(3), Some(2)));
        assert_eq!(row(&mut s, ms)["state"], json!("waiting"));
        // Its last piece: done, with what it came to and its problem.
        apply(&mut s, SyncResult { account: imap, folders: mail(&[4], 6), ..Default::default() });
        let r = row(&mut s, imap);
        assert_eq!(
            (r["state"].as_str(), r["new"].as_u64(), r["folder"].as_str(), r["kept"].as_bool()),
            (Some("done"), Some(3), Some(""), Some(false))
        );
        assert_eq!(r["problems"], json!(["Couldn't send \"Quarterly figures\": refused"]));
        assert_eq!(progress(&mut s)["running"], json!(true), "the Microsoft account is still to go");

        // The Microsoft account's pieces: the folders, then a folder's mail, then stopped.
        let inbox = GraphFolder { id: "in".into(), name: "Inbox".into(), kind: FolderKind::Inbox, total: 500, ..Default::default() };
        let listing = Some(FolderListing { folders: vec![inbox], complete: true });
        graph(&mut s, GraphMailResult { account: ms, listing, partial: true, ..Default::default() });
        assert_eq!((row(&mut s, ms)["state"].as_str(), row(&mut s, ms)["phase"].as_str()), (Some("running"), Some("folders")));
        let at = Some(GraphProgress { folder: "in".into(), read: 150, total: 500, number: 1, folders: 1 });
        graph(&mut s, GraphMailResult { account: ms, progress: at, partial: true, ..Default::default() });
        let r = row(&mut s, ms);
        assert_eq!(
            (r["phase"].as_str(), r["folder"].as_str(), r["done"].as_u64(), r["total"].as_u64()),
            (Some("newMail"), Some("Inbox"), Some(150), Some(500))
        );
        graph(&mut s, GraphMailResult { account: ms, stopped: true, ..Default::default() });
        let all = progress(&mut s);
        assert_eq!((all["running"].as_bool(), all["accounts"][1]["state"].as_str()), (Some(false), Some("stopped")));
        assert_eq!(all["accounts"][0]["state"], json!("done"));
    }

    #[test]
    fn a_run_that_changed_nothing_keeps_the_last_result() {
        let (mut s, imap, ms) = session();
        // No run was begun: the first piece begins one, of its account alone.
        apply(&mut s, SyncResult { account: imap, folders: mail(&[1, 2], 2), ..Default::default() });
        let r = row(&mut s, imap);
        assert_eq!((r["state"].as_str(), r["new"].as_u64(), r["kept"].as_bool()), (Some("done"), Some(2), Some(false)));
        // A run every few minutes that brings nothing doesn't wipe it with "0 new".
        apply(&mut s, SyncResult { account: imap, partial: true, step: step(SyncStage::Connecting, "", 0, 0), ..Default::default() });
        assert_eq!((row(&mut s, imap)["state"].as_str(), row(&mut s, imap)["new"].as_u64()), (Some("running"), Some(0)));
        apply(&mut s, SyncResult { account: imap, ..Default::default() });
        let r = row(&mut s, imap);
        assert_eq!((r["state"].as_str(), r["new"].as_u64(), r["kept"].as_bool()), (Some("done"), Some(2), Some(true)));
        // A run with a problem is something to tell: it takes the place of the last result...
        apply(&mut s, SyncResult { account: imap, errors: vec!["Couldn't open \"Clutter\": no".into()], ..Default::default() });
        let r = row(&mut s, imap);
        assert_eq!((r["new"].as_u64(), r["problems"].as_array().map(Vec::len), r["kept"].as_bool()), (Some(0), Some(1), Some(false)));
        // ...and so is one that was stopped, even with nothing in it.
        apply(&mut s, SyncResult { account: imap, stopped: true, ..Default::default() });
        let r = row(&mut s, imap);
        assert_eq!((r["state"].as_str(), r["problems"].as_array().map(Vec::len), r["kept"].as_bool()), (Some("stopped"), Some(0), Some(false)));
        // A new run takes the place of the last: only its accounts are in it.
        graph(&mut s, GraphMailResult { account: ms, ..Default::default() });
        let all = progress(&mut s);
        assert_eq!((all["accounts"].as_array().map(Vec::len), all["accounts"][0]["id"].as_u64()), (Some(1), Some(ms)));
        assert_eq!(all["accounts"][0]["state"], json!("done"));
        // An answer for an account that isn't there changes nothing.
        let before = s.sync_progress.clone();
        assert!(s.run("sync.apply", &json!({"result": {"account": 424242}})).is_err());
        assert!(s.run("sync.applyGraph", &json!({"result": {"account": imap}})).is_err());
        assert_eq!(s.sync_progress, before);
    }

    #[test]
    fn graph_mail_counts_as_new_and_problems_are_kept_within_bounds() {
        let (mut s, _, ms) = session();
        let inbox = GraphFolder { id: "in".into(), name: "Inbox".into(), kind: FolderKind::Inbox, ..Default::default() };
        graph(
            &mut s,
            GraphMailResult {
                account: ms,
                listing: Some(FolderListing { folders: vec![inbox], complete: true }),
                partial: true,
                ..Default::default()
            },
        );
        let page = |id: &str| {
            serde_json::from_value::<MessagePage>(json!({"folder": "in", "messages": [{"server": {"id": id, "folder": "in"}, "subject": "Hello"}]}))
                .unwrap()
        };
        let errors: Vec<String> = (0..100).map(|n| format!("Problem {n}: {}", "x".repeat(2_000))).collect();
        graph(&mut s, GraphMailResult { account: ms, pages: vec![page("AAA1"), page("AAA2")], errors, partial: true, ..Default::default() });
        graph(&mut s, GraphMailResult { account: ms, whole: true, ..Default::default() });
        let r = row(&mut s, ms);
        let problems = r["problems"].as_array().unwrap();
        assert_eq!((r["state"].as_str(), problems.len()), (Some("done"), MAX_PROBLEMS));
        assert!(problems.iter().all(|p| p.as_str().is_some_and(|p| p.chars().count() <= MAX_PROBLEM_CHARS)));
        assert_eq!((r["new"].as_u64(), s.mailbox.messages.values().filter(|m| m.account == ms).count()), (Some(2), 2));
        // Junk params are nothing to it: it reads and opens nothing.
        for junk in [json!(null), json!([1, 2]), json!({"account": "x", "accounts": [[]]}), json!("sync")] {
            assert_eq!(s.run("sync.progress", &junk).unwrap()["accounts"].as_array().map(Vec::len), Some(1));
        }
    }
}

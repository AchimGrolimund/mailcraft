//! Messages of a Microsoft account fetched whole (R3.2; R3 spec S12, S14, S15): in the
//! background, newest first, within the account's "Download messages in full"; at once, ahead of
//! that, for the message the owner opens; and before Reply, Reply All, Forward, Save Message As
//! and Save Attachment, which then go on. Only while a Graph token from this session is good:
//! each Send/Receive renews it. Cancel stops it all until the next Send/Receive.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;

use mailcraft_engine::cmd::graph_mail;
use mailcraft_model::sync::{GraphFetchRequest, GraphFetchResult};
use mailcraft_model::{AccountKind, Id};
use serde_json::{Value, json};

use crate::{MailApp, now_ms};

/// Messages asked for at a time in the background.
const BATCH: usize = 10;
/// How long the background waits before looking again when there was nothing to fetch.
const IDLE_MS: f64 = 20_000.0;

/// What the host is asked to fetch ([`crate::Services::graph_fetch`]).
pub struct GraphFetchJob {
    pub request: GraphFetchRequest,
    /// A Graph token of this session that covers the mail.
    pub access: String,
    pub cancel: Arc<AtomicBool>,
}

/// The state of fetching, in the window.
#[derive(Default)]
pub struct Fetching {
    /// The background's batch under way, for which account.
    background: Option<(Id, Receiver<GraphFetchResult>)>,
    /// The owner's message under way: which one, and the answer to come.
    now: Option<(Id, Receiver<GraphFetchResult>)>,
    /// What waits for that message: a command and its params.
    then: Option<(String, Value)>,
    /// The message last asked for when it was opened (not asked again while it stays open).
    opened: Option<Id>,
    /// Stopped by Cancel, until the next Send/Receive.
    pub(crate) paused: bool,
    pub(crate) cancel: Arc<AtomicBool>,
    idle_until: f64,
    /// The last try couldn't reach Microsoft.
    pub(crate) offline: bool,
    /// For the status bar: how many are fetched of how many, while some aren't.
    pub(crate) progress: String,
}

impl Fetching {
    /// Whether anything is being fetched.
    pub fn busy(&self) -> bool {
        self.background.is_some() || self.now.is_some()
    }

    /// Something new may be waiting: the background looks again at once.
    pub(crate) fn wake(&mut self) {
        self.idle_until = 0.0;
    }

    /// Whether `message` is being fetched for the owner now.
    pub fn fetching(&self, message: Id) -> bool {
        self.now.as_ref().is_some_and(|(m, _)| *m == message)
    }
}

/// The commands that need a message whole, and the message they are about.
fn needs_whole(app: &MailApp, id: &str, params: &Value) -> Option<Id> {
    if !matches!(id, "mail.reply" | "mail.replyAll" | "mail.forward" | "mail.editDraft" | "file.exportMessage" | "mail.saveAttachment") {
        return None;
    }
    let selected = match app.session.sel.messages.as_slice() {
        [one] => Some(*one),
        _ => None,
    };
    let message = params.get("id").and_then(Value::as_u64).or(selected)?;
    app.session.mailbox.message(message).filter(|m| m.unfetched && m.server.is_some()).map(|m| m.id)
}

/// A Graph token of this session for `account` that is still good.
fn access(app: &MailApp, account: Id) -> Option<String> {
    app.graph_tokens.get(&account).filter(|t| t.fresh(mailcraft_time::now())).map(|t| t.access.clone())
}

impl MailApp {
    /// A command that needs its message whole waits for it to be fetched, and then runs.
    pub(crate) fn fetch_first(&mut self, id: &str, params: &Value) -> Option<Result<Value, String>> {
        let message = needs_whole(self, id, params)?;
        let account = self.session.mailbox.message(message)?.account;
        if !self.fetch_now(account, message) {
            return Some(Err(tl!("This message isn't on this computer yet. It will download when you're online.").to_string()));
        }
        self.fetching.then = Some((id.to_string(), params.clone()));
        self.status(tl!("Downloading the message first…"));
        Some(Ok(json!({"pending": "download", "message": message})))
    }

    /// Fetch `message` at once, ahead of the background. Whether it could be asked for.
    fn fetch_now(&mut self, account: Id, message: Id) -> bool {
        if self.fetching.fetching(message) {
            return true;
        }
        let (Some(fetch), Some(access)) = (&self.services.graph_fetch, access(self, account)) else {
            self.fetching.offline = true;
            return false;
        };
        let Ok(request) = graph_mail::graph_fetch_request(&self.session, account, Some(message), 1) else { return false };
        if request.ids.is_empty() {
            return false;
        }
        let job = GraphFetchJob { request, access, cancel: Arc::new(AtomicBool::new(false)) };
        self.fetching.now = Some((message, fetch(job)));
        true
    }

    /// Each frame: take in what came, fetch what the owner opened, and keep the background going.
    /// (Public for a host's tests, which have no window.)
    pub fn drive_fetch(&mut self) {
        self.poll_fetch_now();
        self.poll_fetch_background();
        if !self.session.graph_mail || self.services.graph_fetch.is_none() {
            return;
        }
        // The message the reading pane shows, at once.
        let shown = match self.session.sel.messages.as_slice() {
            [one] => Some(*one),
            _ => None,
        };
        if shown != self.fetching.opened {
            self.fetching.opened = shown;
            if let Some(m) =
                shown.and_then(|m| self.session.mailbox.message(m)).filter(|m| m.unfetched && m.server.is_some()).map(|m| (m.account, m.id))
                && self.fetching.now.is_none()
            {
                self.fetch_now(m.0, m.1);
            }
        }
        // The background, a few at a time.
        if self.fetching.background.is_some() || self.fetching.paused || now_ms() < self.fetching.idle_until {
            return;
        }
        let accounts: Vec<Id> = self.session.mailbox.accounts.iter().filter(|a| a.kind == AccountKind::Microsoft).map(|a| a.id).collect();
        for account in accounts {
            let Some(token) = access(self, account) else { continue };
            let Ok(request) = graph_mail::graph_fetch_request(&self.session, account, None, BATCH) else { continue };
            if request.ids.is_empty() {
                continue;
            }
            let Some(fetch) = &self.services.graph_fetch else { return };
            self.fetching.cancel = Arc::new(AtomicBool::new(false));
            let job = GraphFetchJob { request, access: token, cancel: self.fetching.cancel.clone() };
            self.fetching.background = Some((account, fetch(job)));
            return;
        }
        self.fetching.idle_until = now_ms() + IDLE_MS;
        self.fetching.progress.clear();
    }

    fn poll_fetch_now(&mut self) {
        let Some((message, rx)) = &self.fetching.now else { return };
        let message = *message;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => GraphFetchResult { offline: true, ..Default::default() },
        };
        self.fetching.now = None;
        self.fetching.offline = result.offline;
        let account = result.account;
        if account != 0
            && let Err(e) = graph_mail::apply_graph_fetch(&mut self.session, result)
        {
            self.status(e.to_string());
        }
        let whole = self.session.mailbox.message(message).is_some_and(|m| !m.unfetched);
        if let Some((command, params)) = self.fetching.then.take() {
            if whole {
                let _ = self.run(&command, params);
            } else {
                self.status(tl!("This message isn't on this computer yet. It will download when you're online."));
            }
        }
    }

    fn poll_fetch_background(&mut self) {
        let Some((account, rx)) = &self.fetching.background else { return };
        let account = *account;
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => GraphFetchResult { account, offline: true, ..Default::default() },
        };
        self.fetching.background = None;
        self.fetching.offline = result.offline;
        let stuck = result.offline || result.busy_for.is_some() || (result.fetched.is_empty() && !result.errors.is_empty());
        if stuck {
            // Not now: try again in a while (a Send/Receive renews the token too).
            let wait = f64::from(result.busy_for.unwrap_or(0)).mul_add(1_000.0, IDLE_MS);
            self.fetching.idle_until = now_ms() + wait;
        }
        match graph_mail::apply_graph_fetch(&mut self.session, result) {
            Ok(said) => {
                let (waiting, large) = (said["waiting"].as_u64().unwrap_or(0), said["large"].as_u64().unwrap_or(0));
                let all = self.session.mailbox.messages.values().filter(|m| m.account == account && m.server.is_some()).count() as u64;
                self.fetching.progress = match (waiting.saturating_sub(large), large) {
                    (0, 0) => String::new(),
                    (0, large) => crate::i18n::fmt(tl!("{large} large messages download when opened"), &[("large", &large.to_string())]),
                    (_, 0) => crate::i18n::fmt(
                        tl!("Downloading {done} of {all}"),
                        &[("done", &all.saturating_sub(waiting).to_string()), ("all", &all.to_string())],
                    ),
                    (_, large) => crate::i18n::fmt(
                        tl!("Downloading {done} of {all} · {large} large messages download when opened"),
                        &[("done", &all.saturating_sub(waiting).to_string()), ("all", &all.to_string()), ("large", &large.to_string())],
                    ),
                };
            }
            Err(e) => self.status(e.to_string()),
        }
    }

    /// Cancel: the background stops after the message in hand, until the next Send/Receive.
    pub(crate) fn pause_fetch(&mut self) {
        self.fetching.paused = true;
        self.fetching.cancel.store(true, Ordering::Relaxed);
    }

    /// A Send/Receive started: the background may go on.
    pub(crate) fn resume_fetch(&mut self) {
        self.fetching.paused = false;
        self.fetching.idle_until = 0.0;
    }
}

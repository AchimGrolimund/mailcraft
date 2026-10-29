// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The host's side of an Exchange server account (#252, E6): `mailcraft-ews` runs its requests over
//! [`Transport`], which is one HTTPS exchange of `mailcraft_net::https` at a time, on a thread of
//! the run's own, and the pieces of the run reach the window through a channel.
//!
//! **The credential** is a Basic sign-in over TLS: the account's user name (the settings', else its
//! address) and the password the window hands over with the job. It lives in memory for the length
//! of the run, is not written anywhere, and is in no log line.
//!
//! **The certificate.** The system's trust first; a certificate the system doesn't trust is accepted
//! only when it is exactly the one the account trusts (its fingerprint). When it is not accepted the
//! first request does not go, so no password has gone either, and `mailcraft_net::NetError::Certificate`
//! comes out of `https::send`. It is *not* a plain TLS error here: the transport keeps what it says
//! ([`Transport::problem`]) and the run's last piece carries the question (`EwsMailResult::untrusted`
//! is the fingerprint of a certificate the owner may be asked to trust, or that has changed since
//! they did), so that the window stops and the account says it needs the certificate question. A
//! certificate nobody may trust (expired, for another name) is told in words and never offered.
//!
//! **Waiting.** A busy server asks the run to wait; the thread sleeps in small steps, so that Cancel
//! is heard, and gives up only when the run is called off.

use std::future::{Future, ready};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mailcraft_ews::{Client, HttpExchange, HttpRequest, HttpResponse, Secret, TransportError};
use mailcraft_model::sync::{EwsFetchRequest, EwsMailRequest, EwsMailResult, GraphFetchResult};
use mailcraft_net::{CertificateProblem, NetError};
use mailcraft_ui_egui::ews_mail::{EwsFetchJob, EwsJob};

/// What the run says when the secure connection could not be had for a reason the owner can't
/// answer (an expired certificate, one for another server): in words with nothing of the certificate.
const NO_SECURE_LINE: &str = "Couldn't set up a secure connection with the server";

/// One HTTPS exchange after another with the server of one account.
pub(crate) struct Transport {
    /// The fingerprint of the certificate the account trusts besides the system's; empty for none.
    trusted: String,
    /// Why the server's certificate was not accepted, when that is what stopped an exchange.
    stopped_at: Mutex<Option<CertificateProblem>>,
}

impl Transport {
    pub(crate) fn new(trusted: &str) -> Self {
        Self { trusted: trusted.trim().to_owned(), stopped_at: Mutex::new(None) }
    }

    /// What the certificate question is, when the run was stopped by one.
    pub(crate) fn problem(&self) -> Option<CertificateProblem> {
        self.stopped_at.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).clone()
    }

    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        let fields: Vec<(&str, &str)> = request.fields().collect();
        let ask = mailcraft_net::https::Request {
            method: request.method().as_str(),
            url: request.url(),
            fields: &fields,
            body: request.body(),
            max_response: request.max_response_bytes(),
            trusted: &self.trusted,
        };
        match mailcraft_net::https::send(&ask) {
            Ok(answer) => {
                let mut response = HttpResponse::new(answer.status, answer.body);
                if let Some(location) = &answer.location {
                    response = response.with_field("Location", location);
                }
                for challenge in &answer.challenges {
                    response = response.with_field("WWW-Authenticate", challenge);
                }
                if let Some(wait) = &answer.retry_after {
                    response = response.with_field("Retry-After", wait);
                }
                if let Some(why) = &answer.policy_reason {
                    response = response.with_field("X-EWS-Policy-Reason", why);
                }
                Ok(response)
            }
            Err(error) => Err(self.judged(error)),
        }
    }

    /// What a failed exchange is to the run. A certificate that is not accepted is kept apart (never
    /// a plain TLS error to the run's machine: it ends the run at once and asks); the rest is
    /// told by its kind.
    fn judged(&self, error: NetError) -> TransportError {
        match error {
            NetError::Certificate(problem) => {
                *self.stopped_at.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(*problem);
                TransportError::Tls
            }
            NetError::Tls { .. } => TransportError::Tls,
            NetError::Connect { .. } => TransportError::Unreachable,
            _ => TransportError::Other,
        }
    }
}

impl HttpExchange for Transport {
    fn exchange(&self, request: &HttpRequest) -> impl Future<Output = Result<HttpResponse, TransportError>> {
        // (The call blocks, on the run's own thread.)
        ready(self.send(request))
    }
}

/// The last piece of a run, with the certificate question in it when a certificate stopped the run:
/// the fingerprint of one the owner may be asked about, else the words that it can't be trusted. The
/// line the run wrote for the secure connection that failed is replaced, not added to.
pub(crate) fn settled(mut piece: EwsMailResult, problem: Option<&CertificateProblem>) -> EwsMailResult {
    let Some(problem) = problem.filter(|_| !piece.partial) else { return piece };
    piece.errors.retain(|line| line != NO_SECURE_LINE);
    piece.whole = false;
    if problem.can_be_trusted() {
        piece.untrusted = problem.info().fingerprint.clone();
    } else {
        piece.errors.insert(0, format!("{problem}"));
    }
    piece
}

/// The same for a batch fetched: no question can be asked from there, so it is told in words.
pub(crate) fn settled_fetch(mut result: GraphFetchResult, problem: Option<&CertificateProblem>) -> GraphFetchResult {
    if let Some(problem) = problem {
        result.errors.retain(|line| !line.ends_with(NO_SECURE_LINE));
        result.errors.insert(0, format!("Couldn't download the messages from the server: {problem}"));
    }
    result
}

/// The Basic credential of `username` (the address when it is empty) and `password`, as a client for
/// `url`. Why not: words that name neither.
fn client(url: &str, username: &str, email: &str, password: &str) -> Result<Client, &'static str> {
    let name = if username.trim().is_empty() { email.trim() } else { username.trim() };
    let secret = Secret::basic(name, password).map_err(|_| "The user name or the password can't be used to sign in")?;
    Client::new(url, secret).map_err(|_| "The server's address can't be used")
}

/// Sleeps `ms` milliseconds in small steps; false when the run was called off meanwhile.
fn wait(cancel: &AtomicBool, ms: u32) -> bool {
    let mut left = u64::from(ms);
    while left > 0 {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        let step = left.min(100);
        std::thread::sleep(Duration::from_millis(step));
        left = left.saturating_sub(step);
    }
    !cancel.load(Ordering::Relaxed)
}

/// Runs an Exchange run on a thread of its own, handing the pieces to the window as they come.
pub(crate) fn run_mail(job: EwsJob, wake: &egui::Context) -> Receiver<EwsMailResult> {
    // Through a channel short enough that a busy window holds the run back instead of piling pieces up.
    let (tx, rx) = sync_channel(8);
    let wake = wake.clone();
    let spawned = std::thread::Builder::new().name("exchange send/receive".into()).spawn(move || {
        let EwsJob { request, secret, cancel } = job;
        run_mail_with(&request, &secret, &cancel, &mut |piece| {
            // Nobody is listening any more (the window closed): stop.
            if tx.send(piece).is_err() {
                cancel.store(true, Ordering::Relaxed);
            }
            wake.request_repaint();
        });
    });
    // A dropped sender reads as "stopped unexpectedly" in the app.
    if let Err(e) = spawned {
        log::error!("couldn't start Send/Receive for an Exchange server account: {e}");
    }
    rx
}

/// [`run_mail`] on this thread: every piece through `emit`.
pub(crate) fn run_mail_with(request: &EwsMailRequest, password: &str, cancel: &AtomicBool, emit: &mut dyn FnMut(EwsMailResult)) {
    let client = match client(&request.url, &request.username, &request.email, password) {
        Ok(client) => client,
        Err(why) => return emit(mailcraft_ews::failed_run(request, why)),
    };
    let transport = Transport::new(&request.trusted);
    let mut pause = |ms: u32| wait(cancel, ms);
    crate::block_on::block_on(mailcraft_ews::sync_mail(&client, &transport, request, cancel, &mut pause, &mut |piece| {
        emit(settled(piece, transport.problem().as_ref()));
    }));
}

/// Fetches messages of an Exchange server account whole on a thread of its own.
pub(crate) fn run_fetch(job: EwsFetchJob, wake: &egui::Context) -> Receiver<GraphFetchResult> {
    let (tx, rx) = std::sync::mpsc::channel();
    let wake = wake.clone();
    let spawned = std::thread::Builder::new().name("exchange message download".into()).spawn(move || {
        let EwsFetchJob { request, secret, cancel } = job;
        let _ = tx.send(run_fetch_with(&request, &secret, &cancel));
        wake.request_repaint();
    });
    if let Err(e) = spawned {
        log::error!("couldn't start a download for an Exchange server account: {e}");
    }
    rx
}

/// [`run_fetch`] on this thread.
pub(crate) fn run_fetch_with(request: &EwsFetchRequest, password: &str, cancel: &Arc<AtomicBool>) -> GraphFetchResult {
    let client = match client(&request.url, &request.username, &request.email, password) {
        Ok(client) => client,
        Err(why) => {
            return GraphFetchResult { account: request.account, email: request.email.clone(), errors: vec![why.to_owned()], ..Default::default() };
        }
    };
    let transport = Transport::new(&request.trusted);
    let mut pause = |ms: u32| wait(cancel, ms);
    let result = crate::block_on::block_on(mailcraft_ews::fetch_messages(&client, &transport, request, cancel, &mut pause));
    settled_fetch(result, transport.problem().as_ref())
}

#[cfg(test)]
#[path = "ews_runner_tests.rs"]
mod tests;

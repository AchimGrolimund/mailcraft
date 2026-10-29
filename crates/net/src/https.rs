// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! One HTTPS exchange with an Exchange server (#252): Autodiscover and Exchange Web Services.
//!
//! The rules are those of every connection of this crate (TLS, or plain only to this machine;
//! no redirect followed, no cookie, nothing logged of the request, answers read within limits),
//! with one addition for the servers organisations run themselves, whose certificates the system
//! often doesn't know: [`Request::trusted`] may hold the SHA-256 fingerprint of the one
//! certificate the owner chose to trust for the account. The certificate is then accepted when
//! the system trusts it **or** when it is exactly that one, in date and for the server's name
//! ([`crate::pinned`]). It is never "ignore errors":
//!
//! - an expired, not-yet-valid or wrong-name certificate is refused whatever the fingerprint,
//!   and is not offered for trust ([`CertificateProblem::Unacceptable`]);
//! - a certificate the system doesn't trust and the account doesn't know is refused with what
//!   setup shows the owner ([`CertificateProblem::NotTrusted`]: fingerprint, subject, issuer,
//!   dates), before any request, so no password has gone;
//! - a different certificate than the trusted one is refused and asks again
//!   ([`CertificateProblem::Changed`]).
//!
//! The call blocks; a host calls it on a background thread.

use std::time::Duration;

use crate::NetError;
use crate::cert::{self, CertificateProblem};
use crate::http::{self, Request as HttpRequest, Url};

/// How long a request has to connect and to be answered.
const PATIENCE: Duration = Duration::from_secs(30);

/// A request to an Exchange server.
#[derive(Clone, Copy)]
pub struct Request<'a> {
    /// `GET` or `POST`.
    pub method: &'a str,
    /// An `https://` address (`http://` only to this machine, for tests). No user name in it.
    pub url: &'a str,
    /// The header fields to send, `Authorization` among them. Sent exactly; nothing here logs them.
    pub fields: &'a [(&'a str, &'a str)],
    /// The body.
    pub body: &'a [u8],
    /// The most body octets to read of the answer.
    pub max_response: usize,
    /// The SHA-256 fingerprint (64 hexadecimal digits) of the one certificate the owner chose to
    /// trust for this account although the system doesn't; empty for none.
    pub trusted: &'a str,
}

impl std::fmt::Debug for Request<'_> {
    /// The method and the body's length: no address, no credential, no content.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request").field("method", &self.method).field("body_len", &self.body.len()).finish()
    }
}

/// An answer: the status, the few header fields that are read, and the body.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct Response {
    /// The status code.
    pub status: u16,
    /// The body (within [`Request::max_response`]).
    pub body: Vec<u8>,
    /// `Location`, as written.
    pub location: Option<String>,
    /// Every `WWW-Authenticate`, in order (at most 16, each short).
    pub challenges: Vec<String>,
    /// `Retry-After`.
    pub retry_after: Option<String>,
    /// `X-EWS-Policy-Reason`.
    pub policy_reason: Option<String>,
}

impl std::fmt::Debug for Response {
    /// Status and body length: the body may hold someone's mail.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Response").field("status", &self.status).field("body_len", &self.body.len()).finish()
    }
}

/// Performs `request` and returns the answer, whatever its status.
///
/// An error is a [`NetError`]: [`NetError::Certificate`] when the server's certificate is not
/// accepted (nothing of the request went), [`NetError::Config`] for an address or a fingerprint
/// that is no such thing, and the usual connection and protocol errors otherwise.
pub fn send(request: &Request<'_>) -> Result<Response, NetError> {
    let url = Url::parse(request.url)?;
    let pin = match request.trusted.trim() {
        "" => None,
        fingerprint => Some(
            cert::parse_fingerprint(fingerprint)
                .ok_or_else(|| NetError::Config("the trusted certificate's fingerprint isn't a SHA-256 fingerprint".into()))?,
        ),
    };
    let http_request =
        HttpRequest { method: request.method, headers: request.fields, body: Some(request.body), patience: PATIENCE, max_body: request.max_response };
    if !url.secure {
        return http::request(&url, &http_request).map(response);
    }
    let (config, verifier) = crate::tls::pinned_config(pin).map_err(|reason| NetError::Tls { host: url.host.clone(), reason })?;
    match http::request_with(&url, &http_request, Some(config)) {
        Ok(reply) => Ok(response(reply)),
        // A handshake that stopped at the certificate says which, and why; any other failure is the usual one.
        Err(error @ NetError::Tls { .. }) => Err(verifier.take_problem().map_or(error, |problem| NetError::Certificate(Box::new(problem)))),
        Err(other) => Err(other),
    }
}

fn response(reply: http::Reply) -> Response {
    Response {
        status: reply.status,
        body: reply.body,
        location: reply.location,
        challenges: reply.challenges,
        retry_after: reply.retry_after,
        policy_reason: reply.policy_reason,
    }
}

/// What a person is shown about a certificate (re-exported for hosts).
pub use crate::cert::CertificateInfo;

/// The problem with the server's certificate behind `error`, when that is what it is.
pub fn certificate_problem(error: &NetError) -> Option<&CertificateProblem> {
    match error {
        NetError::Certificate(problem) => Some(problem.as_ref()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "https_tests.rs"]
mod https_tests;

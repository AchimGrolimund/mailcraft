// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Descriptions of one HTTP exchange, without performing it.
//!
//! The crate builds an [`HttpRequest`]; the program performs it however it can (a native HTTP
//! client, the browser's Fetch API, a test double) and describes what came back in an
//! [`HttpResponse`]. The program must:
//!
//! - send the method, address, header fields and body exactly as given;
//! - follow no redirect itself (a redirect carries the owner's credentials somewhere else);
//! - read [`HttpRequest::max_response_bytes`] before reading the body, and stop reading as soon as
//!   the body passes it, reporting [`TransportError::BodyTooLarge`] (the crate refuses a longer
//!   body anyway, but only after it has been read);
//! - copy into the response the header fields named in [`HttpResponse::FIELDS_READ`].

use std::fmt;
use std::future::Future;

/// The most an answer may weigh, in octets. (Messages come down in pieces, never as one
/// answer of this size.)
pub const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;
/// The longest address a request goes to.
const MAX_ADDRESS: usize = 2_048;
/// The longest header field value the response keeps; longer ones are dropped.
pub const MAX_FIELD_VALUE_LEN: usize = 8 * 1024;

/// A credential: a header value that is never shown, in `Debug` output or anywhere else.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    /// Keeps `value` (`Basic …`, `Bearer …`) for the `Authorization` field. CR, LF and NUL are
    /// taken out, so that it can never end the field.
    pub fn new(value: &str) -> Self {
        Self(value.chars().filter(|c| !c.is_control()).collect())
    }

    /// The value, for whoever sends it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// Why an address is not one a request goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EndpointError {
    /// Not an `http` or `https` address, or too long, or with a character no address has.
    Malformed,
    /// Plain `http` to anything but the machine itself: an unencrypted connection carries
    /// nothing of the owner's.
    NotEncrypted,
    /// A user name or password written into the address.
    Credentials,
}

impl fmt::Display for EndpointError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "the server's address is not a web address",
            Self::NotEncrypted => "the server's address must start with https://",
            Self::Credentials => "the server's address must not hold a user name or a password",
        })
    }
}

impl std::error::Error for EndpointError {}

/// Checks the transport rule on `url`: `https`, or plain `http` to `localhost`, `127.0.0.0/8` or
/// `[::1]` only (for the tests). Nothing is fetched from it.
pub fn check_endpoint(url: &str) -> Result<(), EndpointError> {
    if url.len() > MAX_ADDRESS || url.chars().any(|c| c.is_control() || c.is_whitespace() || !c.is_ascii()) {
        return Err(EndpointError::Malformed);
    }
    let (secure, rest) = match url.split_once("://") {
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("https") => (true, rest),
        Some((scheme, rest)) if scheme.eq_ignore_ascii_case("http") => (false, rest),
        _ => return Err(EndpointError::Malformed),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() {
        return Err(EndpointError::Malformed);
    }
    if authority.contains('@') {
        return Err(EndpointError::Credentials);
    }
    if secure {
        return Ok(());
    }
    let host = match authority.strip_prefix('[') {
        Some(inside) => inside.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    let loopback = host.eq_ignore_ascii_case("localhost") || host == "::1" || {
        let mut parts = host.split('.');
        parts.next() == Some("127")
            && parts.clone().count() == 3
            && parts.all(|part| !part.is_empty() && part.len() <= 3 && part.bytes().all(|b| b.is_ascii_digit()))
    };
    if loopback { Ok(()) } else { Err(EndpointError::NotEncrypted) }
}

/// A request to send: always a `POST` of a `text/xml` body.
#[derive(Clone, PartialEq, Eq)]
pub struct HttpRequest {
    url: String,
    authorization: Secret,
    body: Vec<u8>,
    max_response: usize,
}

impl HttpRequest {
    /// A `POST` of `body` to `url`, with `authorization` in its `Authorization` field.
    pub fn soap(url: &str, authorization: &Secret, body: String) -> Result<Self, EndpointError> {
        check_endpoint(url)?;
        Ok(Self { url: url.to_owned(), authorization: authorization.clone(), body: body.into_bytes(), max_response: RESPONSE_LIMIT })
    }

    /// Lowers the largest response body accepted to `bytes` (it can only go down).
    #[must_use]
    pub fn with_response_limit(mut self, bytes: usize) -> Self {
        self.max_response = bytes.min(RESPONSE_LIMIT);
        self
    }

    /// The method.
    pub fn method(&self) -> &'static str {
        "POST"
    }

    /// The address.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The header fields to send, `Authorization` included.
    pub fn fields(&self) -> [(&'static str, &str); 3] {
        [("Authorization", self.authorization.expose()), ("Content-Type", "text/xml; charset=utf-8"), ("Accept", "text/xml")]
    }

    /// The body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }

    /// The most body octets to read from the response.
    pub fn max_response_bytes(&self) -> usize {
        self.max_response
    }
}

impl fmt::Debug for HttpRequest {
    /// The method and the body's length: no address, no credential, no content.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpRequest").field("body_len", &self.body.len()).field("max_response_bytes", &self.max_response).finish()
    }
}

/// What came back, as the program describes it.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct HttpResponse {
    status: u16,
    retry_after: Option<String>,
    policy_reason: Option<String>,
    body: Vec<u8>,
}

impl HttpResponse {
    /// The header fields the crate reads; others may be left out.
    pub const FIELDS_READ: [&'static str; 2] = ["Retry-After", "X-EWS-Policy-Reason"];

    /// A response with a status code and a body.
    pub fn new(status: u16, body: Vec<u8>) -> Self {
        Self { status, body, ..Self::default() }
    }

    /// Adds a header field; names are matched without regard to case and only those of
    /// [`HttpResponse::FIELDS_READ`] are kept. A value longer than [`MAX_FIELD_VALUE_LEN`] is
    /// dropped, and when a field comes twice the first value is kept.
    #[must_use]
    pub fn with_field(mut self, name: &str, value: &str) -> Self {
        if value.len() > MAX_FIELD_VALUE_LEN {
            return self;
        }
        let slot = if name.eq_ignore_ascii_case("retry-after") {
            &mut self.retry_after
        } else if name.eq_ignore_ascii_case("x-ews-policy-reason") {
            &mut self.policy_reason
        } else {
            return self;
        };
        if slot.is_none() {
            *slot = Some(value.trim_matches([' ', '\t']).to_owned());
        }
        self
    }

    /// The status code.
    pub fn status(&self) -> u16 {
        self.status
    }

    /// The `Retry-After` field value.
    pub fn retry_after(&self) -> Option<&str> {
        self.retry_after.as_deref()
    }

    /// The `X-EWS-Policy-Reason` field value: why an organisation does not allow the service.
    pub fn policy_reason(&self) -> Option<&str> {
        self.policy_reason.as_deref()
    }

    /// The body.
    pub fn body(&self) -> &[u8] {
        &self.body
    }
}

impl fmt::Debug for HttpResponse {
    /// Status and body length: the body may hold someone's mail.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpResponse").field("status", &self.status).field("body_len", &self.body.len()).finish()
    }
}

/// Why the program could not complete an exchange; no free text, so nothing private can travel
/// in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum TransportError {
    /// No answer in the time the program allows.
    TimedOut,
    /// The host could not be reached (name resolution, connection).
    Unreachable,
    /// TLS failed, a certificate among the causes.
    Tls,
    /// The body was longer than [`HttpRequest::max_response_bytes`].
    BodyTooLarge,
    /// The server redirected the request and the program, as asked, did not follow.
    RedirectRefused,
    /// The exchange was cancelled.
    Aborted,
    /// Anything else.
    Other,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::TimedOut => "the server did not answer in time; check the network connection and try again",
            Self::Unreachable => "the server could not be reached; check its address and the network connection",
            Self::Tls => "no secure connection could be set up, for example because the server's certificate is not trusted",
            Self::BodyTooLarge => "the answer passed the size limit set for this request and was not read further",
            Self::RedirectRefused => "the server redirected the request elsewhere and the redirect was not followed",
            Self::Aborted => "the request was cancelled before it completed",
            Self::Other => "the connection failed in a way the transport did not classify",
        })
    }
}

impl std::error::Error for TransportError {}

/// Performs one HTTP exchange.
///
/// The returned future need not be `Send`, so that a browser transport can implement it.
pub trait HttpExchange {
    /// Sends `request` and describes the response.
    fn exchange(&self, request: &HttpRequest) -> impl Future<Output = Result<HttpResponse, TransportError>>;
}

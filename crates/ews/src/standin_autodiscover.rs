// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A stand-in for Autodiscover (POX, [MS-OXDSCLI]) for the tests, next to [`crate::standin`]'s
//! Exchange Web Services. It runs inside the test: nothing here leaves the machine.
//!
//! It holds a client to what the specification documents:
//!
//! - **The request.** `POST` to `/autodiscover/autodiscover.xml` over `https://` (a plain
//!   `http://` address is never answered: the test fails instead), with the `Authorization` it
//!   was set up with (401 otherwise), a `text/xml` body whose document element `Autodiscover`
//!   is in the request namespace and holds `Request` with an `EMailAddress` and the
//!   `AcceptableResponseSchema` the answer is read for (400 otherwise).
//! - **The hosts.** Each host name is given a [`Route`]; a host nobody set up is unreachable.
//!   Every request is noted ([`Seen`]), so a test can say which hosts were sent a sign-in, and
//!   that none went where it should not.

use std::future::{Future, ready};
use std::sync::{Mutex, MutexGuard};

use crate::autodiscover::{ANSWER_SCHEMA, REQUEST_SCHEMA, RESPONSE_SCHEMA, host_of};
use crate::http::{HttpExchange, HttpRequest, HttpResponse, Method, TransportError};
use crate::xml::{self, escape_text};

/// The `Authorization` value the stand-in takes.
pub const BASIC: &str = "Basic c3RhbmRpbjpwYXNzd29yZA==";

/// What a host does with a request.
#[derive(Clone, Debug)]
pub enum Route {
    /// 200 with this document.
    Document(String),
    /// This status and no body.
    Status(u16),
    /// This status and a `Location`.
    Redirect(u16, String),
    /// 200 with these bytes, whatever they are.
    Bytes(Vec<u8>),
    /// The secure connection fails.
    TlsFails,
    /// Nobody answers.
    Unreachable,
}

/// A request that came.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    /// The address asked, as the client wrote it.
    pub url: String,
    /// The host in it.
    pub host: String,
    /// Whether the `Authorization` was the one the stand-in takes.
    pub authorized: bool,
    /// The address in the request's `EMailAddress`.
    pub email: String,
    /// The status answered.
    pub status: u16,
}

/// The stand-in's state.
#[derive(Debug, Default)]
pub struct State {
    /// Each host's route (lower case).
    pub routes: Vec<(String, Route)>,
    /// What came, in order.
    pub seen: Vec<Seen>,
}

/// The stand-in.
#[derive(Debug, Default)]
pub struct AutodiscoverStandIn {
    state: Mutex<State>,
}

impl AutodiscoverStandIn {
    /// A stand-in no host answers on.
    pub fn new() -> Self {
        Self::default()
    }

    /// Its state, to read or to change.
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Sets what `host` does (replacing what it did).
    #[must_use]
    pub fn route(self, host: &str, route: Route) -> Self {
        {
            let mut state = self.state();
            state.routes.retain(|(have, _)| have != host);
            state.routes.push((host.to_ascii_lowercase(), route));
        }
        self
    }

    /// The hosts that were sent a request, in order.
    pub fn hosts_asked(&self) -> Vec<String> {
        self.state().seen.iter().map(|seen| seen.host.clone()).collect()
    }
}

impl HttpExchange for AutodiscoverStandIn {
    fn exchange(&self, request: &HttpRequest) -> impl Future<Output = Result<HttpResponse, TransportError>> {
        ready(self.state().respond(request))
    }
}

impl State {
    fn respond(&mut self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        // A client of this crate never asks without encryption: the test would be wrong, not the host.
        let url = request.url().to_owned();
        assert!(url.get(..8).is_some_and(|scheme| scheme.eq_ignore_ascii_case("https://")), "Autodiscover was asked over plain HTTP");
        let host = host_of(&url).unwrap_or_default();
        let mut seen = Seen { url: url.clone(), host: host.clone(), authorized: request.field("authorization") == Some(BASIC), ..Seen::default() };
        let route = self.routes.iter().find(|(have, _)| *have == host).map(|(_, route)| route.clone());
        let result = match route {
            None | Some(Route::Unreachable) => Err(TransportError::Unreachable),
            Some(Route::TlsFails) => Err(TransportError::Tls),
            Some(route) => Ok(answer(request, &route, &mut seen)),
        };
        seen.status = result.as_ref().map_or(0, HttpResponse::status);
        self.seen.push(seen);
        result
    }
}

/// The stand-in's answer when the host is up.
fn answer(request: &HttpRequest, route: &Route, seen: &mut Seen) -> HttpResponse {
    let path = request.url().get(8..).and_then(|rest| rest.find('/').and_then(|at| rest.get(at..))).unwrap_or("");
    if request.method() != Method::Post || !path.eq_ignore_ascii_case("/autodiscover/autodiscover.xml") {
        return HttpResponse::new(404, Vec::new());
    }
    if !seen.authorized {
        return HttpResponse::new(401, Vec::new()).with_field("WWW-Authenticate", "Basic realm=\"stand-in\"");
    }
    if !request.field("content-type").is_some_and(|value| value.to_ascii_lowercase().starts_with("text/xml")) {
        return HttpResponse::new(400, Vec::new());
    }
    let Some(email) = asked_email(request.body()) else { return HttpResponse::new(400, Vec::new()) };
    seen.email = email;
    match route {
        Route::Document(document) => HttpResponse::new(200, document.clone().into_bytes()),
        Route::Status(status) => HttpResponse::new(*status, Vec::new()),
        Route::Redirect(status, location) => HttpResponse::new(*status, Vec::new()).with_field("Location", location),
        Route::Bytes(bytes) => HttpResponse::new(200, bytes.clone()),
        Route::TlsFails | Route::Unreachable => HttpResponse::new(500, Vec::new()),
    }
}

/// The address a well-formed POX request asks for.
fn asked_email(body: &[u8]) -> Option<String> {
    let root = xml::parse(body).ok()?;
    if !root.is(REQUEST_SCHEMA, "Autodiscover") {
        return None;
    }
    let request = root.child_in(REQUEST_SCHEMA, "Request")?;
    let schema = request.child_in(REQUEST_SCHEMA, "AcceptableResponseSchema")?;
    (schema.text.trim() == RESPONSE_SCHEMA).then_some(())?;
    let email = request.child_in(REQUEST_SCHEMA, "EMailAddress")?.text.trim().to_owned();
    (!email.is_empty()).then_some(email)
}

// ---- answers as the service writes them ----

fn wrap(account: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"{ANSWER_SCHEMA}\"><Response xmlns=\"{RESPONSE_SCHEMA}\"><User><DisplayName>Ada Example</DisplayName></User><Account><AccountType>email</AccountType>{account}</Account></Response></Autodiscover>"
    )
}

/// The answer of an Exchange Server with Outlook Anywhere: an internal (`EXCH`) and an external
/// (`EXPR`) `Protocol`, as the service writes them, with plenty besides the address.
pub fn settings(internal: &str, external: &str, server: &str) -> String {
    wrap(&format!(
        "<Action>settings</Action><Protocol><Type>EXCH</Type><Server>{server}</Server><ServerDN>/o=Example/ou=Exchange/cn=Servers/cn=EXCH01</ServerDN><ServerVersion>73C08A5B</ServerVersion><MdbDN>/o=Example/cn=Databases/cn=DB1</MdbDN><ASUrl>{internal}</ASUrl><EwsUrl>{internal}</EwsUrl><OOFUrl>{internal}</OOFUrl><AuthPackage>Ntlm</AuthPackage></Protocol>\
         <Protocol><Type>EXPR</Type><Server>{server}</Server><SSL>On</SSL><AuthPackage>Basic</AuthPackage><ASUrl>{external}</ASUrl><EwsUrl>{external}</EwsUrl></Protocol>\
         <Protocol><Type>WEB</Type><External><OWAUrl AuthenticationMethod=\"Fba\">https://owa.example.org/owa</OWAUrl></External></Protocol>"
    ))
}

/// An answer that sends the client on with `redirectAddr`.
pub fn redirect_addr(address: &str) -> String {
    wrap(&format!("<Action>redirectAddr</Action><RedirectAddr>{}</RedirectAddr>", escape_text(address)))
}

/// An answer that sends the client on with `redirectUrl`.
pub fn redirect_url(url: &str) -> String {
    wrap(&format!("<Action>redirectUrl</Action><RedirectUrl>{}</RedirectUrl>", escape_text(url)))
}

/// An answer that is an `Error` (600, "Invalid Request").
pub fn error_answer() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><Autodiscover xmlns=\"{ANSWER_SCHEMA}\"><Response><Error Time=\"10:00:00.0\" Id=\"1\"><ErrorCode>600</ErrorCode><Message>Invalid Request</Message><DebugData /></Error></Response></Autodiscover>"
    )
}

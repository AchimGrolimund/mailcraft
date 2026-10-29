// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Judging an answer: the HTTP status first, then the SOAP envelope, a fault, and the response
//! messages with their classes and codes.
//!
//! The elements that matter are found by their namespace and local name, never by a prefix.
//! Everything else in an answer (an element this crate does not know, a header, a remark) is
//! skipped.

use crate::error::{DEFAULT_BACK_OFF_MS, Error, MAX_BACK_OFF_MS, PolicyReason, ResponseClass, ResponseCode};
use crate::http::HttpResponse;
use crate::soap::{ERRORS, MESSAGES, SOAP};
use crate::xml::{self, Node};

/// The body of a SOAP answer that was not a fault.
#[derive(Debug)]
pub struct Reply {
    body: Node,
}

/// One response message of an operation's answer.
#[derive(Debug)]
pub struct Message<'n> {
    class: ResponseClass,
    code: ResponseCode,
    back_off_ms: Option<u32>,
    node: &'n Node,
}

impl Reply {
    /// Judges `response`: the status, then the document. `limit` is the most body there may be.
    pub fn judge(response: &HttpResponse, limit: usize) -> Result<Self, Error> {
        match response.status() {
            401 => return Err(Error::Unauthorized),
            403 => {
                return Err(match response.policy_reason() {
                    Some(reason) => Error::Blocked(PolicyReason::from_header(reason)),
                    None => Error::Forbidden,
                });
            }
            429 | 503 => return Err(Error::Busy { back_off_ms: retry_after_ms(response.retry_after()) }),
            200 | 500 => {}
            other => return Err(Error::Status(other)),
        }
        if response.body().len() > limit {
            return Err(Error::TooLarge);
        }
        match Self::parse(response.body()) {
            // (A 500 that is no fault is no answer of the service.)
            Ok(_) if response.status() == 500 => Err(Error::Status(500)),
            Err(Error::Malformed | Error::Unexpected) if response.status() == 500 => Err(Error::Status(500)),
            other => other,
        }
    }

    /// Reads the document `body`: a SOAP envelope whose body is not a fault. A fault is the
    /// error it names.
    pub fn parse(body: &[u8]) -> Result<Self, Error> {
        let root = xml::parse(body)?;
        if !root.is(SOAP, "Envelope") {
            return Err(Error::Unexpected);
        }
        let body = root.children.into_iter().find(|child| child.is(SOAP, "Body")).ok_or(Error::Unexpected)?;
        if let Some(fault) = body.child_in(SOAP, "Fault") {
            return Err(read_fault(fault));
        }
        Ok(Self { body })
    }

    /// The response messages of the answer to `operation` (`GetFolder`: the
    /// `GetFolderResponseMessage`s inside `GetFolderResponse`), in order.
    pub fn messages(&self, operation: &str) -> Result<Vec<Message<'_>>, Error> {
        let response = self.body.child_in(MESSAGES, &format!("{operation}Response")).ok_or(Error::Unexpected)?;
        let all = response.child_in(MESSAGES, "ResponseMessages").ok_or(Error::Unexpected)?;
        let name = format!("{operation}ResponseMessage");
        all.children.iter().filter(|child| child.is(MESSAGES, &name)).map(Message::read).collect()
    }
}

impl<'n> Message<'n> {
    fn read(node: &'n Node) -> Result<Self, Error> {
        let class = node.attribute("ResponseClass").and_then(ResponseClass::from_name).ok_or(Error::Unexpected)?;
        let code = node.child_in(MESSAGES, "ResponseCode").map(|code| ResponseCode::from_name(&code.text)).ok_or(Error::Unexpected)?;
        // The wait is a child of the message, or a value of its `MessageXml`.
        let back_off_ms =
            node.child_in(MESSAGES, "BackOffMilliseconds").map(|wait| wait.text.as_str()).or_else(|| back_off_value(node)).map(back_off_ms);
        Ok(Self { class, code, back_off_ms, node })
    }

    /// How the server says it went.
    pub fn class(&self) -> ResponseClass {
        self.class
    }

    /// The code the server gave.
    pub fn code(&self) -> &ResponseCode {
        &self.code
    }

    /// The element, for whoever reads what it holds.
    pub fn node(&self) -> &'n Node {
        self.node
    }

    /// The element when the message says it was done; the error it names when it was not.
    pub fn ok(&self) -> Result<&'n Node, Error> {
        if self.class != ResponseClass::Error {
            return Ok(self.node);
        }
        Err(match self.code {
            ResponseCode::ServerBusy => Error::Busy { back_off_ms: self.back_off_ms.unwrap_or(DEFAULT_BACK_OFF_MS) },
            ref code => Error::Server(code.clone()),
        })
    }
}

/// `Value Name="BackOffMilliseconds"` in a message's `MessageXml`.
fn back_off_value(node: &Node) -> Option<&str> {
    let values = node.child_in(MESSAGES, "MessageXml")?;
    values
        .children
        .iter()
        .find(|value| value.name == "Value" && value.attribute("Name") == Some("BackOffMilliseconds"))
        .map(|value| value.text.as_str())
}

/// A wait in milliseconds, as a server writes it: digits, capped; anything else is the default.
fn back_off_ms(text: &str) -> u32 {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return DEFAULT_BACK_OFF_MS;
    }
    // (A number of any length: too many digits is simply the most.)
    text.parse::<u64>().map_or(MAX_BACK_OFF_MS, |ms| u32::try_from(ms.min(u64::from(MAX_BACK_OFF_MS))).unwrap_or(MAX_BACK_OFF_MS))
}

/// A `Retry-After` in seconds (a date is not read: the default), as milliseconds, capped.
fn retry_after_ms(value: Option<&str>) -> u32 {
    let text = value.unwrap_or("").trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return DEFAULT_BACK_OFF_MS;
    }
    let seconds = text.parse::<u64>().unwrap_or(u64::MAX);
    u32::try_from(seconds.saturating_mul(1000).min(u64::from(MAX_BACK_OFF_MS))).unwrap_or(MAX_BACK_OFF_MS)
}

/// The error a `Fault` names: the response code of its `detail`, and for a busy server the wait.
fn read_fault(fault: &Node) -> Error {
    let detail = fault.child("detail");
    let code = detail
        .and_then(|detail| detail.child_in(ERRORS, "ResponseCode"))
        .map(|code| ResponseCode::from_name(&code.text))
        .unwrap_or_else(|| ResponseCode::Other(String::new()));
    match code {
        ResponseCode::ServerBusy => {
            let wait = detail
                .and_then(|detail| detail.child_in(ERRORS, "MessageXml"))
                .and_then(|xml| xml.children.iter().find(|value| value.name == "Value" && value.attribute("Name") == Some("BackOffMilliseconds")))
                .map_or(DEFAULT_BACK_OFF_MS, |value| back_off_ms(&value.text));
            Error::Busy { back_off_ms: wait }
        }
        code => Error::Server(code),
    }
}

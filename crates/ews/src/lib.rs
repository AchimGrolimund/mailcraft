// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Exchange Web Services (EWS) without sockets and without browser APIs.
//!
//! The crate describes HTTP requests and judges the answers; the program performs each exchange
//! through an [`HttpExchange`] (an HTTP client where there are sockets, or, in the tests, the
//! in-process, strict server of `standin`, behind the `stand-in` feature). It has no I/O of its
//! own, no thread and no clock, so it builds for wasm32 unchanged.
//!
//! It is written from Microsoft's public documentation only: the EWS reference on Microsoft
//! Learn and the Open Specifications [MS-OXWSCDATA] (types), [MS-OXWSCORE] (the operations and
//! their messages) and [MS-OXWSFOLD] (folders).
//!
//! - [`soap`]: the request: a SOAP 1.1 envelope with `RequestServerVersion` and one operation,
//!   written from [`Element`]s. Every name, attribute and text is stripped of control characters
//!   (CR, LF and NUL among them) and XML-escaped.
//! - [`xml`]: the hostile reader. No DTD, no entity expansion, nesting and size and count capped,
//!   unknown elements skipped, namespaces resolved so that elements are found by namespace and
//!   local name and never by prefix.
//! - [`reply`], [`error`]: the answer: HTTP status, SOAP fault, response messages with their
//!   class and code, and `ErrorServerBusy` with a capped `BackOffMilliseconds` ([`Error::Busy`]).
//!   No error holds text a server wrote.
//! - [`folder`], [`client`]: `GetFolder` by distinguished name, and [`Client::check`], which
//!   tests an account with it.
//!
//! Everything a server sends is hostile: nothing indexes into what came over the wire, and a
//! server's ids stay behind types whose `Debug` shows nothing ([`FolderId`], [`Secret`]).
#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod client;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod error;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod folder;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod http;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod reply;
#[deny(missing_docs)]
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod soap;
#[cfg_attr(not(test), deny(clippy::indexing_slicing, clippy::arithmetic_side_effects))]
pub mod xml;

#[cfg(any(test, feature = "stand-in"))]
pub mod standin;

#[cfg(test)]
mod hostile_tests;
#[cfg(test)]
mod tests;

pub use client::Client;
pub use error::{Error, PolicyReason, ResponseClass, ResponseCode};
pub use folder::{DistinguishedFolder, FolderId, FolderInfo, GetFolder};
pub use http::{EndpointError, HttpExchange, HttpRequest, HttpResponse, Method, Secret, TransportError};
pub use reply::{Message, Reply};
pub use soap::{Element, Version};

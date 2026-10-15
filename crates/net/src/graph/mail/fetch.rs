//! Messages of a Microsoft account fetched whole (R3.2; R3 spec S12, S14): `GET
//! /me/messages/{id}/$value`, each message's MIME as the server has it, attachments and all.
//!
//! A host asks for a few at a time: in the background, newest first, with the share of requests
//! background work has (at most 2 of the mailbox's 4); at once, ahead of that, for the message
//! the owner opened or is replying to. Cancel is looked at before every message. What comes back
//! is the message's own bytes, which the engine reads as any message is read; nothing of it goes
//! into a log line or an error.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use mailcraft_model::sync::{GraphFetchRequest, GraphFetchResult, GraphFetched};

use super::super::pace::{Gate, Kind, Priority, Retry};
use super::super::{Doing, Graph, graph_base};
use super::id_of;
use crate::NetError;
use crate::http;

/// Largest message taken (Microsoft's own limit is 150 MB).
const MAX_MIME: usize = 160 * 1024 * 1024;
/// How long one message may take to come.
const FETCH_TIMEOUT: Duration = Duration::from_secs(300);

/// Fetch the messages of `request` whole, as the owner of `token`.
pub fn fetch_messages(token: &str, request: &GraphFetchRequest, cancel: &AtomicBool) -> GraphFetchResult {
    fetch_messages_with(&graph_base(), token, request, Gate::for_mailbox(&request.email), Retry::STANDARD, cancel)
}

/// [`fetch_messages`] at `base`, through `gate`, asking again as `retry` says.
pub(crate) fn fetch_messages_with(
    base: &str,
    token: &str,
    request: &GraphFetchRequest,
    gate: Arc<Gate>,
    retry: Retry,
    cancel: &AtomicBool,
) -> GraphFetchResult {
    let mut out = GraphFetchResult { account: request.account, email: request.email.clone(), ..Default::default() };
    let priority = if request.background { Priority::Background } else { Priority::Owner };
    let graph = match Graph::for_mail(base, token, gate, priority, retry) {
        Ok(graph) => graph,
        Err(e) => {
            out.errors.push(format!("Couldn't reach Microsoft for the messages: {e}"));
            return out;
        }
    };
    for id in request.ids.iter().take(100) {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Some(id) = id_of(id) else { continue };
        let url = graph.url(&format!("/v1.0/me/messages/{}/$value", http::encode(&id)));
        let reply = match graph.call(Kind::Read, &url, "GET", &[], None, FETCH_TIMEOUT, MAX_MIME) {
            Ok(reply) => reply,
            Err(e) => {
                stop(&mut out, &e);
                break;
            }
        };
        match reply.status {
            200 => out.fetched.push(GraphFetched { id, mime: reply.body }),
            404 => out.missing.push(id),
            _ => {
                let e = graph.refusal(Doing::Mail, &reply, "");
                stop(&mut out, &e);
                break;
            }
        }
    }
    log::info!("graph mail: {} of {} messages fetched whole, {} no longer there", out.fetched.len(), request.ids.len(), out.missing.len());
    out
}

fn stop(out: &mut GraphFetchResult, e: &NetError) {
    if let NetError::Busy { retry_after, .. } = e {
        out.busy_for = Some(retry_after.unwrap_or(0));
    }
    out.offline = e.is_unreachable();
    out.errors.push(format!("Couldn't download the messages from Microsoft: {e}"));
}

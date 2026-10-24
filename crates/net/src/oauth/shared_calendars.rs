// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading other people's calendars (#70, Open Shared Calendar) takes one more permission,
//! `Calendars.ReadWrite.Shared`, asked for in the way the shared mailboxes' are: by itself, when
//! the owner has opened a calendar by address and it is first read, and never on the page of a
//! sign-in ([`sign_in_for_shared_calendars`], [`refresh_graph_shared_calendars`]). An account
//! with no calendar opened by address is never asked for it.
//!
//! (The permission is among those an administrator approves in advance: [`super::admin_scope`].)

use std::sync::atomic::AtomicBool;

use mailcraft_model::OAuth;

use super::{
    Endpoints, FavoritesRefusal, GraphTokens, MICROSOFT_GRAPH, MICROSOFT_GRAPH_SHARED_LATER, Resource, SIGN_IN_SCOPE, SIGN_IN_TIMEOUT, Tokens,
    refresh_for_at, sign_in_for_favorites_within,
};
use crate::NetError;

/// What the tokens of a calendar job for an account with calendars opened by address are asked
/// for: what its calendar takes, and reading and changing other people's calendars on top.
pub(super) fn shared_calendars_scope() -> String {
    format!("{MICROSOFT_GRAPH} {MICROSOFT_GRAPH_SHARED_LATER} {SIGN_IN_SCOPE}")
}

/// Ask the owner, on the provider's page, to let this program read the calendars people share
/// with them and open by address (`Calendars.ReadWrite.Shared`), beside what Microsoft Graph was
/// asked for before. Only when a calendar was opened by address, and never with an ordinary
/// sign-in. A no says whose it is, in the same kinds as the Favorites' ([`FavoritesRefusal`]).
/// Nothing is tried a second time.
pub fn sign_in_for_shared_calendars(oauth: &OAuth, login_hint: &str, open: &dyn Fn(&str), cancel: &AtomicBool) -> Result<Tokens, FavoritesRefusal> {
    let plain = Endpoints::for_resource(oauth, Resource::Graph);
    let endpoints = Endpoints { scope: shared_calendars_scope(), consent: String::new(), lesser: String::new(), ..plain };
    sign_in_for_favorites_within(&endpoints, oauth, login_hint, open, cancel, SIGN_IN_TIMEOUT)
}

/// A Graph token for a calendar job of an account with calendars opened by address: asked for
/// with `Calendars.ReadWrite.Shared` too, and for nothing less. A sign-in that doesn't cover it
/// gets [`NetError::ConsentNeeded`], and the job goes on with the account's own calendars while
/// the owner is asked on the provider's page ([`sign_in_for_shared_calendars`]).
pub fn refresh_graph_shared_calendars(oauth: &OAuth, refresh_token: &str) -> Result<GraphTokens, NetError> {
    refresh_graph_shared_calendars_at(&Endpoints::for_resource(oauth, Resource::Graph), oauth, refresh_token)
}

/// [`refresh_graph_shared_calendars`] at endpoints other than the provider's usual ones (their
/// scope is replaced by what other people's calendars take).
pub fn refresh_graph_shared_calendars_at(endpoints: &Endpoints, oauth: &OAuth, refresh_token: &str) -> Result<GraphTokens, NetError> {
    let endpoints = Endpoints { scope: shared_calendars_scope(), ..endpoints.clone() };
    let tokens = refresh_for_at(&endpoints, oauth, refresh_token, Resource::Graph)?;
    Ok(GraphTokens { tokens, extras_missing: true, covers_favorites: false })
}

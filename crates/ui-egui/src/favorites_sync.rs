//! Keeping the Favorites of the folder pane the ones Outlook shows, for an account that signs
//! in with Microsoft: read from its server, never written there.
//!
//! The read rides along with the jobs the host does for Microsoft Graph anyway (the calendar's
//! own job, [`crate::calendar_sync`], and the Graph part of a Send/Receive): when the app
//! starts, when the owner asks for a Send/Receive, right after the owner connected them, and
//! otherwise not more often than every quarter of an hour. What comes back is taken in by the
//! engine (`sync.applyFavorites`), and the pane lists what the engine says
//! (`mailcraft_engine::cmd::folder::favorites`).
//!
//! Reading them takes two permissions that a sign-in doesn't have until the owner agrees to
//! them on the provider's page, by itself and when the owner chooses to: a quiet line under the
//! Favorites heading offers it (and so does File › Options). Until then, and whenever the
//! server's list can't be had, the favourites kept in the mailbox are shown, untouched. Nothing
//! here ever puts a dialog in front of the owner:
//!
//! - the sign-in doesn't cover them: the offer, and the server isn't asked again that day;
//! - the owner asked and it wasn't to be had (the organisation doesn't let its people give the
//!   permission themselves, or this kind of account has none such): one plain line saying so,
//!   and nothing is asked again that day unless the owner asks;
//! - a read came to nothing: one line with why in a few words, and "Try again".

use std::collections::{HashMap, HashSet};

use mailcraft_engine::cmd::remote;
use mailcraft_model::Id;
use mailcraft_model::remote::{FavoritesDownload, FavoritesOutcome, RemoteFavorite};
use serde_json::{Value, json};

use crate::{GraphResult, MailApp, now_ms};

/// How often an account's Favorites are read at most (milliseconds), and for how long the
/// server is left alone after it said the sign-in doesn't cover them.
const EVERY_MS: f64 = 900_000.0;
const DAY_MS: f64 = 86_400_000.0;

/// What a sign-in for the Favorites says when the organisation the account belongs to doesn't
/// let its people give the permission themselves, and when this kind of account has no such
/// permission: the host answers a [`crate::SignInRequest`] that has `favorites` with exactly
/// one of these as its error, and the pane shows it (in the interface language).
pub const REFUSED_BY_ORGANISATION: &str = "Not allowed by your organisation";
pub const REFUSED_FOR_ACCOUNT: &str = "Not available for this account";

/// What the pane says under the Favorites heading, when there is something to say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Note {
    /// The sign-in doesn't cover reading Outlook's Favorites: the offer to show them.
    Offer,
    /// The owner asked, and they are not to be had: why, in a line.
    Refused(String),
    /// The last read came to nothing: why, in a few plain words.
    Failed(String),
}

impl Note {
    /// What the control channel calls it.
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Note::Offer => "offer",
            Note::Refused(_) => "refused",
            Note::Failed(_) => "failed",
        }
    }
}

/// What the app remembers of keeping the Favorites current. All of it is for the run: nothing
/// here is saved.
#[derive(Default)]
pub(crate) struct FavoritesSync {
    /// When each account's Favorites were last asked for (milliseconds).
    asked: HashMap<Id, f64>,
    /// The accounts whose sign-in was found not to cover reading them, and when (milliseconds).
    missing: HashMap<Id, f64>,
    /// What there is to say under the heading, account by account.
    notes: HashMap<Id, Note>,
    /// The accounts whose Graph token in memory is known not to be good for reading them (it
    /// was asked for without those permissions): a job that wants them gets a new one.
    uncovered: HashSet<Id>,
    /// The accounts whose Favorites are read at the next chance, whatever the clock says: the
    /// owner asked.
    due: HashSet<Id>,
}

impl MailApp {
    /// Whether `account` signs in with Microsoft.
    fn is_microsoft(&self, account: Id) -> bool {
        let microsoft = |a: &mailcraft_model::Account| a.oauth.as_ref().is_some_and(|o| o.provider == mailcraft_model::OAuthProvider::Microsoft);
        self.session.mailbox.account(account).is_some_and(microsoft)
    }

    /// Whether a Graph job for `account` that starts at `now` reads the Favorites (and so asks
    /// for a token that is good for reading them, when it has to ask for one: a job that
    /// doesn't read them asks for nothing it didn't always ask for). `asked_for` when the owner
    /// asked for the job (a Send/Receive). Notes that they were asked for when they are.
    pub(crate) fn favorites_ask(&mut self, account: Id, now: f64, asked_for: bool) -> bool {
        if !self.is_microsoft(account) || self.session.prefs.own_favorites {
            return false;
        }
        let owner = self.favorites.due.contains(&account);
        // Told that the sign-in doesn't cover them: not again that day, unless the owner asks.
        let refused = self.favorites.missing.get(&account).is_some_and(|at| (0.0..DAY_MS).contains(&(now - at)));
        let due = owner || asked_for || self.favorites.asked.get(&account).is_none_or(|at| now - at >= EVERY_MS || now < *at);
        let read = due && (owner || !refused);
        if read {
            self.favorites.asked.insert(account, now);
            self.favorites.due.remove(&account);
        }
        read
    }

    /// The Graph token to hand a job for `account`: the one in memory when it is still good,
    /// unless the job reads the Favorites and this one is known not to be good for that. A job
    /// that will have to ask for a token, and not for one that is good for the Favorites, leaves
    /// the account with one that isn't: that is noted here.
    pub(crate) fn graph_access_for(&mut self, account: Id, favorites: bool) -> String {
        let access = self.graph_access(account, favorites);
        if access.is_empty() && !favorites {
            self.favorites.uncovered.insert(account);
        }
        access
    }

    /// [`graph_access_for`](Self::graph_access_for), without noting anything.
    pub(crate) fn graph_access(&self, account: Id, favorites: bool) -> String {
        if favorites && self.favorites.uncovered.contains(&account) {
            return String::new();
        }
        self.graph_tokens.get(&account).filter(|t| t.fresh(mailcraft_time::now())).map(|t| t.access.clone()).unwrap_or_default()
    }

    /// A Graph token for `account` came from somewhere that didn't ask for the Favorites'
    /// permissions with it (a check of the calendar connection, "Connect calendar and contacts").
    pub(crate) fn favorites_token_is_plain(&mut self, account: Id) {
        self.favorites.uncovered.insert(account);
    }

    /// Take in what a Graph job said of the Favorites of `account`: whether its token covers
    /// them, and the list, or why there is none. Never a dialog, never the problems report:
    /// what there is to say is said under the Favorites heading.
    pub(crate) fn take_in_favorites(&mut self, account: Id, result: &mut GraphResult) {
        match result.favorites_covered {
            Some(true) => {
                self.favorites.uncovered.remove(&account);
            }
            Some(false) => {
                self.favorites.uncovered.insert(account);
            }
            None => {}
        }
        let Some(download) = result.favorites.take() else { return };
        let why = std::mem::take(&mut result.favorites_error);
        self.apply_favorites_download(account, &download, &why);
    }

    /// [`take_in_favorites`](Self::take_in_favorites) for one download; `why` is what went
    /// wrong, in a few plain words, when the read failed.
    fn apply_favorites_download(&mut self, account: Id, download: &FavoritesDownload, why: &str) {
        // (By the clock of the look that asked, which is the one the next look goes by.)
        let now = self.favorites.asked.get(&account).copied().unwrap_or_else(now_ms);
        match download.outcome {
            FavoritesOutcome::Read | FavoritesOutcome::Missing => {
                self.favorites.missing.remove(&account);
                self.favorites.notes.remove(&account);
            }
            FavoritesOutcome::NotPermitted => {
                self.favorites.missing.insert(account, now);
                self.favorites.uncovered.insert(account);
                // (What the owner was told when he asked stays said.)
                if !matches!(self.favorites.notes.get(&account), Some(Note::Refused(_))) {
                    self.favorites.notes.insert(account, Note::Offer);
                }
            }
            FavoritesOutcome::Failed => {
                let why = if why.trim().is_empty() { "Couldn't read the list" } else { why.trim() };
                self.favorites.notes.insert(account, Note::Failed(why.chars().filter(|c| !c.is_control()).take(60).collect()));
            }
        }
        match remote::apply_favorites(&mut self.session, download) {
            // Counts, never a name: what the log may hold.
            Ok(answer) => log::info!(
                "favorites, account {account}: {:?}; {} folders listed here, {} of the server's are not folders here, {} entries were no folders, {} named none",
                download.outcome,
                answer["favorites"].as_array().map_or(0, Vec::len),
                answer["absent"].as_u64().unwrap_or(0),
                download.skipped,
                download.unresolved
            ),
            // (For another account than the mailbox has now: nobody's.)
            Err(_) => log::debug!("favorites, account {account}: the answer was not taken in"),
        }
    }

    /// What the pane says under the Favorites heading: the first account that has something to
    /// say, and what. Nothing while the owner would rather see the mailbox's own favourites.
    pub(crate) fn favorites_note(&self) -> Option<(Id, Note)> {
        if self.session.prefs.own_favorites {
            return None;
        }
        let accounts = self.session.mailbox.accounts.iter().map(|a| a.id);
        accounts.filter(|id| self.is_microsoft(*id)).find_map(|id| self.favorites.notes.get(&id).map(|note| (id, note.clone())))
    }

    /// The account whose Favorites could be Outlook's if the owner connected them: the first
    /// that has the offer standing (File › Options offers it too).
    pub(crate) fn favorites_offer(&self) -> Option<Id> {
        let accounts = self.session.mailbox.accounts.iter().map(|a| a.id);
        accounts.filter(|id| self.is_microsoft(*id)).find(|id| matches!(self.favorites.notes.get(id), Some(Note::Offer | Note::Refused(_))))
    }

    /// The Favorites of `account` are read at the next chance, whatever the clock says and
    /// whatever was said before: the owner asked. The calendar's own job carries the read, so
    /// it is made due too.
    pub(crate) fn favorites_due(&mut self, account: Id) {
        if !self.is_microsoft(account) {
            return;
        }
        self.favorites.due.insert(account);
        self.favorites.missing.remove(&account);
        self.calendar.checked.remove(&account);
        self.calendar.failures.remove(&account);
    }

    /// What was remembered of an account that is gone.
    pub(crate) fn forget_favorites(&mut self, account: Id) {
        let kept = &mut self.favorites;
        kept.asked.remove(&account);
        kept.missing.remove(&account);
        kept.notes.remove(&account);
        kept.uncovered.remove(&account);
        kept.due.remove(&account);
    }

    /// The sign-in for the Favorites of `account` came back with tokens that are good for
    /// them: nothing stands in the way any more, and they are read at once.
    pub(crate) fn favorites_connected(&mut self, account: Id) {
        self.favorites.notes.remove(&account);
        self.favorites.uncovered.remove(&account);
        self.favorites_due(account);
    }

    /// The sign-in for the Favorites of `account` came to nothing: `error` is why. When the no
    /// is not the owner's to change ([`REFUSED_BY_ORGANISATION`], [`REFUSED_FOR_ACCOUNT`]) the
    /// pane says so in a line, and nothing is asked again that day; anything else (the owner
    /// said no, the page was closed) goes to the status bar, and the offer stands.
    pub(crate) fn favorites_refused(&mut self, account: Id, error: &str) {
        if error == REFUSED_BY_ORGANISATION || error == REFUSED_FOR_ACCOUNT {
            self.favorites.notes.insert(account, Note::Refused(error.to_string()));
            self.favorites.missing.insert(account, now_ms());
            self.status(tl!(error));
        } else {
            self.status(error.to_string());
        }
    }

    /// The Microsoft account a favourites command is for: the one named, else the first.
    fn favorites_account(&self, p: &Value) -> Result<Id, String> {
        let named = p.get("account").and_then(Value::as_u64);
        let first = || self.session.mailbox.accounts.iter().map(|a| a.id).find(|id| self.is_microsoft(*id));
        match named.or_else(first) {
            Some(id) if self.is_microsoft(id) => Ok(id),
            Some(_) => Err(tl!("This account doesn't sign in with Microsoft").to_string()),
            None => Err(tl!("No account signs in with Microsoft").to_string()),
        }
    }

    /// What the control channel is told of the Favorites of `account`.
    fn favorites_state(&self, account: Id) -> Value {
        let from_server = mailcraft_engine::cmd::folder::favorites_from_server(&self.session, account);
        let note = self.favorites.notes.get(&account).filter(|_| !self.session.prefs.own_favorites);
        let text = match note {
            Some(Note::Offer) => tl!("Show Outlook's Favorites").to_string(),
            Some(Note::Refused(why) | Note::Failed(why)) => tl!(why).to_string(),
            None => String::new(),
        };
        json!({
            "account": account,
            "from": if from_server { "server" } else { "mailbox" },
            "own": self.session.prefs.own_favorites,
            "line": note.map(Note::code),
            "text": text,
            "favorites": mailcraft_engine::cmd::folder::favorites(&self.session),
        })
    }

    /// `ui.connectFavorites`: have the owner agree, on Microsoft's page, to this program reading
    /// the Favorites of Outlook. Nothing else is asked for there that wasn't agreed to before.
    fn connect_favorites(&mut self, p: &Value) -> Result<Value, String> {
        let account = self.favorites_account(p)?;
        let Some(owner) = self.session.mailbox.account(account).cloned() else { return Err(tl!("No account signs in with Microsoft").to_string()) };
        let Some(oauth) = owner.oauth.clone() else { return Err(tl!("This account doesn't sign in with Microsoft").to_string()) };
        self.favorites_next = true;
        self.begin_sign_in(account, oauth, &owner.email, String::new(), false, true)?;
        Ok(json!({"pending": "signIn", "account": account}))
    }

    /// `ui.favorites`: what the folder pane has of an account's Favorites in Outlook. With
    /// `"refresh": true` they are read again at the next chance ("Try again"). With `answer`,
    /// nothing is asked of anybody: it is taken in as what the host brought (for a script, and
    /// for looking at the pane in each of its states).
    fn favorites_command(&mut self, p: &Value) -> Result<Value, String> {
        let account = self.favorites_account(p)?;
        if let Some(answer) = p.get("answer") {
            let outcome = answer.get("outcome").and_then(Value::as_str).unwrap_or("");
            let why = answer.get("why").and_then(Value::as_str).unwrap_or("");
            // (The owner asked on the provider's page and was told no.)
            if outcome == "refused" {
                let by = if why == REFUSED_FOR_ACCOUNT { REFUSED_FOR_ACCOUNT } else { REFUSED_BY_ORGANISATION };
                self.favorites_refused(account, by);
                return Ok(self.favorites_state(account));
            }
            let outcome = match outcome {
                "read" => FavoritesOutcome::Read,
                "missing" => FavoritesOutcome::Missing,
                "notPermitted" => FavoritesOutcome::NotPermitted,
                "failed" => FavoritesOutcome::Failed,
                _ => return Err(tl!("`answer.outcome` is read, missing, notPermitted, refused or failed").to_string()),
            };
            let folders: Vec<RemoteFavorite> = match answer.get("folders") {
                None | Some(Value::Null) => Vec::new(),
                Some(folders) => {
                    serde_json::from_value(folders.clone()).map_err(|_| tl!("`answer.folders` is a list of {path, kind}").to_string())?
                }
            };
            let Some(owner) = self.session.mailbox.account(account).cloned() else {
                return Err(tl!("No account signs in with Microsoft").to_string());
            };
            let download = FavoritesDownload { outcome, folders, ..Default::default() }.for_account(&owner);
            self.apply_favorites_download(account, &download, why);
            return Ok(self.favorites_state(account));
        }
        if p.get("refresh").and_then(Value::as_bool) != Some(true) {
            return Ok(self.favorites_state(account));
        }
        if self.session.prefs.own_favorites {
            return Err(tl!("Outlook's Favorites are switched off in File › Options").to_string());
        }
        if self.services.graph.is_none() {
            return Err(tl!("This build can't reach the account's server.").to_string());
        }
        // What the last read said is not said any longer: the next one says what there is.
        if matches!(self.favorites.notes.get(&account), Some(Note::Failed(_))) {
            self.favorites.notes.remove(&account);
        }
        self.favorites_due(account);
        Ok(self.favorites_state(account))
    }
}

/// File › Options: whether the folder pane shows Outlook's Favorites for an account that has
/// them, and the way to connect them for one whose sign-in doesn't cover reading them yet.
/// Nothing is shown while no account signs in with Microsoft.
pub(crate) fn options(app: &mut MailApp, ui: &mut egui::Ui, t: &crate::theme::Tokens) {
    let ids: Vec<Id> = app.session.mailbox.accounts.iter().map(|a| a.id).collect();
    if !ids.iter().any(|id| app.is_microsoft(*id)) {
        return;
    }
    let mut outlook = !app.session.prefs.own_favorites;
    let about = tl!(
        "For an account that signs in with Microsoft: the folders you keep under Favorites in Outlook, in the same order. They are read from the server and never changed there. Without this, the favourites you mark in MailCraft are shown."
    );
    let resp = ui.checkbox(&mut outlook, tl!("Show the Favorites from Outlook in the folder pane")).on_hover_text(about);
    app.hit.add("favoritesOption", 0, resp.rect);
    if resp.changed() {
        let _ = app.run("view.favorites", json!({"own": !outlook}));
    }
    // The sign-in doesn't cover reading them yet: the same offer as under the Favorites heading.
    if let Some(account) = app.favorites_offer().filter(|_| outlook) {
        let said = match app.favorites.notes.get(&account) {
            Some(Note::Refused(why)) => tl!(why).to_string(),
            _ => String::new(),
        };
        ui.horizontal(|ui| {
            ui.add_space(24.0);
            let link = ui.link(tl!("Connect Outlook's Favorites")).on_hover_text(tl!(
                "Sign in with Microsoft once more to let MailCraft read the folders you keep under Favorites in Outlook. They are only read, never changed, and everything else stays as it is."
            ));
            app.hit.add("favoritesConnect", account, link.rect);
            if link.clicked()
                && let Err(e) = app.run("ui.connectFavorites", json!({"account": account}))
            {
                app.status(e);
            }
            if !said.is_empty() {
                ui.label(egui::RichText::new(said).color(t.text_dim));
            }
        });
    }
}

/// The `ui.*` commands of the Favorites; `None` for any other.
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "ui.connectFavorites" => app.connect_favorites(p),
        "ui.favorites" => app.favorites_command(p),
        _ => return None,
    })
}

#[cfg(test)]
mod tests;

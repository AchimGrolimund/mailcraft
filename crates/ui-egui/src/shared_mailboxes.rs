//! Shared mailboxes in the window (#70, a draft): read through Microsoft Graph with the sign-in
//! of the account they are opened through, at their own address.
//!
//! A shared mailbox's Send/Receive is its folders and mail alone: no Outbox (mail written there
//! goes from the owner's own account), no calendar, no contacts, no Favorites. Its token is
//! renewed for the run with `Mail.ReadWrite.Shared`; when the owner's sign-in doesn't cover that
//! yet, he is asked for it on Microsoft's page, by itself, when he asked for the run (adding a
//! shared mailbox does), and never with a sign-in of any other kind. The refresh token that
//! comes back is the owner's, kept as his; the access token is the shared mailbox's, in memory,
//! for fetching its messages whole. A server that won't open the mailbox for the owner says so,
//! and the mailbox is not read again by a run that started by itself until the owner asks.

use mailcraft_model::Id;
use serde_json::{Value, json};

use crate::{GraphRequest, GraphResult, MailApp, Tokens};

impl MailApp {
    /// A Send/Receive of the shared mailbox `account`: its folders and mail, with its owner's
    /// sign-in.
    pub(crate) fn start_shared_mail(&mut self, account: Id, quiet: bool) -> Result<Value, String> {
        let mailbox = self.session.mailbox.account(account).cloned().ok_or_else(|| tl!("The account is gone").to_string())?;
        let owner = self
            .session
            .mailbox
            .owner_of(account)
            .cloned()
            .ok_or_else(|| tl!("This shared mailbox's account is gone: remove the shared mailbox and add it again").to_string())?;
        if quiet && self.shared_denied.contains(&account) {
            return Err(tl!("Microsoft won't open this shared mailbox for you: Send/Receive tries again").to_string());
        }
        let oauth = owner.oauth.clone().ok_or_else(|| tl!("This account has no sign-in with Microsoft").to_string())?;
        let address = owner.email.clone();
        let kept = self.tokens.get(&owner.id).or(self.graph_tokens.get(&owner.id)).map(|t| t.refresh.clone()).filter(|r| !r.is_empty());
        let refresh = kept.or_else(|| self.remembered("refresh", &address)).unwrap_or_default();
        if refresh.is_empty() {
            if quiet {
                return Err(format!("{} {address}", tl!("Use Send/Receive to sign in to")));
            }
            // The owner's own sign-in first; the shared mailbox is read after it, when asked.
            self.begin_sign_in(owner.id, oauth, &address, String::new(), false, true)?;
            return Ok(json!({"pending": "signIn", "account": owner.id}));
        }
        let mail = mailcraft_engine::cmd::graph_mail::graph_mail_request(&self.session, account).map_err(|e| e.to_string())?;
        let (pieces, taken) = std::sync::mpsc::sync_channel(8);
        let ask = GraphRequest {
            account: mailbox,
            oauth: self.effective_oauth(oauth),
            refresh,
            // (Renewed for the run, with what reading a shared mailbox takes.)
            access: String::new(),
            outbox: Vec::new(),
            directory: false,
            calendar: None,
            changes: None,
            asks: Vec::new(),
            settings: false,
            favorites: false,
            favorites_scopes: false,
            mail: Some(mail),
            mail_pieces: Some(pieces),
            cancel: self.services.sync_cancel.clone(),
        };
        self.resume_fetch();
        let Some(graph) = &self.services.graph else { return Err(tl!("This build can't connect to Microsoft").to_string()) };
        self.services.sync_cancel.store(false, std::sync::atomic::Ordering::Relaxed);
        self.graph_pieces = Some(taken);
        self.graph_problems.clear();
        // Asked for: the owner wants to know whether the server opens it now.
        if !quiet {
            self.shared_denied.remove(&account);
        }
        self.graph_rx = Some((account, quiet, false, graph(ask)));
        self.session.sync_status = tl!("Sending and receiving…").to_string();
        Ok(json!({"started": true, "account": account, "graph": true, "shared": true}))
    }

    /// A shared mailbox's run came back (its pieces are in already): keep the owner's refresh
    /// token as his and the access token for this mailbox, ask for the permission when it is
    /// missing, and say what went wrong.
    pub(crate) fn finish_shared_mail(&mut self, account: Id, quiet: bool, mut result: GraphResult) {
        let owner = self.session.mailbox.owner_of(account).map(|o| (o.id, o.email.clone()));
        let mut errors = std::mem::take(&mut result.errors);
        errors.append(&mut self.graph_problems);
        if result.token_refused {
            self.graph_tokens.remove(&account);
        }
        if let (Some(tokens), Some((owner, address))) = (result.tokens.take(), owner.as_ref()) {
            if !tokens.refresh.is_empty() {
                self.remember("refresh", address, &tokens.refresh);
                if let Some(kept) = self.tokens.get_mut(owner) {
                    kept.refresh = tokens.refresh.clone();
                }
            }
            let now = mailcraft_time::now();
            self.graph_tokens.insert(account, Tokens { expires_at: tokens.expires_at.clamp(now, now.saturating_add(86_400)), ..tokens });
        }
        if let Some(mail) = result.mail.take() {
            if mail.denied {
                self.shared_denied.insert(account);
            }
            match mailcraft_engine::cmd::graph_mail::apply_graph_mail(&mut self.session, mail) {
                Ok(taken) => {
                    let said = taken["errors"].as_array().map(|all| all.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>());
                    errors.extend(said.unwrap_or_default());
                }
                Err(e) => errors.push(e.to_string()),
            }
        }
        if result.consent_needed {
            self.session.sync_status.clear();
            if quiet {
                errors.push(tl!("Reading shared mailboxes needs your permission: use Send/Receive").to_string());
            } else if let Err(e) = self.begin_shared_sign_in(account) {
                errors.push(e);
            }
        }
        errors.dedup();
        if let Some(first) = errors.first() {
            self.session.sync_status = first.clone();
        } else if !result.consent_needed {
            self.session.sync_status.clear();
        }
        self.note_sync_problems(account, &errors, true);
    }

    /// Ask the owner of the shared mailbox `shared`, on Microsoft's page, to let this program read
    /// shared mailboxes: by itself, because he wants this one read.
    pub(crate) fn begin_shared_sign_in(&mut self, shared: Id) -> Result<(), String> {
        let owner = self.session.mailbox.owner_of(shared).cloned().ok_or_else(|| tl!("This shared mailbox's account is gone").to_string())?;
        let oauth = owner.oauth.clone().ok_or_else(|| tl!("This account has no sign-in with Microsoft").to_string())?;
        self.shared_next = Some(shared);
        let begun = self.begin_sign_in(owner.id, oauth, &owner.email, String::new(), false, true);
        // (A sign-in that couldn't start doesn't leave the question for another one.)
        self.shared_next = None;
        begun?;
        self.session.sync_status = tl!("Allow MailCraft to read shared mailboxes on the page that opened in your browser…").to_string();
        Ok(())
    }

    /// The owner answered on Microsoft's page about shared mailboxes: with the tokens, the shared
    /// mailbox is read now; without, why not is said.
    pub(crate) fn shared_signed_in(&mut self, shared: Id, owner: Id, answer: Result<Tokens, String>) {
        self.session.sync_status.clear();
        let address = self.session.mailbox.account(owner).map(|a| a.email.clone()).unwrap_or_default();
        match answer {
            // The owner picks who signs in on the page: it has to be the account it is opened through.
            Ok(tokens) if !tokens.email.is_empty() && !tokens.email.trim().eq_ignore_ascii_case(address.trim()) => {
                self.status(format!("{} {} {} {address}", tl!("You signed in as"), tokens.email, tl!("but this account is")));
            }
            Ok(tokens) => {
                if !tokens.refresh.is_empty() {
                    self.remember("refresh", &address, &tokens.refresh);
                    if let Some(kept) = self.tokens.get_mut(&owner) {
                        kept.refresh = tokens.refresh.clone();
                    }
                }
                self.graph_tokens.insert(shared, tokens);
                if let Err(e) = self.start_sync(&json!({"account": shared})) {
                    self.status(e);
                }
            }
            Err(e) if e == crate::favorites_sync::NEEDS_APPROVAL => {
                self.status(tl!("Your organisation's administrator has to approve reading shared mailboxes once: File › Info"));
            }
            Err(e) => self.status(format!("{}: {e}", tl!("Shared mailboxes weren't allowed"))),
        }
    }
}

#[cfg(test)]
#[path = "shared_mailboxes_tests.rs"]
mod tests;

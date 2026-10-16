//! The page that offers the switch to Microsoft Graph, and what the window does with the answer
//! (R3.7). Nothing here leaves this machine: the hosts are stand-ins, the mail invented.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use mailcraft_engine::Session;
use mailcraft_model::sync::{SyncRequest, SyncResult, Tokens};
use mailcraft_model::{AccountKind, FolderKind, GraphSwitch, Id, Message};
use serde_json::json;

use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

fn window(app: MailApp) -> Harness<'static, MailApp> {
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn says(h: &Harness<'static, MailApp>, text: &str) -> bool {
    h.query_all_by_label_contains(text).next().is_some()
}

/// What the status bar says now.
fn status(h: &Harness<'static, MailApp>) -> String {
    h.state().status_msg.as_ref().map(|(said, _)| said.clone()).unwrap_or_default()
}

fn click(h: &mut Harness<'static, MailApp>, label: &str) {
    h.get_by_role_and_label(Role::Button, label).click();
    h.run_steps(4);
}

fn page(h: &Harness<'static, MailApp>) -> Option<&'static str> {
    h.state().dialog.as_ref().map(|d| d.name())
}

/// The sample mailbox, kept in a file of the test's own, with a Microsoft account on IMAP that
/// has a token at hand (and no refresh token: nothing of Graph's is asked before the switch).
fn app(name: &str) -> (MailApp, Id, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("mailcraft-r37-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0);
    s.path = Some(dir.join("mailbox.mailcraft"));
    let mut app = MailApp::new(s, Services::default());
    app.autosave = false;
    app.offer_graph_switch = true;
    let added = app.execute(
        "account.add",
        json!({"email": "owner@contoso.example", "auth": "oauth2", "provider": "microsoft", "incoming": {"host": "outlook.office365.com"}, "outgoing": {"host": "smtp.office365.com", "port": 587, "security": "starttls"}}),
    );
    let account = added.unwrap()["id"].as_u64().unwrap();
    let tokens = Tokens { access: "imap-token".into(), expires_at: mailcraft_time::now() + 3600, ..Default::default() };
    app.tokens.insert(account, tokens);
    (app, account, dir)
}

#[test]
fn the_page_is_shown_once_and_later_keeps_the_account_on_imap() {
    let (app, account, dir) = app("later");
    let mut h = window(app);
    assert_eq!(page(&h), Some("graphSwitch"));
    for line in super::wording("owner@contoso.example") {
        assert!(says(&h, line.trim_start_matches(['•', ' '])), "{line}");
    }
    assert!(says(&h, "Kept from owner@contoso.example"));
    click(&mut h, "Later");
    assert_eq!(page(&h), None);
    let a = h.state().session.mailbox.account(account).unwrap();
    assert_eq!((a.kind, a.graph_switch), (AccountKind::Imap, GraphSwitch::Later));
    assert!(status(&h).contains("stays on IMAP for now"), "{}", status(&h));
    // Not again: not in this run, nor in the next one (the answer is kept with the account).
    h.run_steps(4);
    assert_eq!(page(&h), None);
    let session = std::mem::replace(&mut h.state_mut().session, Session::empty());
    let mut again = MailApp::new(session, Services::default());
    again.autosave = false;
    again.offer_graph_switch = true;
    let h = window(again);
    assert_eq!(page(&h), None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_page_closed_without_an_answer_comes_back_the_next_time() {
    let (app, account, dir) = app("closed");
    let mut h = window(app);
    assert_eq!(page(&h), Some("graphSwitch"));
    h.state_mut().dialog = None;
    h.run_steps(4);
    assert_eq!(page(&h), None, "not again in this run");
    assert_eq!(h.state().session.mailbox.account(account).unwrap().graph_switch, GraphSwitch::NotAsked);
    // And from command search, whenever the owner likes.
    h.state_mut().execute("ui.graphSwitch", json!({})).unwrap();
    h.run_steps(2);
    assert_eq!(page(&h), Some("graphSwitch"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn switch_sends_what_waits_first_waits_while_offline_and_then_switches() {
    let (mut app, account, dir) = app("switch");
    // Something waits for the IMAP server: a message in the Outbox.
    let outbox = app.session.mailbox.folder_of_kind(account, FolderKind::Outbox).unwrap();
    let waiting =
        Message { folder: outbox, subject: "Minutes".into(), to: vec![mailcraft_model::Address::new("", "kit@example.org")], ..Default::default() };
    let waiting = app.session.mailbox.add_message(waiting).unwrap();
    // The IMAP host: offline until told otherwise; online, it sends what it was given.
    let offline = Arc::new(AtomicBool::new(true));
    let runs: Arc<Mutex<usize>> = Default::default();
    let (offline2, runs2) = (offline.clone(), runs.clone());
    app.services.sync = Some(Box::new(move |req: SyncRequest| {
        *runs2.lock().unwrap() += 1;
        let (tx, rx) = mpsc::channel();
        let result = if offline2.load(Ordering::Relaxed) {
            SyncResult { account: req.account.id, errors: vec!["Couldn't connect to outlook.office365.com".into()], ..Default::default() }
        } else {
            SyncResult { account: req.account.id, sent: req.outbox.iter().map(|m| m.id).collect(), ..Default::default() }
        };
        tx.send(result).unwrap();
        rx
    }));
    let mut h = window(app);
    assert_eq!(page(&h), Some("graphSwitch"));
    click(&mut h, "Switch");
    assert_eq!(page(&h), None);
    // The Send/Receive went first, and couldn't reach the server: the switch waits.
    h.run_steps(6);
    assert_eq!(*runs.lock().unwrap(), 1);
    let a = h.state().session.mailbox.account(account).unwrap();
    assert_eq!((a.kind, a.graph_switch), (AccountKind::Imap, GraphSwitch::Switching));
    assert!(status(&h).contains("waits until MailCraft can reach the server"), "{}", status(&h));
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no backup yet");
    // Online again: the next Send/Receive sends it, and the switch goes ahead by itself.
    offline.store(false, Ordering::Relaxed);
    h.state_mut().execute("ui.sync", json!({"account": account})).unwrap();
    h.run_steps(8);
    assert!(h.state().session.mailbox.message(waiting).is_none_or(|m| m.folder != outbox), "sent");
    let a = h.state().session.mailbox.account(account).unwrap();
    assert_eq!((a.kind, a.incoming.is_none()), (AccountKind::Microsoft, true));
    let backup = dir.join("mailbox (backup 2026-10-14).mailcraft");
    assert!(backup.exists());
    assert!(status(&h).contains("now uses Microsoft Graph"), "{}", status(&h));
    let _ = std::fs::remove_dir_all(&dir);
}

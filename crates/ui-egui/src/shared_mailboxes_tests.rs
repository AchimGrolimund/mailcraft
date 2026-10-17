//! A shared mailbox in the window (#70): read with its owner's sign-in, its permission asked for
//! by itself when the owner asked for the run, and a mailbox the server won't open left alone by
//! runs that start by themselves. Hosts are fakes; every address is invented.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_model::sync::{GraphMailRequest, GraphMailResult, Tokens};
use mailcraft_model::{FolderKind, Id};
use serde_json::json;

use crate::{GraphRequest, GraphResult, MailApp, Services, SignInRequest};

const OWNER: &str = "noor@example.org";
const SHARED: &str = "support@example.org";

/// Set when anything asked for a mail-protocol run: a shared mailbox is read through Graph alone.
/// Tests run on threads of one process, so each test reads it only through [`no_imap`], which
/// every test here shares: none of them may ever set it.
static IMAP_USED: AtomicBool = AtomicBool::new(false);

fn no_imap() {
    assert!(!IMAP_USED.load(Ordering::Relaxed), "a mail-protocol run was started");
}

type Answers = Arc<Mutex<VecDeque<(GraphResult, Vec<GraphMailResult>)>>>;
/// What each Graph job was asked: the account, the refresh token, the mail request, the Outbox's
/// length and whether the calendar was asked for.
type Jobs = Arc<Mutex<Vec<(Id, String, Option<GraphMailRequest>, usize, bool)>>>;
/// What each sign-in was asked: the account, for Graph, for shared mailboxes, the address.
type SignIns = Arc<Mutex<Vec<(Id, bool, bool, String)>>>;

fn tokens(access: &str, refresh: &str) -> Tokens {
    Tokens { access: access.into(), refresh: refresh.into(), expires_at: mailcraft_time::now() + 3_600, ..Default::default() }
}

/// A window with a Microsoft account signed in this session, and hosts that note what they are
/// asked and answer what `answers` holds, in turn.
fn window() -> (MailApp, Id, Answers, Jobs, SignIns) {
    let session = Session::empty();
    let mut app = MailApp::new(session, Services::default());
    app.autosave = false;
    let owner = app.execute("account.add", json!({"email": OWNER, "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
    app.tokens.insert(owner, tokens("own-access", "ref-1"));
    let (answers, jobs, sign_ins): (Answers, Jobs, SignIns) = Default::default();
    let (to_answer, log) = (answers.clone(), jobs.clone());
    app.services.graph = Some(Box::new(move |req: GraphRequest| {
        log.lock().unwrap().push((req.account.id, req.refresh.clone(), req.mail.clone(), req.outbox.len(), req.directory));
        let (tx, rx) = mpsc::channel();
        let (answer, pieces) = to_answer.lock().unwrap().pop_front().unwrap_or_default();
        if let Some(to) = &req.mail_pieces {
            for piece in pieces {
                to.send(piece).unwrap();
            }
        }
        tx.send(answer).unwrap();
        rx
    }));
    // The desktop host can open mail connections; a shared mailbox never uses them (it has no
    // IMAP leg), which [`no_imap`] checks at the end of every test.
    app.services.sync = Some(Box::new(|req: mailcraft_model::sync::SyncRequest| {
        IMAP_USED.store(true, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel();
        tx.send(mailcraft_model::sync::SyncResult { account: req.account.id, ..Default::default() }).unwrap();
        rx
    }));
    let log = sign_ins.clone();
    app.services.sign_in = Some(Box::new(move |req: SignInRequest| {
        log.lock().unwrap().push((req.account, req.graph, req.shared, req.email.clone()));
        let (tx, rx) = mpsc::channel();
        tx.send(Ok(Tokens { email: OWNER.into(), ..tokens("shared-access", "ref-2") })).unwrap();
        rx
    }));
    (app, owner, answers, jobs, sign_ins)
}

#[test]
fn a_shared_mailbox_is_read_with_its_owners_sign_in_and_its_permission_is_asked_for_once() {
    let (mut app, owner, answers, jobs, sign_ins) = window();
    let shared = app.execute("account.addSharedMailbox", json!({"address": SHARED})).unwrap()["id"].as_u64().unwrap();
    // Its first run: the owner's sign-in doesn't cover shared mailboxes yet.
    answers.lock().unwrap().push_back((GraphResult { account: shared, consent_needed: true, ..Default::default() }, Vec::new()));
    let listing = mailcraft_model::sync::FolderListing {
        folders: vec![mailcraft_model::sync::GraphFolder { id: "in".into(), name: "Inbox".into(), kind: FolderKind::Inbox, ..Default::default() }],
        complete: true,
    };
    let piece = GraphMailResult { account: shared, email: SHARED.into(), listing: Some(listing), ..Default::default() };
    answers
        .lock()
        .unwrap()
        .push_back((GraphResult { account: shared, tokens: Some(tokens("shared-access-2", "ref-3")), ..Default::default() }, vec![piece]));

    assert_eq!(app.execute("ui.sync", json!({"account": shared})).unwrap()["shared"], true);
    {
        let jobs = jobs.lock().unwrap();
        let (account, refresh, mail, outbox, directory) = jobs[0].clone();
        assert_eq!((account, refresh.as_str(), outbox, directory), (shared, "ref-1", 0, false), "the owner's refresh token, mail alone");
        let mail = mail.unwrap();
        assert_eq!((mail.account, mail.email.as_str(), mail.shared), (shared, SHARED, true));
    }
    app.poll_graph();
    // Asked for on Microsoft's page, by itself, for the owner.
    assert_eq!(*sign_ins.lock().unwrap(), [(owner, true, true, OWNER.to_string())]);
    app.poll_sign_in();
    // Allowed: read at once, and its folders come in under it.
    assert_eq!(jobs.lock().unwrap().len(), 2);
    app.poll_graph();
    assert!(app.session.mailbox.folder_by_server(shared, "in").is_some());
    assert!(app.session.mailbox.folder_by_server(owner, "in").is_none());
    // Its access token is its own, in memory, for fetching; the owner's is as it was.
    assert_eq!(app.graph_tokens.get(&shared).map(|t| t.access.as_str()), Some("shared-access-2"));
    assert_eq!(app.tokens.get(&owner).map(|t| (t.access.as_str(), t.refresh.as_str())), Some(("own-access", "ref-3")));
    assert_eq!(sign_ins.lock().unwrap().len(), 1, "asked once");
    no_imap();
}

#[test]
fn a_shared_mailbox_the_server_wont_open_waits_for_the_owner_to_ask() {
    let (mut app, _, answers, jobs, sign_ins) = window();
    let shared = app.execute("account.addSharedMailbox", json!({"address": SHARED})).unwrap()["id"].as_u64().unwrap();
    let refused = GraphMailResult {
        account: shared,
        email: SHARED.into(),
        denied: true,
        errors: vec!["Microsoft won't open this shared mailbox for you: you need Full Access to it.".into()],
        ..Default::default()
    };
    answers.lock().unwrap().push_back((GraphResult { account: shared, ..Default::default() }, vec![refused]));
    app.execute("ui.sync", json!({"account": shared})).unwrap();
    app.poll_graph();
    assert!(app.session.sync_status.contains("Full Access"), "{}", app.session.sync_status);
    assert!(app.shared_denied.contains(&shared) && !app.can_sync_quietly(shared));
    // A run that starts by itself doesn't ask again; the owner's own does.
    assert!(app.execute("ui.sync", json!({"account": shared, "quiet": true})).is_err());
    assert_eq!(jobs.lock().unwrap().len(), 1);
    app.execute("ui.sync", json!({"account": shared})).unwrap();
    app.poll_graph();
    assert_eq!(jobs.lock().unwrap().len(), 2);
    assert!(!app.shared_denied.contains(&shared), "it opened this time");
    assert!(sign_ins.lock().unwrap().is_empty(), "nobody was sent to a sign-in page");
    no_imap();
}

#[test]
fn the_add_shared_mailbox_dialog_collects_the_address_and_runs_the_command() {
    let (mut app, owner, _, jobs, _) = window();
    // Nothing to open it under: a local account can't have one.
    let local = app.execute("account.add", json!({"email": "ada@example.org"})).unwrap()["id"].as_u64().unwrap();
    assert!(app.execute("ui.dialog", json!({"name": "sharedMailbox", "account": local})).is_err());
    app.execute("ui.dialog", json!({"name": "sharedMailbox", "account": owner})).unwrap();
    assert!(matches!(app.dialog, Some(crate::dialogs::Dialog::SharedMailbox { account, .. }) if account == owner));
    // Laid out in both themes, as File › Info is with a shared mailbox under its owner.
    app.execute("account.addSharedMailbox", json!({"account": owner, "address": "sales@example.org"})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    for theme in [crate::theme::Appearance::Light, crate::theme::Appearance::Dark] {
        h.state_mut().ui.theme = theme;
        h.run_steps(3);
        h.state_mut().execute("ui.backstage", json!({"value": true, "page": "info"})).unwrap();
        h.run_steps(3);
        h.state_mut().execute("ui.backstage", json!({"value": false})).unwrap();
    }
    // An address that is no address is said in the dialog; a good one adds the mailbox and reads it.
    if let Some(crate::dialogs::Dialog::SharedMailbox { address, .. }) = h.state_mut().dialog.as_mut() {
        *address = "not an address".into();
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(matches!(&h.state().dialog, Some(crate::dialogs::Dialog::SharedMailbox { error, .. }) if error.contains("e-mail address")));
    if let Some(crate::dialogs::Dialog::SharedMailbox { address, .. }) = h.state_mut().dialog.as_mut() {
        *address = SHARED.into();
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert!(h.state().dialog.is_none());
    let added = h.state().session.mailbox.accounts.iter().find(|a| a.email == SHARED).map(|a| (a.id, a.shared_of));
    let (id, of) = added.unwrap();
    assert_eq!(of, Some(owner));
    assert!(jobs.lock().unwrap().iter().any(|(account, ..)| *account == id), "read at once");
    no_imap();
}

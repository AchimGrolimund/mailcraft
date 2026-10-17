//! Shared mailboxes through Microsoft Graph (#70): opened under a Microsoft account, read with
//! its sign-in at their own address, and kept apart from it. (Against the Graph stand-in:
//! `crates/net/src/graph/mail/shared_tests.rs`.)

use mailcraft_model::sync::{FolderListing, GraphFolder, GraphMailResult, GraphMessage, MessagePage, ServerMessage};
use mailcraft_model::{AccountKind, FolderKind, Id};
use serde_json::json;

use crate::Session;
use crate::cmd::graph_mail::{apply_graph_mail, graph_fetch_request, graph_mail_request};

const OWNER: &str = "owner@example.org";
const SHARED: &str = "support@example.org";

/// A session with a Microsoft account kept in step through Graph, and a local one; their ids.
fn session() -> (Session, Id, Id) {
    let mut s = Session::empty();
    s.shared_mailboxes = true;
    let local = s.run("account.add", &json!({"email": "ada@example.org"})).unwrap()["id"].as_u64().unwrap();
    let owner = s.run("account.add", &json!({"email": OWNER, "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
    (s, local, owner)
}

fn add(s: &mut Session, params: serde_json::Value) -> Result<Id, String> {
    s.run("account.addSharedMailbox", &params).map(|v| v["id"].as_u64().unwrap_or(0)).map_err(|e| e.to_string())
}

/// What a mailbox's server lists: the Inbox and the Deleted Items, by ids that every mailbox of
/// these tests uses alike.
fn listing(account: Id, email: &str) -> GraphMailResult {
    let folder = |id: &str, name: &str, kind| GraphFolder { id: id.into(), name: name.into(), kind, ..Default::default() };
    let folders = vec![folder("in", "Inbox", FolderKind::Inbox), folder("de", "Deleted Items", FolderKind::Deleted)];
    GraphMailResult { account, email: email.into(), listing: Some(FolderListing { folders, complete: true }), ..Default::default() }
}

fn page(account: Id, email: &str, ids: &[&str]) -> GraphMailResult {
    let messages = ids
        .iter()
        .map(|id| GraphMessage {
            server: ServerMessage { id: (*id).into(), folder: "in".into(), ..Default::default() },
            subject: format!("{email}: {id}"),
            ..Default::default()
        })
        .collect();
    let page = MessagePage {
        folder: "in".into(),
        messages,
        link: "https://graph.example.org/delta?$deltatoken=1".into(),
        caught_up: true,
        ..Default::default()
    };
    GraphMailResult { account, email: email.into(), pages: vec![page], whole: true, ..Default::default() }
}

#[test]
fn a_shared_mailbox_is_added_under_a_microsoft_account_and_removed_again() {
    let (mut s, local, owner) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    let a = s.mailbox.account(shared).unwrap();
    assert_eq!((a.email.as_str(), a.kind, a.shared_of), (SHARED, AccountKind::Microsoft, Some(owner)));
    let listed = s.run("account.list", &json!({})).unwrap();
    assert!(listed["accounts"].as_array().unwrap().iter().any(|a| a["sharedOf"] == json!(owner) && a["email"] == SHARED));
    // Refused: under an account that isn't a Microsoft one, twice, an address that isn't one.
    assert!(add(&mut s, json!({"account": local, "address": "sales@example.org"})).unwrap_err().contains("Microsoft Graph"));
    assert!(add(&mut s, json!({"address": "Support@Example.org"})).unwrap_err().contains("already here"));
    assert!(add(&mut s, json!({"address": "support@example.org/../../me"})).unwrap_err().contains("isn't an e-mail address"));
    assert!(add(&mut s, json!({"account": shared, "address": "sales@example.org"})).unwrap_err().contains("can't open another"));
    assert!(add(&mut s, json!({})).is_err());
    // Removed by its address, or its id; an account that isn't one isn't removed this way.
    assert!(s.run("account.removeSharedMailbox", &json!({"id": owner})).is_err());
    assert!(s.run("account.removeSharedMailbox", &json!({"address": "nobody@example.org"})).is_err());
    s.run("account.removeSharedMailbox", &json!({"address": "SUPPORT@example.org"})).unwrap();
    assert!(s.mailbox.account(shared).is_none() && s.mailbox.folders.iter().all(|f| f.account != shared));
    let again = add(&mut s, json!({"account": owner, "address": SHARED})).unwrap();
    s.run("account.removeSharedMailbox", &json!({"id": again})).unwrap();
    assert!(s.run("account.removeSharedMailbox", &json!({"id": again})).is_err());
}

#[test]
fn the_last_account_is_not_removed_with_its_shared_mailboxes() {
    let mut s = Session::empty();
    s.shared_mailboxes = true;
    let owner = s.run("account.add", &json!({"email": OWNER, "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    assert!(s.run("account.remove", &json!({"id": owner})).unwrap_err().to_string().contains("last account"));
    assert!(s.mailbox.account(shared).is_some());
    s.run("account.add", &json!({"email": "ada@example.org"})).unwrap();
    s.run("account.remove", &json!({"id": owner})).unwrap();
    assert!(s.mailbox.account(shared).is_none(), "it goes with its owner");
    assert!(s.mailbox.folder(s.view.folder).is_some(), "the folder on screen is one that is there");
}

#[test]
fn a_shared_mailbox_is_read_at_its_own_address_with_its_owners_sign_in() {
    let (mut s, _, owner) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    let mine = graph_mail_request(&s, owner).unwrap();
    assert_eq!((mine.email.as_str(), mine.shared), (OWNER, false));
    let theirs = graph_mail_request(&s, shared).unwrap();
    assert_eq!((theirs.account, theirs.email.as_str(), theirs.shared), (shared, SHARED, true));
    let fetch = graph_fetch_request(&s, shared, None, 10).unwrap();
    assert_eq!((fetch.email.as_str(), fetch.shared), (SHARED, true));
    // Its Graph request by command says so too.
    let asked = s.run("sync.graphRequest", &json!({"account": shared})).unwrap();
    assert_eq!((asked["email"].as_str(), asked["shared"].as_bool()), (Some(SHARED), Some(true)));
    // An answer for the owner's mailbox is not taken in by the shared one, nor the other way.
    assert!(apply_graph_mail(&mut s, listing(shared, OWNER)).is_err());
    assert!(apply_graph_mail(&mut s, listing(owner, SHARED)).is_err());
    // A shared mailbox whose owner is gone (a file may say so) isn't read at all.
    if let Ok(a) = s.mailbox.account_mut(shared) {
        a.shared_of = Some(9_999);
    }
    assert!(graph_mail_request(&s, shared).unwrap_err().to_string().contains("account is gone"));
}

#[test]
fn the_same_server_ids_in_two_mailboxes_are_two_messages() {
    let (mut s, _, owner) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    apply_graph_mail(&mut s, listing(owner, OWNER)).unwrap();
    apply_graph_mail(&mut s, listing(shared, SHARED)).unwrap();
    let (mine, theirs) = (s.mailbox.folder_by_server(owner, "in").unwrap(), s.mailbox.folder_by_server(shared, "in").unwrap());
    assert_ne!(mine, theirs);
    // Each mailbox names its messages by itself: the same ids, other messages.
    for _ in 0..2 {
        apply_graph_mail(&mut s, page(owner, OWNER, &["AAkA1", "AAkA2"])).unwrap();
        apply_graph_mail(&mut s, page(shared, SHARED, &["AAkA1", "AAkA2", "AAkA3"])).unwrap();
    }
    assert_eq!((s.mailbox.count(mine), s.mailbox.count(theirs)), (2, 3), "and a page that came twice changes nothing");
    assert!(s.mailbox.messages_in(theirs).all(|m| m.account == shared && m.subject.starts_with(SHARED)));
    assert!(s.mailbox.messages_in(mine).all(|m| m.account == owner && m.subject.starts_with(OWNER)));
}

#[test]
fn mail_written_in_a_shared_mailbox_goes_from_its_owners_account() {
    let (mut s, _, owner) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    s.view.folder = s.mailbox.folder_of_kind(shared, FolderKind::Inbox).unwrap();
    s.run("mail.new", &json!({"to": "bob@example.org", "subject": "Hello"})).unwrap();
    s.run("compose.saveDraft", &json!({})).unwrap();
    let draft = s.mailbox.messages.values().find(|m| m.subject == "Hello").unwrap();
    assert_eq!((draft.account, draft.from.email.as_str()), (owner, OWNER));
    assert_eq!(s.mailbox.folder(draft.folder).map(|f| f.account), Some(owner));
}

#[test]
fn a_shared_mailbox_keeps_its_address_and_its_owners_sign_in() {
    let (mut s, _, _) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    let moved = s.run("account.edit", &json!({"id": shared, "email": "other@example.org"})).unwrap_err();
    assert!(moved.to_string().contains("can't be changed"), "{moved}");
    s.run("account.edit", &json!({"id": shared, "name": "Support", "kind": "imap", "incoming": {"host": "imap.example.org"}})).ok();
    let a = s.mailbox.account(shared).unwrap();
    assert_eq!((a.kind, a.incoming.is_none(), a.oauth.is_none(), a.shared_of.is_some()), (AccountKind::Microsoft, true, true, true));
}

#[test]
fn what_is_changed_in_a_shared_mailbox_waits_here_and_nothing_of_it_is_asked_to_go_up() {
    let (mut s, _, owner) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    apply_graph_mail(&mut s, listing(shared, SHARED)).unwrap();
    apply_graph_mail(&mut s, page(shared, SHARED, &["AAkA1", "AAkA2"])).unwrap();
    // The owner's mailbox has the same ids: what goes up for one must never name the other's.
    apply_graph_mail(&mut s, listing(owner, OWNER)).unwrap();
    apply_graph_mail(&mut s, page(owner, OWNER, &["AAkA1"])).unwrap();
    let theirs: Vec<Id> = s.mailbox.messages.values().filter(|m| m.account == shared).map(|m| m.id).collect();
    s.run("mail.markRead", &json!({"ids": theirs})).unwrap();
    s.run("mail.flag", &json!({"id": theirs[0]})).unwrap();
    let inbox = s.mailbox.folder_of_kind(shared, FolderKind::Inbox).unwrap();
    s.run("folder.new", &json!({"parent": inbox, "name": "Ours"})).unwrap();
    s.view.folder = inbox;
    let asked = graph_mail_request(&s, shared).unwrap();
    assert!(asked.shared && asked.changes.is_empty() && asked.messages.is_empty() && asked.drafts.is_empty() && asked.sends.is_empty(), "{asked:?}");
    assert!(!asked.links.is_empty(), "it is still read from where it got to");
    // They wait here, counted, and are still there for when changes go up.
    assert!(crate::cmd::graph_mail::unpushed(&s, shared) >= 3, "{}", crate::cmd::graph_mail::unpushed(&s, shared));
    // The owner's own changes are the owner's alone.
    let own = graph_mail_request(&s, owner).unwrap();
    assert!(own.messages.is_empty() && own.changes.is_empty(), "{own:?}");
}

#[test]
fn shared_mailboxes_are_off_until_they_are_switched_on_for_testing() {
    let (mut s, _, _) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    s.shared_mailboxes = false;
    assert!(add(&mut s, json!({"address": "sales@example.org"})).unwrap_err().contains("MAILCRAFT_SHARED_MAILBOXES"));
    assert!(graph_mail_request(&s, shared).is_err() && graph_fetch_request(&s, shared, None, 10).is_err());
    // What was opened while they were on can still be closed.
    s.run("account.removeSharedMailbox", &json!({"id": shared})).unwrap();
}

#[test]
fn a_shared_mailbox_keeps_as_much_mail_whole_as_its_owners_account() {
    let (mut s, _, owner) = session();
    let shared = add(&mut s, json!({"address": SHARED})).unwrap();
    apply_graph_mail(&mut s, listing(shared, SHARED)).unwrap();
    apply_graph_mail(&mut s, page(shared, SHARED, &["AAkA1"])).unwrap();
    // Each message is 30 MB: over the owner's 10 MB, under its Everything.
    let theirs: Vec<Id> = s.mailbox.messages.values().filter(|m| m.account == shared).map(|m| m.id).collect();
    for id in theirs {
        if let Some(server) = s.mailbox.message_mut(id).ok().and_then(|m| m.server.as_mut()) {
            server.size = 30 << 20;
        }
    }
    // (A file may say anything of the shared mailbox's own: it isn't what counts.)
    if let Ok(a) = s.mailbox.account_mut(shared) {
        a.download_in_full = mailcraft_model::DownloadInFull::Everything;
    }
    assert!(graph_fetch_request(&s, shared, None, 10).unwrap().ids.is_empty());
    if let Ok(a) = s.mailbox.account_mut(owner) {
        a.download_in_full = mailcraft_model::DownloadInFull::Everything;
    }
    if let Ok(a) = s.mailbox.account_mut(shared) {
        a.download_in_full = mailcraft_model::DownloadInFull::UpTo10Mb;
    }
    assert_eq!(graph_fetch_request(&s, shared, None, 10).unwrap().ids, ["AAkA1"]);
}

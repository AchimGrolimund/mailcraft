// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An Exchange server account (#252, E6), as the engine works out what a host should read and
//! takes in what it brings back, with results made by hand. (Against `mailcraft-ews`'s stand-in:
//! `apps/mailcraft/src/ews_mail_flow.rs`.)

use mailcraft_model::sync::{EwsFolderDeltas, EwsFolderState, EwsMessagePage, GraphFetched, GraphFolder, GraphMessage, ReadState, ServerMessage};
use mailcraft_model::{Address, FlagStatus, Message};
use serde_json::json;

use super::*;

const ME: &str = "ada@example.org";
const URL: &str = "https://exchange.example.org/EWS/Exchange.asmx";

fn session() -> (Session, Id) {
    let mut s = Session::empty();
    s.ews = true;
    let added = s.run("account.add", &json!({"email": ME, "kind": "exchange", "ews": {"url": URL}})).unwrap();
    (s, added["id"].as_u64().unwrap())
}

fn folder(id: &str, parent: &str, name: &str, kind: FolderKind) -> GraphFolder {
    GraphFolder { id: id.into(), parent: parent.into(), name: name.into(), kind, ..Default::default() }
}

fn folders() -> Vec<GraphFolder> {
    vec![
        folder("F1", "", "Inbox", FolderKind::Inbox),
        folder("F2", "", "Sent Items", FolderKind::Sent),
        folder("F3", "F5", "Apollo", FolderKind::Custom),
        folder("F5", "", "Projects", FolderKind::Custom),
    ]
}

fn mail(id: &str, subject: &str, received: i64) -> GraphMessage {
    GraphMessage {
        server: ServerMessage { id: id.into(), size: 1_200, change_key: "CK1".into(), ..Default::default() },
        message_id: format!("{id}@example.org"),
        subject: subject.into(),
        from: Address::new("Zoe Park", "zoe@example.org"),
        to: vec![Address::new("Ada", ME)],
        sent: received - 60,
        received,
        preview: format!("About {subject}"),
        ..Default::default()
    }
}

fn piece(account: Id) -> EwsMailResult {
    EwsMailResult { account, email: ME.into(), url: URL.into(), partial: true, ..Default::default() }
}

fn page(folder: &str, messages: Vec<GraphMessage>, afresh: bool, last: bool) -> EwsMessagePage {
    EwsMessagePage { folder: folder.into(), messages, afresh, last, ..Default::default() }
}

fn state(folder: &str, state: &str) -> EwsFolderState {
    EwsFolderState { folder: folder.into(), state: state.into() }
}

/// The folder list, as a first read brings it.
fn list(s: &mut Session, account: Id) {
    let deltas = EwsFolderDeltas { changed: folders(), afresh: true, ..Default::default() };
    apply_ews_mail(s, EwsMailResult { folders: Some(deltas), hierarchy: "HIER-1".into(), ..piece(account) }).unwrap();
}

/// One page of a folder with its state, more to come.
fn read(s: &mut Session, account: Id, page: EwsMessagePage, state_after: &str) -> Value {
    let folder = page.folder.clone();
    apply_ews_mail(s, EwsMailResult { pages: vec![page], states: vec![state(&folder, state_after)], ..piece(account) }).unwrap()
}

/// The run's last piece.
fn end(s: &mut Session, account: Id, whole: bool) -> Value {
    apply_ews_mail(s, EwsMailResult { whole, partial: false, ..piece(account) }).unwrap()
}

fn here<'s>(s: &'s Session, account: Id, id: &str) -> Option<&'s Message> {
    s.mailbox.messages.values().map(|m| &**m).find(|m| m.account == account && m.server.as_ref().is_some_and(|server| server.id == id))
}

fn state_of(s: &Session, account: Id, server: &str) -> String {
    let id = s.mailbox.folder_by_server(account, server).unwrap();
    s.mailbox.folder(id).unwrap().server.as_ref().unwrap().sync_state.clone()
}

#[test]
fn the_first_request_reads_everything_from_the_start() {
    let (s, account) = session();
    let request = ews_mail_request(&s, account, false).unwrap();
    assert_eq!((request.url.as_str(), request.email.as_str()), (URL, ME));
    assert!(request.list_folders && request.hierarchy.is_empty() && request.states.is_empty());
    assert_eq!(request.scope, EwsScope::All);
    // A light run of an account with nothing yet still reads the folder list.
    assert!(ews_mail_request(&s, account, true).unwrap().list_folders);
    assert!(ews_inbox_request(&s, account).is_none(), "no Inbox of the server's yet");
    // The request names no password and, in debug output, no address.
    assert!(!format!("{request:?}").contains("exchange.example.org"));
}

#[test]
fn the_folders_and_the_first_pages_come_in_with_the_states_that_go_on_from_them() {
    let (mut s, account) = session();
    list(&mut s, account);
    assert_eq!(s.mailbox.account(account).unwrap().ews.as_ref().unwrap().hierarchy, "HIER-1");
    let apollo = s.mailbox.folder_by_server(account, "F3").unwrap();
    let projects = s.mailbox.folder_by_server(account, "F5").unwrap();
    assert_eq!(s.mailbox.folder(apollo).unwrap().parent, Some(projects), "a folder lies in the one it is listed under, whichever comes first");
    let said = read(&mut s, account, page("F1", vec![mail("A1", "Kickoff", 1_000), mail("A2", "Budget", 2_000)], true, false), "S-1");
    assert_eq!(said["new"], 2);
    let kickoff = here(&s, account, "A1").unwrap();
    assert!(kickoff.unfetched && kickoff.body.text == "About Kickoff" && kickoff.subject == "Kickoff");
    assert_eq!(kickoff.server.as_ref().unwrap().change_key, "CK1");
    assert_eq!(state_of(&s, account, "F1"), "S-1");
    // The next page goes on from it, and a folder whose pages haven't ended hasn't had its first round.
    let request = ews_mail_request(&s, account, false).unwrap();
    assert!(request.states.iter().any(|st| st.folder == "F1" && st.state == "S-1"));
    assert!(request.states.iter().any(|st| st.folder == "F2" && st.state.is_empty()), "a folder not read yet is read from its start");
    assert_eq!(request.hierarchy, "HIER-1");
    read(&mut s, account, page("F1", vec![mail("A3", "Agenda", 3_000)], false, true), "S-2");
    assert_eq!(state_of(&s, account, "F1"), "S-2");
    let inbox = s.mailbox.folder_by_server(account, "F1").unwrap();
    assert!(s.mailbox.folder(inbox).unwrap().server.as_ref().unwrap().round_ended);
    assert_eq!(s.mailbox.messages.values().filter(|m| m.folder == inbox).count(), 3);
    // A light run reads the Inbox and the folder on screen, and its states are those alone.
    let light = ews_mail_request(&s, account, true).unwrap();
    assert!(!light.list_folders);
    assert_eq!(light.states.iter().map(|st| st.folder.as_str()).collect::<Vec<_>>(), vec!["F1"]);
    let pass = ews_inbox_request(&s, account).unwrap();
    assert!(pass.inbox_pass && pass.states.len() == 1 && !pass.list_folders);
}

#[test]
fn a_delta_changes_what_changed_and_what_left_goes_when_the_run_ends_well() {
    let (mut s, account) = session();
    list(&mut s, account);
    read(
        &mut s,
        account,
        page("F1", vec![mail("A1", "Kickoff", 1_000), mail("A2", "Budget", 2_000), mail("A3", "Agenda", 3_000)], true, true),
        "S-1",
    );
    end(&mut s, account, true);
    let mut edited = mail("A1", "Kickoff (moved)", 1_000);
    edited.server.change_key = "CK2".into();
    edited.server.flag = FlagStatus::Flagged;
    let delta = EwsMessagePage {
        folder: "F1".into(),
        messages: vec![edited, mail("A4", "Late", 4_000)],
        removed: vec!["A3".into()],
        read: vec![ReadState { id: "A2".into(), read: true }],
        last: true,
        ..Default::default()
    };
    let said = read(&mut s, account, delta, "S-2");
    assert_eq!((said["new"].as_u64(), said["changed"].as_u64(), said["gone"].as_u64()), (Some(1), Some(2), Some(1)));
    // Matched by the item id: one message, with the new change key, not a second one.
    let kickoff = here(&s, account, "A1").unwrap();
    assert_eq!((kickoff.subject.as_str(), kickoff.flag), ("Kickoff", FlagStatus::Flagged));
    assert_eq!(kickoff.server.as_ref().unwrap().change_key, "CK2");
    assert!(here(&s, account, "A2").unwrap().read);
    // Kept until a run that read every folder ends well; then gone.
    assert!(here(&s, account, "A3").is_some_and(|m| m.server.as_ref().unwrap().gone));
    end(&mut s, account, false);
    assert!(here(&s, account, "A3").is_some(), "a run that did not go well removes nothing");
    let said = end(&mut s, account, true);
    assert!(here(&s, account, "A3").is_none());
    assert_eq!(said["removedMessages"], 1);
    // Mail that arrives from now on is new mail, which the lists and the rules sort.
    assert_eq!(state_of(&s, account, "F1"), "S-2");
}

#[test]
fn a_folder_read_afresh_keeps_what_the_server_still_has_and_drops_the_rest() {
    let (mut s, account) = session();
    list(&mut s, account);
    read(&mut s, account, page("F1", vec![mail("A1", "Kickoff", 1_000), mail("A2", "Budget", 2_000)], true, true), "S-1");
    // The one fetched whole keeps its body through a fresh read: it is matched by item id.
    let kickoff = here(&s, account, "A1").unwrap().id;
    let mime = b"From: Zoe Park <zoe@example.org>\r\nTo: Ada <ada@example.org>\r\nSubject: Kickoff\r\nMessage-ID: <A1@example.org>\r\n\r\nThe whole text.\r\n";
    apply_ews_fetch(
        &mut s,
        GraphFetchResult { account, email: ME.into(), fetched: vec![GraphFetched { id: "A1".into(), mime: mime.to_vec() }], ..Default::default() },
    )
    .unwrap();
    assert!(!s.mailbox.message(kickoff).unwrap().unfetched);
    // The server lost its state: the fresh read lists A1 and A3, and not A2.
    let fresh = read(&mut s, account, page("F1", vec![mail("A1", "Kickoff", 1_000), mail("A3", "Agenda", 3_000)], true, true), "S-9");
    assert_eq!((fresh["new"].as_u64(), fresh["gone"].as_u64()), (Some(1), Some(0)));
    assert!(here(&s, account, "A2").is_some_and(|m| m.server.as_ref().unwrap().gone));
    end(&mut s, account, true);
    assert!(here(&s, account, "A2").is_none() && here(&s, account, "A3").is_some());
    let kickoff = here(&s, account, "A1").unwrap();
    assert_eq!(kickoff.id, s.mailbox.message(kickoff.id).unwrap().id);
    assert!(!kickoff.unfetched && kickoff.body.text.contains("The whole text."), "the message fetched whole stays whole");
    assert_eq!(state_of(&s, account, "F1"), "S-9");
}

#[test]
fn a_deleted_folder_goes_with_its_mail_and_a_renamed_one_keeps_where_it_went_on_from() {
    let (mut s, account) = session();
    list(&mut s, account);
    read(&mut s, account, page("F5", vec![mail("P1", "Plan", 1_000)], true, true), "S-P");
    let delta = EwsFolderDeltas { changed: vec![folder("F5", "", "Projects (old)", FolderKind::Custom)], deleted: vec!["F3".into()], afresh: false };
    apply_ews_mail(&mut s, EwsMailResult { folders: Some(delta), hierarchy: "HIER-2".into(), ..piece(account) }).unwrap();
    let projects = s.mailbox.folder_by_server(account, "F5").unwrap();
    assert_eq!(s.mailbox.folder(projects).unwrap().name, "Projects (old)");
    assert_eq!(state_of(&s, account, "F5"), "S-P", "a rename leaves where the folder goes on from");
    assert!(s.mailbox.folder_by_server(account, "F3").is_none(), "deleted on the server");
    assert!(s.mailbox.folder_by_server(account, "F1").is_some(), "what the delta doesn't mention stays");
    assert_eq!(s.mailbox.account(account).unwrap().ews.as_ref().unwrap().hierarchy, "HIER-2");
    // Read afresh, the folder list is every folder: one it lacks is gone, but not a standard folder.
    let all = EwsFolderDeltas { changed: vec![folder("F1", "", "Inbox", FolderKind::Inbox)], afresh: true, ..Default::default() };
    apply_ews_mail(&mut s, EwsMailResult { folders: Some(all), hierarchy: "HIER-3".into(), ..piece(account) }).unwrap();
    assert!(s.mailbox.folder_by_server(account, "F5").is_none());
    assert!(s.mailbox.folder_of_kind(account, FolderKind::Inbox).is_some());
}

#[test]
fn a_page_for_a_folder_that_is_not_here_is_passed_over_with_its_state() {
    let (mut s, account) = session();
    let said = read(&mut s, account, page("F1", vec![mail("A1", "Kickoff", 1_000)], true, true), "S-1");
    assert_eq!(said["new"], 0);
    assert!(here(&s, account, "A1").is_none());
}

#[test]
fn a_piece_that_is_too_large_is_refused_and_one_that_is_odd_is_cut_and_removes_nothing() {
    let (mut s, account) = session();
    list(&mut s, account);
    read(&mut s, account, page("F1", vec![mail("A1", "Kickoff", 1_000), mail("A2", "Budget", 2_000)], true, true), "S-1");
    end(&mut s, account, true);
    let before = s.mailbox.clone();
    // A subject, a preview, a name or a list far beyond any message: refused whole.
    for odd in [
        GraphMessage { subject: "x".repeat(20_000), ..mail("B1", "s", 1) },
        GraphMessage { preview: "x".repeat(20_000), ..mail("B1", "s", 1) },
        GraphMessage { from: Address::new("x".repeat(20_000), "a@example.org"), ..mail("B1", "s", 1) },
        GraphMessage { to: vec![Address::new("x", "a@example.org"); 2_000], ..mail("B1", "s", 1) },
        GraphMessage { message_id: "x".repeat(20_000), ..mail("B1", "s", 1) },
    ] {
        let why =
            apply_ews_mail(&mut s, EwsMailResult { pages: vec![page("F1", vec![odd], false, true)], ..piece(account) }).unwrap_err().to_string();
        assert!(why.contains("longer than is taken in"), "{why}");
    }
    let long_folder = EwsFolderDeltas { changed: vec![folder("F9", "", &"x".repeat(5_000), FolderKind::Custom)], ..Default::default() };
    assert!(apply_ews_mail(&mut s, EwsMailResult { folders: Some(long_folder), ..piece(account) }).is_err());
    assert!(s.mailbox == before);
    // An id that is no id: the message is passed over, the piece is cut, and the run that
    // follows it is not one that went well: nothing is removed for it.
    let odd_id = mail("<script>", "Odd", 5_000);
    read(&mut s, account, page("F1", vec![odd_id, mail("A3", "Agenda", 3_000)], false, false), "S-2");
    assert!(here(&s, account, "A3").is_some() && here(&s, account, "<script>").is_none());
    // The page that came cut is no fresh read, and its state is not kept.
    assert_eq!(state_of(&s, account, "F1"), "S-1");
    // Long text that is within the limits is cut to what is shown.
    let long = GraphMessage { subject: format!("{}\r\n{}", "y".repeat(2_000), "z"), ..mail("A4", "s", 4_000) };
    read(&mut s, account, page("F1", vec![long], false, true), "S-3");
    let subject = &here(&s, account, "A4").unwrap().subject;
    assert!(subject.chars().count() <= 998 && !subject.contains(['\r', '\n']));
}

#[test]
fn a_run_for_another_account_or_another_server_is_not_taken_in() {
    let (mut s, account) = session();
    list(&mut s, account);
    let other_account = EwsMailResult { email: "bob@example.org".into(), ..piece(account) };
    assert!(apply_ews_mail(&mut s, other_account).unwrap_err().to_string().contains("another account"));
    let other_server = EwsMailResult { url: "https://other.example.org/EWS/Exchange.asmx".into(), ..piece(account) };
    assert!(apply_ews_mail(&mut s, other_server).unwrap_err().to_string().contains("named before"));
    let local = s.run("account.add", &json!({"email": "local@example.org"})).unwrap()["id"].as_u64().unwrap();
    assert!(apply_ews_mail(&mut s, EwsMailResult { account: local, email: "local@example.org".into(), ..Default::default() }).is_err());
}

#[test]
fn what_a_run_says_of_trouble_is_told_and_a_certificate_asks() {
    let (mut s, account) = session();
    list(&mut s, account);
    let said = apply_ews_mail(&mut s, EwsMailResult { auth_refused: true, partial: false, ..piece(account) }).unwrap();
    assert_eq!(said["authRefused"], true);
    assert!(said["errors"][0].as_str().unwrap().contains("turned down the sign-in"));
    let print = "ab".repeat(32);
    let said = apply_ews_mail(&mut s, EwsMailResult { untrusted: print.clone(), whole: true, partial: false, ..piece(account) }).unwrap();
    assert_eq!(said["untrusted"], print.as_str());
    assert!(said["errors"][0].as_str().unwrap().contains("needs your say"), "{said}");
    // (A run with a certificate to answer for is not one that went well: nothing is removed.)
    let said = apply_ews_mail(&mut s, EwsMailResult { busy_for: Some(7), partial: false, ..piece(account) }).unwrap();
    assert_eq!(said["busyFor"], 7);
    assert!(s.sync_status.starts_with("Up to date") || !s.sync_status.is_empty());
}

#[test]
fn messages_are_fetched_whole_within_what_the_account_keeps() {
    let (mut s, account) = session();
    list(&mut s, account);
    read(&mut s, account, page("F1", vec![mail("A1", "Kickoff", 1_000), mail("A2", "Budget", 2_000)], true, true), "S-1");
    let queue = ews_fetch_request(&s, account, None, 10).unwrap();
    assert_eq!(queue.ids.len(), 2);
    assert!(queue.background && queue.url == URL && queue.email == ME);
    let kickoff = here(&s, account, "A1").unwrap().id;
    let one = ews_fetch_request(&s, account, Some(kickoff), 10).unwrap();
    assert_eq!(one.ids, vec!["A1".to_owned()]);
    assert!(!one.background);
    let mime = b"From: Zoe Park <zoe@example.org>\r\nTo: Ada <ada@example.org>\r\nSubject: Kickoff\r\nMessage-ID: <A1@example.org>\r\n\r\nThe whole text.\r\n";
    let said = apply_ews_fetch(
        &mut s,
        GraphFetchResult {
            account,
            email: ME.into(),
            fetched: vec![GraphFetched { id: "A1".into(), mime: mime.to_vec() }, GraphFetched { id: "<bad>".into(), mime: mime.to_vec() }],
            missing: vec!["A2".into()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(said["fetched"], 1);
    assert!(!here(&s, account, "A1").unwrap().unfetched);
    // The one the server no longer has is not asked for again in the background this session.
    assert!(ews_fetch_request(&s, account, None, 10).unwrap().ids.is_empty());
    // A command that asks for it by the commands' own door gets the same.
    assert_eq!(s.run("sync.ewsFetchRequest", &json!({"account": account, "message": kickoff})).unwrap()["ids"], json!([]));
    let again = s.run("sync.applyEwsFetch", &json!({"result": {"account": account, "email": ME, "fetched": [{"id": "A2", "mime": mime.to_vec()}]}}));
    assert!(again.is_ok());
}

#[test]
fn the_commands_take_a_result_by_json_and_leave_no_server_id_in_their_errors() {
    let (mut s, account) = session();
    let result = json!({"result": {"account": account, "email": ME, "url": URL, "folders": {"changed": [{"id": "F1", "name": "Inbox", "kind": "inbox"}], "afresh": true}, "hierarchy": "H", "partial": true}});
    s.run("sync.applyEws", &result).unwrap();
    assert!(s.mailbox.folder_by_server(account, "F1").is_some());
    let bad = json!({"result": {"account": account, "email": ME, "pages": [{"folder": "F1\u{7}", "messages": []}]}});
    let why = s.run("sync.applyEws", &bad).unwrap_err().to_string();
    assert!(!why.contains("F1"), "{why}");
    assert!(s.run("sync.applyEws", &json!({"result": "x"})).is_err());
}

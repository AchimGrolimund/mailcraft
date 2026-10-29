// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mail down against the strict stand-in (#252, E6): the first download, a delta, a lost sync
//! state, a busy server, hostile answers, a refused sign-in, and a message fetched whole.

use std::sync::atomic::AtomicBool;

use mailcraft_model::FolderKind;
use mailcraft_model::sync::{EwsFetchRequest, EwsFolderState, EwsMailRequest, EwsMailResult, EwsScope};

use crate::standin::{AUTHORIZATION, Hostile, StandIn, block_on};
use crate::*;

const URL: &str = "https://mail.example.org/EWS/Exchange.asmx";

fn client() -> Client {
    Client::new(URL, Secret::new(AUTHORIZATION)).unwrap()
}

fn request() -> EwsMailRequest {
    EwsMailRequest { email: "ada@example.org".into(), url: URL.into(), list_folders: true, ..Default::default() }
}

/// Runs the request to its end: every piece, in order, and the waits asked for.
fn run(standin: &StandIn, request: &EwsMailRequest) -> (Vec<EwsMailResult>, Vec<u32>) {
    run_as(&client(), standin, request)
}

fn run_as(client: &Client, standin: &StandIn, request: &EwsMailRequest) -> (Vec<EwsMailResult>, Vec<u32>) {
    let (mut pieces, mut waits) = (Vec::new(), Vec::new());
    let cancel = AtomicBool::new(false);
    block_on(sync_mail(
        client,
        standin,
        request,
        &cancel,
        &mut |ms| {
            waits.push(ms);
            true
        },
        &mut |piece| pieces.push(piece),
    ))
    .unwrap();
    (pieces, waits)
}

fn subjects(pieces: &[EwsMailResult]) -> Vec<String> {
    pieces.iter().flat_map(|p| &p.pages).flat_map(|page| &page.messages).map(|m| m.subject.clone()).collect()
}

fn last(pieces: &[EwsMailResult]) -> &EwsMailResult {
    pieces.last().unwrap()
}

#[test]
fn the_first_run_lists_the_folders_and_downloads_the_inbox_in_pages() {
    let standin = StandIn::new();
    {
        let mut s = standin.state();
        for n in 0..450 {
            s.deliver("inbox", &format!("Mail {n:03}"), n % 2 == 0);
        }
    }
    let (pieces, _) = run(&standin, &request());
    let folders = pieces.iter().find_map(|p| p.folders.as_ref()).unwrap();
    assert!(folders.afresh, "a first read of the folders is every folder");
    let kinds: Vec<FolderKind> = folders.changed.iter().map(|f| f.kind).collect();
    for kind in [FolderKind::Inbox, FolderKind::Drafts, FolderKind::Sent, FolderKind::Deleted, FolderKind::Junk] {
        assert!(kinds.contains(&kind), "{kind:?}");
    }
    // (The calendar is not a mail folder: left out.)
    assert!(folders.changed.iter().all(|f| f.name != "Calendar"));
    assert!(folders.changed.iter().all(|f| f.parent.is_empty()), "folders at the top have no parent here");
    assert!(!pieces.iter().find_map(|p| (!p.hierarchy.is_empty()).then(|| p.hierarchy.clone())).unwrap().is_empty());
    // 450 messages, 200 to a page.
    let inbox_pages: Vec<_> = pieces.iter().flat_map(|p| &p.pages).filter(|page| page.messages.len() > 1).collect();
    assert_eq!(inbox_pages.iter().map(|p| p.messages.len()).collect::<Vec<_>>(), vec![200, 200, 50]);
    assert!(inbox_pages[0].afresh && !inbox_pages[1].afresh, "only the first page begins a fresh read");
    assert_eq!(inbox_pages.iter().map(|p| p.last).collect::<Vec<_>>(), vec![false, false, true]);
    let first = &inbox_pages[0].messages[0];
    assert_eq!((first.subject.as_str(), first.to.len(), first.from.email.as_str()), ("Mail 000", 1, "zoe@example.org"));
    assert_eq!(first.preview, "Hello about Mail 000.");
    assert!(first.received > 0 && first.sent > 0 && first.server.size > 0 && !first.server.change_key.is_empty());
    assert!(first.message_id.ends_with("@example.org") && !first.message_id.starts_with('<'));
    assert!(last(&pieces).whole && !last(&pieces).partial && last(&pieces).errors.is_empty());
    // Each page comes with the state to go on from.
    assert!(pieces.iter().filter(|p| !p.pages.is_empty()).all(|p| p.states.len() == 1 && !p.states[0].state.is_empty()));
}

#[test]
fn a_run_from_the_states_it_gave_brings_the_changes_alone() {
    let standin = StandIn::new();
    let (a, b, c) = {
        let mut s = standin.state();
        (s.deliver("inbox", "Kickoff", false), s.deliver("inbox", "Budget", false), s.deliver("inbox", "Agenda", true))
    };
    let (first, _) = run(&standin, &request());
    let hierarchy = first.iter().find_map(|p| (!p.hierarchy.is_empty()).then(|| p.hierarchy.clone())).unwrap();
    let states: Vec<EwsFolderState> = first.iter().flat_map(|p| p.states.clone()).collect();
    let inbox = standin.state().folder_id("inbox").unwrap();
    assert!(states.iter().any(|s| s.folder == inbox));
    {
        let mut s = standin.state();
        s.deliver("inbox", "Late", false);
        s.edit_subject(&a, "Kickoff (moved)");
        s.set_read(&b, true);
        s.remove(&c);
        s.add_folder("Projects");
    }
    let delta = EwsMailRequest { hierarchy, states, ..request() };
    let (pieces, _) = run(&standin, &delta);
    assert_eq!(subjects(&pieces), vec!["Late".to_owned(), "Kickoff (moved)".to_owned()]);
    let page = pieces.iter().flat_map(|p| &p.pages).find(|page| !page.removed.is_empty()).unwrap();
    assert_eq!((page.removed.len(), page.read.len(), page.afresh), (1, 1, false));
    assert!(page.read[0].read);
    let folders = pieces.iter().find_map(|p| p.folders.as_ref()).unwrap();
    assert!(!folders.afresh);
    assert_eq!(folders.changed.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), vec!["Projects"]);
    // The new folder is read from its start too.
    assert!(pieces.iter().flat_map(|p| &p.pages).any(|p| p.afresh && p.messages.is_empty() && p.last));
    // Nothing happened since: an empty page, and the run is whole.
    let again = EwsMailRequest {
        states: pieces.iter().flat_map(|p| p.states.clone()).collect(),
        hierarchy: pieces.iter().find_map(|p| (!p.hierarchy.is_empty()).then(|| p.hierarchy.clone())).unwrap(),
        ..request()
    };
    let (pieces, _) = run(&standin, &again);
    assert!(subjects(&pieces).is_empty() && last(&pieces).whole);
}

#[test]
fn a_state_the_server_forgot_reads_the_folder_and_the_folders_afresh() {
    let standin = StandIn::new();
    standin.state().deliver("inbox", "Kickoff", false);
    let (first, _) = run(&standin, &request());
    let hierarchy = first.iter().find_map(|p| (!p.hierarchy.is_empty()).then(|| p.hierarchy.clone())).unwrap();
    let states: Vec<EwsFolderState> = first.iter().flat_map(|p| p.states.clone()).collect();
    standin.state().forget_sync_states();
    let (pieces, _) = run(&standin, &EwsMailRequest { hierarchy, states, ..request() });
    assert!(pieces.iter().find_map(|p| p.folders.as_ref()).unwrap().afresh, "the folder list is read afresh");
    assert_eq!(subjects(&pieces), vec!["Kickoff".to_owned()]);
    let page = pieces.iter().flat_map(|p| &p.pages).find(|page| !page.messages.is_empty()).unwrap();
    assert!(page.afresh && page.last);
    assert!(last(&pieces).whole && last(&pieces).errors.is_empty(), "a lost state is not trouble");
}

#[test]
fn a_busy_server_is_waited_for_and_then_given_up_on() {
    let standin = StandIn::new();
    standin.state().deliver("inbox", "Kickoff", false);
    standin.state().busy = (2, 1500);
    let (pieces, waits) = run(&standin, &request());
    assert_eq!(waits, vec![1500, 1500]);
    assert_eq!(subjects(&pieces), vec!["Kickoff".to_owned()]);
    assert!(last(&pieces).whole);
    // Busy past the tries: the run ends there, with the seconds it asked for.
    standin.state().busy = (100, 4000);
    let (pieces, waits) = run(&standin, &request());
    assert_eq!(waits.len(), 3);
    assert_eq!(last(&pieces).busy_for, Some(4));
    assert!(!last(&pieces).whole);
    // Throttling as Exchange says it: a 200 answer with an inner error.
    {
        let mut s = standin.state();
        s.busy = (1, 2000);
        s.busy_inner = true;
    }
    let (pieces, waits) = run(&standin, &request());
    assert_eq!(waits, vec![2000]);
    assert!(last(&pieces).whole);
}

#[test]
fn a_wait_that_is_called_off_is_a_stopped_run() {
    let standin = StandIn::new();
    standin.state().busy = (1, 1000);
    let cancel = AtomicBool::new(false);
    let mut pieces = Vec::new();
    block_on(sync_mail(&client(), &standin, &request(), &cancel, &mut |_| false, &mut |piece| pieces.push(piece))).unwrap();
    assert!(last(&pieces).stopped && !last(&pieces).whole);
}

#[test]
fn a_server_without_the_preview_is_asked_without_it() {
    let standin = StandIn::new();
    standin.state().deliver("inbox", "Kickoff", false);
    standin.state().lacks_preview = true;
    let (pieces, _) = run(&standin, &request());
    let message = pieces.iter().flat_map(|p| &p.pages).flat_map(|page| &page.messages).next().unwrap();
    assert_eq!((message.subject.as_str(), message.preview.as_str()), ("Kickoff", ""));
    assert!(last(&pieces).whole, "what a server lacks is not trouble");
}

#[test]
fn a_refused_sign_in_ends_the_run_and_says_so() {
    let standin = StandIn::new();
    let wrong = Client::new(URL, Secret::basic("ada", "not-the-password").unwrap()).unwrap();
    let (pieces, _) = run_as(&wrong, &standin, &request());
    assert_eq!(pieces.len(), 1);
    assert!(last(&pieces).auth_refused && !last(&pieces).whole && !last(&pieces).partial);
    assert!(standin.state().requests.iter().all(|r| r.status == 401));
}

#[test]
fn the_inbox_pass_reads_the_inbox_alone() {
    let standin = StandIn::new();
    standin.state().deliver("inbox", "Kickoff", false);
    standin.state().deliver("sentitems", "Sent one", true);
    let (pieces, _) = run(&standin, &EwsMailRequest { inbox_pass: true, list_folders: false, ..request() });
    assert_eq!(subjects(&pieces), vec!["Kickoff".to_owned()]);
    assert!(pieces.iter().all(|p| p.folders.is_none()));
    // A light run reads the Inbox and the folders named.
    let sent = standin.state().folder_id("sentitems").unwrap();
    let (pieces, _) = run(&standin, &EwsMailRequest { scope: EwsScope::Light { also: vec![sent] }, list_folders: false, ..request() });
    assert_eq!(subjects(&pieces), vec!["Kickoff".to_owned(), "Sent one".to_owned()]);
}

#[test]
fn a_hostile_server_ends_a_run_with_a_line_and_never_a_panic() {
    for how in [
        Hostile::Dtd,
        Hostile::CutShort,
        Hostile::NotXml,
        Hostile::Deep,
        Hostile::Huge,
        Hostile::Fault,
        Hostile::TooFew,
        Hostile::BadId,
        Hostile::BadItems,
    ] {
        let standin = StandIn::new();
        standin.state().deliver("inbox", "Kickoff", false);
        standin.state().hostile = Some(how);
        let (pieces, _) = run(&standin, &request());
        let end = last(&pieces);
        assert!(!end.partial && (how == Hostile::BadItems || !end.whole), "{how:?}");
        // Whatever it brought is within what the model takes in.
        for piece in &pieces {
            if how != Hostile::BadItems {
                assert!(piece.check().is_ok(), "{how:?}");
            }
        }
        let text = format!("{:?}", end.errors);
        assert!(!text.contains("script") && !text.contains("example.org"), "{how:?}: {text}");
    }
}

#[test]
fn items_with_ids_that_are_not_ids_are_passed_over() {
    let standin = StandIn::new();
    standin.state().deliver("inbox", "Kickoff", false);
    standin.state().hostile = Some(Hostile::BadItems);
    let (pieces, _) = run(&standin, &EwsMailRequest { inbox_pass: true, list_folders: false, ..request() });
    assert!(subjects(&pieces).is_empty(), "an id that isn't one is no message");
}

#[test]
fn a_message_is_fetched_whole_and_the_missing_and_the_refused_are_told() {
    let standin = StandIn::new();
    let id = standin.state().deliver("inbox", "Kickoff", false);
    let fetch = EwsFetchRequest {
        email: "ada@example.org".into(),
        url: URL.into(),
        ids: vec![id.clone(), "AAMkAGV4aXN0c19ub3QA".into()],
        ..Default::default()
    };
    let cancel = AtomicBool::new(false);
    let got = block_on(fetch_messages(&client(), &standin, &fetch, &cancel, &mut |_| true)).unwrap();
    assert_eq!(got.fetched.len(), 1);
    assert_eq!(got.fetched[0].id, id);
    assert!(String::from_utf8_lossy(&got.fetched[0].mime).contains("Hello about Kickoff."));
    assert_eq!(got.missing, vec!["AAMkAGV4aXN0c19ub3QA".to_owned()]);
    assert!(got.failed.is_empty() && got.errors.is_empty());
    // Over all it is a few requests of one message each; nothing was written to the mailbox.
    assert!(standin.state().writes.is_empty());
    // A server that answers garbage: the message is one it wouldn't give, and the batch goes on.
    standin.state().hostile = Some(Hostile::NotXml);
    let got = block_on(fetch_messages(&client(), &standin, &fetch, &cancel, &mut |_| true)).unwrap();
    assert!(got.fetched.is_empty() && got.failed.len() == 2);
}

#[test]
fn a_fetch_with_a_refused_sign_in_says_so() {
    let standin = StandIn::new();
    let id = standin.state().deliver("inbox", "Kickoff", false);
    let wrong = Client::new(URL, Secret::basic("ada", "nope").unwrap()).unwrap();
    let fetch = EwsFetchRequest { ids: vec![id], ..Default::default() };
    let got = block_on(fetch_messages(&wrong, &standin, &fetch, &AtomicBool::new(false), &mut |_| true)).unwrap();
    assert!(got.fetched.is_empty() && got.errors.len() == 1);
}

#[test]
fn the_requests_hold_to_the_schema_and_write_nothing() {
    let standin = StandIn::new();
    standin.state().deliver("inbox", "Kickoff", false);
    let (_, _) = run(&standin, &request());
    let state = standin.state();
    assert!(state.writes.is_empty());
    assert!(
        state.requests.iter().all(|r| r.status == 200),
        "{:?}",
        state.requests.iter().map(|r| (&r.operation, r.status, &r.code)).collect::<Vec<_>>()
    );
    let operations: Vec<&str> = state.requests.iter().map(|r| r.operation.as_str()).collect();
    assert_eq!(operations.iter().take(2).collect::<Vec<_>>(), [&"GetFolder", &"SyncFolderHierarchy"]);
    assert!(operations.iter().skip(2).all(|o| *o == "SyncFolderItems") && operations.len() == 8, "{operations:?}");
}

#[test]
fn a_run_is_called_off_before_its_next_request() {
    let standin = StandIn::new();
    let cancel = AtomicBool::new(true);
    let mut pieces = Vec::new();
    block_on(sync_mail(&client(), &standin, &request(), &cancel, &mut |_| true, &mut |piece| pieces.push(piece))).unwrap();
    assert_eq!(pieces.len(), 1);
    assert!(last(&pieces).stopped && standin.state().requests.is_empty());
}

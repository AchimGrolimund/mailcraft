// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mail moved and copied between a shared mailbox and its owner's (#70), from end to end: the
//! engine's `graph_mail_request` and `apply_graph_mail` with `mailcraft_net::graph::sync_mail` in
//! between, against the strict Graph stand-in. Microsoft moves and copies only within a mailbox,
//! so a copy is made in the destination by its own run at its own address, and the original goes
//! from its own mailbox only once that copy is confirmed there. Every way it can go wrong is
//! tried here: nothing is ever lost, nothing meant for the shared mailbox reaches `/me`, and
//! nothing of the owner's reaches the shared mailbox's address. Both mailboxes hold the same ids,
//! as Exchange's may. Nothing here leaves this machine; every address and message is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::graph_mail::{apply_graph_fetch, apply_graph_mail, graph_fetch_request, graph_mail_request, unpushed};
use mailcraft_model::{FolderKind, Id, Mailbox};
use mailcraft_net::graph::standin::{Fault, StandIn, TOKEN};
use serde_json::json;

/// Friday 2027-01-15 08:00 UTC.
const NOW: i64 = 1_800_000_000;
/// The addresses of a test's own two mailboxes: the gate that counts a mailbox's requests (and
/// waits when one is told to come back later) is the process's, so no two tests share one.
fn addresses() -> (String, String) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    (format!("imogen{n}@example.org"), format!("reception{n}@example.org"))
}
/// MailCraft's marker, as the stand-in keeps extended properties.
const MARKER: &str = "String {6d1c4e3a-8f2b-4c5d-9e0a-1b2c3d4e5f60} Name MailCraftMarker";

/// One whole run of `account`, each piece taken in as it comes; the requests it made, as
/// `METHOD path body`, are checked to be at the right mailbox's address and no other.
fn run(server: &StandIn, s: &mut Session, account: Id) {
    let request = graph_mail_request(s, account).unwrap();
    let shared = request.shared;
    let address = request.email.clone();
    server.state().requests.clear();
    {
        let _routed = server.route();
        let cancel = AtomicBool::new(false);
        mailcraft_net::graph::sync_mail(TOKEN, &request, &cancel, &mut |piece| {
            apply_graph_mail(s, piece).unwrap();
        });
    }
    for r in server.state().requests.iter() {
        let path = r.path.join("/");
        let said = format!("{} {path} {}", r.method, r.body);
        if shared {
            // Nothing meant for the shared mailbox reaches the owner's.
            assert!(path == "v1.0/$batch" || path.starts_with(&format!("v1.0/users/{address}/")), "{said}");
            assert!(!path.starts_with("v1.0/me") && !r.body.contains("\"/me/") && !r.body.contains("\"me/"), "{said}");
        } else {
            // And nothing of the owner's reaches the shared mailbox's address.
            assert!(!said.to_lowercase().contains("users/") && !said.to_lowercase().contains("reception"), "{said}");
        }
    }
}

/// Every message of `account` its server should give whole, fetched and taken in: only a message
/// that is whole here goes to another mailbox.
fn fetch(server: &StandIn, s: &mut Session, account: Id) {
    let request = graph_fetch_request(s, account, None, 100).unwrap();
    let _routed = server.route();
    let result = mailcraft_net::graph::fetch_messages(TOKEN, &request, &AtomicBool::new(false));
    apply_graph_fetch(s, result).unwrap();
}

/// The shared mailbox's folder `well_known` on the stand-in.
fn theirs(server: &StandIn, well_known: &str) -> String {
    server.state().others[0].folders.iter().find(|f| f.well_known == well_known).unwrap().id.clone()
}

/// What the stand-in has in a folder: (id, subject, draft, has the marker), in the shared mailbox
/// or the owner's.
fn in_folder(server: &StandIn, shared: bool, folder: &str) -> Vec<(String, String, bool, bool)> {
    let state = server.state();
    let mail = if shared { &state.others[0].mailbox } else { &state.mailbox };
    let mut all: Vec<(String, String, bool, bool)> = mail
        .messages
        .values()
        .filter(|m| m.folder == folder)
        .map(|m| (m.id.clone(), m.subject.clone(), m.draft, m.extended.iter().any(|(id, _)| id == MARKER)))
        .collect();
    all.sort();
    all
}

fn has(server: &StandIn, shared: bool, id: &str) -> bool {
    let state = server.state();
    if shared { state.others[0].mailbox.messages.contains_key(id) } else { state.mailbox.messages.contains_key(id) }
}

/// A stand-in with both mailboxes, three messages in each Inbox under the same ids, and a session
/// that has read both: the stand-in, the session, the owner, the shared mailbox and the ids.
fn set_up() -> (StandIn, Session, Id, Id, Vec<String>) {
    let (owner_address, shared_address) = addresses();
    let server = StandIn::start().unwrap();
    server.state().add_other(&shared_address, true);
    let mine = server.state().well_known("inbox");
    let own = server.state().mailbox.generate(&mine, 3, NOW);
    let theirs_ids = {
        let mut state = server.state();
        let other = state.other(&shared_address).unwrap();
        let inbox = other.folders.iter().find(|f| f.well_known == "inbox").unwrap().id.clone();
        other.mailbox.generate(&inbox, 3, NOW)
    };
    assert_eq!(own, theirs_ids, "the same ids in both mailboxes");
    let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
    let owner = s.run("account.add", &json!({"email": owner_address, "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
    let shared = s.run("account.addSharedMailbox", &json!({"account": owner, "address": shared_address})).unwrap()["id"].as_u64().unwrap();
    run(&server, &mut s, owner);
    run(&server, &mut s, shared);
    fetch(&server, &mut s, owner);
    fetch(&server, &mut s, shared);
    assert!(s.mailbox.messages.values().all(|m| !m.unfetched));
    (server, s, owner, shared, own)
}

/// The message here that is `id` on `account`'s server.
fn here(s: &Session, account: Id, id: &str) -> Id {
    s.mailbox.messages.values().find(|m| m.account == account && m.server.as_ref().is_some_and(|x| x.id == id)).map(|m| m.id).unwrap()
}

fn inbox(s: &Session, account: Id) -> Id {
    s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap()
}

#[test]
fn moved_both_ways_and_copied_the_copy_is_made_at_the_destinations_own_address_and_the_original_goes_only_once_it_is_there() {
    let (server, mut s, owner, shared, ids) = set_up();
    let my_inbox = server.state().well_known("inbox");
    let their_inbox = theirs(&server, "inbox");
    // The owner's first message into the shared mailbox's Inbox; the shared mailbox's second into
    // the owner's Inbox; the shared mailbox's third copied into the owner's.
    let (mine, theirs2, theirs3) = (here(&s, owner, &ids[0]), here(&s, shared, &ids[1]), here(&s, shared, &ids[2]));
    s.run("mail.flag", &json!({"id": mine})).unwrap();
    s.run("mail.categorize", &json!({"ids": [mine], "category": "Blue category"})).unwrap();
    s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap();
    s.run("mail.move", &json!({"ids": [theirs2], "folder": inbox(&s, owner)})).unwrap();
    let copy = s.run("mail.copy", &json!({"ids": [theirs3], "folder": inbox(&s, owner)})).unwrap();
    let copy = copy["copies"].as_array().and_then(|ids| ids.first()).and_then(|id| id.as_u64()).unwrap_or_else(|| panic!("{copy}"));
    // There at once here, each waiting to be made in its new mailbox.
    assert_eq!(s.mailbox.message(mine).map(|m| m.account), Some(shared));
    assert_eq!(s.mailbox.message(theirs2).map(|m| m.account), Some(owner));
    assert_eq!(s.mailbox.message(copy).map(|m| m.account), Some(owner));
    assert_eq!((unpushed(&s, shared), unpushed(&s, owner)), (1, 2));

    // The owner's run first: the two made in his mailbox. Nothing is taken from the shared
    // mailbox yet (its own run does that), and the owner's own first message is still his.
    run(&server, &mut s, owner);
    let made_mine: Vec<_> = in_folder(&server, false, &my_inbox).into_iter().filter(|m| m.3).collect();
    assert_eq!(made_mine.len(), 2, "{made_mine:?}");
    assert!(made_mine.iter().all(|m| !m.2), "received mail, not drafts");
    assert!(has(&server, true, &ids[1]) && has(&server, true, &ids[2]), "the originals are still in the shared mailbox");
    assert!(has(&server, false, &ids[0]), "the owner's message moved out is still his until the shared mailbox has it");

    // The shared mailbox's run: its copy of the owner's message made, and its own moved one goes.
    run(&server, &mut s, shared);
    let made_theirs: Vec<_> = in_folder(&server, true, &their_inbox).into_iter().filter(|m| m.3).collect();
    assert_eq!(made_theirs.len(), 1);
    assert!(!has(&server, true, &ids[1]), "moved out once its copy was in the owner's mailbox");
    assert!(has(&server, true, &ids[2]), "copied: still there");
    let made_there = server.state().others[0].mailbox.messages.get(&made_theirs[0].0).cloned().unwrap();
    assert!(made_there.flag == "flagged" && made_there.categories.iter().any(|c| c == "Blue category"), "{made_there:?}");
    assert!(made_there.internet_id.contains("mail.example.org"), "its Message-ID went with it: {}", made_there.internet_id);
    assert!(has(&server, false, &ids[0]), "and the owner's original goes with his next run");

    // The owner's run again: his moved message goes now, its copy confirmed in the shared mailbox.
    run(&server, &mut s, owner);
    assert!(!has(&server, false, &ids[0]));
    // Here: each one message, its server's copy, no longer being moved; nothing more to send.
    for (id, account) in [(mine, shared), (theirs2, owner), (copy, owner)] {
        let m = s.mailbox.message(id).unwrap();
        assert_eq!(m.account, account);
        assert!(m.moving_out.is_none() && m.server.as_ref().is_some_and(|x| !x.id.is_empty()), "{m:?}");
    }
    run(&server, &mut s, shared);
    run(&server, &mut s, owner);
    assert_eq!((unpushed(&s, shared), unpushed(&s, owner)), (0, 0));
    // Nothing twice: three in the owner's Inbox here (two of his own, the moved one, the copy,
    // less his own moved away) and four in the shared mailbox's.
    assert_eq!((s.mailbox.count(inbox(&s, owner)), s.mailbox.count(inbox(&s, shared))), (4, 3));
    assert_eq!((in_folder(&server, false, &my_inbox).len(), in_folder(&server, true, &their_inbox).len()), (4, 3));
}

#[test]
fn a_copy_the_destination_refuses_puts_the_message_back_where_its_original_still_is() {
    let (server, mut s, owner, shared, ids) = set_up();
    let their_inbox = theirs(&server, "inbox");
    let mine = here(&s, owner, &ids[0]);
    s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap();
    let address = s.mailbox.account(shared).unwrap().email.clone();
    server.state().fail_next.push((format!("POST /v1.0/users/{address}/mailFolders/{their_inbox}/messages"), 400, "ErrorInvalidRequest".into()));
    run(&server, &mut s, shared);
    let m = s.mailbox.message(mine).unwrap();
    assert_eq!((m.account, m.folder), (owner, inbox(&s, owner)), "back in the owner's Inbox");
    assert_eq!(m.server.as_ref().map(|x| x.id.as_str()), Some(ids[0].as_str()), "as the server message it always was");
    assert!(s.sync_status.contains("Couldn't move"), "{}", s.sync_status);
    assert!(in_folder(&server, true, &their_inbox).iter().all(|m| !m.3), "nothing made there");
    run(&server, &mut s, owner);
    assert!(has(&server, false, &ids[0]), "nothing taken from the owner's mailbox");
    assert_eq!(unpushed(&s, owner), 0, "and no move of it goes up");

    // A copy refused is taken away here again; its source is where it was.
    let theirs1 = here(&s, shared, &ids[1]);
    let copy = s.run("mail.copy", &json!({"ids": [theirs1], "folder": inbox(&s, owner)})).unwrap()["copies"][0].as_u64().unwrap();
    let my_inbox = server.state().well_known("inbox");
    server.state().fail_next.push((format!("POST /v1.0/me/mailFolders/{my_inbox}/messages"), 403, "ErrorAccessDenied".into()));
    run(&server, &mut s, owner);
    assert!(s.mailbox.message(copy).is_none() && s.mailbox.message(theirs1).is_some_and(|m| m.account == shared));
    assert!(has(&server, true, &ids[1]));
}

#[test]
fn a_shared_mailbox_that_wont_let_the_owner_in_keeps_the_message_waiting_and_the_original_where_it_is() {
    let (server, mut s, owner, shared, ids) = set_up();
    let mine = here(&s, owner, &ids[0]);
    s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap();
    server.state().others[0].full_access = false;
    run(&server, &mut s, shared);
    run(&server, &mut s, owner);
    let m = s.mailbox.message(mine).unwrap();
    assert!(m.account == shared && m.server.is_none() && m.moving_out.is_some(), "waiting: {m:?}");
    assert!(has(&server, false, &ids[0]), "the original stays");
    assert_eq!(unpushed(&s, shared), 1);
    // Access given back: it is made there, and only then does the original go.
    server.state().others[0].full_access = true;
    run(&server, &mut s, shared);
    assert!(has(&server, false, &ids[0]));
    run(&server, &mut s, owner);
    assert!(!has(&server, false, &ids[0]));
    assert!(has(&server, true, &ids[0]), "the shared mailbox's own message of the same id is untouched");
}

#[test]
fn a_delete_of_the_original_refused_or_held_never_loses_the_copy_and_is_asked_again() {
    let (server, mut s, owner, shared, ids) = set_up();
    // Out of the shared mailbox into the owner's: copy made, then its delete there is held (a
    // 403 that isn't "no access"), and asked again once it is allowed.
    let theirs0 = here(&s, shared, &ids[0]);
    s.run("mail.move", &json!({"ids": [theirs0], "folder": inbox(&s, owner)})).unwrap();
    run(&server, &mut s, owner);
    server.state().others[0].mailbox.faults.insert(ids[0].clone(), Fault::OverQuota);
    run(&server, &mut s, shared);
    assert!(has(&server, true, &ids[0]), "held: still there");
    assert!(s.mailbox.message(theirs0).is_some_and(|m| m.moving_out.is_some()), "still being moved");
    server.state().others[0].mailbox.faults.clear();
    run(&server, &mut s, shared);
    assert!(!has(&server, true, &ids[0]));
    assert!(has(&server, false, &ids[0]), "the owner's own message of the same id is untouched");
    assert!(s.mailbox.message(theirs0).is_some_and(|m| m.moving_out.is_none() && m.account == owner));

    // Out of the owner's into the shared one: the delete is told to come back later, then refused
    // for good; the copy stays, and so does the original (twice, nothing lost).
    let mine1 = here(&s, owner, &ids[1]);
    s.run("mail.move", &json!({"ids": [mine1], "folder": inbox(&s, shared)})).unwrap();
    run(&server, &mut s, shared);
    server.state().mailbox.faults.insert(ids[1].clone(), Fault::Throttled(1_000));
    run(&server, &mut s, owner);
    assert!(has(&server, false, &ids[1]), "busy: it waits");
    server.state().mailbox.faults.insert(ids[1].clone(), Fault::Denied);
    run(&server, &mut s, owner);
    assert!(has(&server, false, &ids[1]), "refused: the original stays");
    let m = s.mailbox.message(mine1).unwrap();
    assert!(m.account == shared && m.moving_out.is_none() && m.server.is_some(), "and the copy is the one here: {m:?}");
}

#[test]
fn a_run_cut_off_after_the_copy_finds_it_by_its_marker_and_never_makes_it_twice() {
    let (server, mut s, owner, shared, ids) = set_up();
    let their_inbox = theirs(&server, "inbox");
    let mine = here(&s, owner, &ids[0]);
    s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap();
    // Made, and the answer lost.
    server.state().lose_next.push("POST /v1.0/users/*/mailFolders/*/messages".into());
    run(&server, &mut s, shared);
    assert_eq!(in_folder(&server, true, &their_inbox).iter().filter(|m| m.3).count(), 1, "made once");
    assert!(s.mailbox.message(mine).is_some_and(|m| m.server.is_none()), "not known here yet");
    run(&server, &mut s, owner);
    assert!(has(&server, false, &ids[0]), "the original stays while the copy isn't known");

    // The program stops here, between the copy and the delete: the mailbox file as it was is all
    // that is left.
    let saved = serde_json::to_value(&s.mailbox).unwrap();
    let mut s = Session::at(serde_json::from_value::<Mailbox>(saved).unwrap(), NOW, 0);
    run(&server, &mut s, shared);
    assert_eq!(in_folder(&server, true, &their_inbox).iter().filter(|m| m.3).count(), 1, "found by its marker, not made twice");
    run(&server, &mut s, owner);
    assert!(!has(&server, false, &ids[0]));
    run(&server, &mut s, shared);
    assert_eq!(s.mailbox.messages.values().filter(|m| m.account == shared && m.folder == inbox(&s, shared)).count(), 4, "one each, nothing twice");
}

#[test]
fn a_server_that_keeps_the_copy_as_a_draft_has_it_taken_away_and_the_message_stays_where_it_was() {
    let (server, mut s, owner, shared, ids) = set_up();
    let their_inbox = theirs(&server, "inbox");
    server.state().drafts_only = true;
    let mine = here(&s, owner, &ids[0]);
    s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap();
    run(&server, &mut s, shared);
    assert!(in_folder(&server, true, &their_inbox).iter().all(|m| !m.3), "the draft it made is gone again");
    assert!(s.mailbox.message(mine).is_some_and(|m| m.account == owner && m.server.is_some()));
    run(&server, &mut s, owner);
    assert!(has(&server, false, &ids[0]));
}

#[test]
fn undo_of_a_move_between_mailboxes_before_and_after_it_reached_the_servers() {
    let (server, mut s, owner, shared, ids) = set_up();
    // Undone before any run: nothing is asked of either server.
    let mine = here(&s, owner, &ids[0]);
    s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert!(s.mailbox.message(mine).is_some_and(|m| m.account == owner && m.server.is_some() && m.moving_out.is_none()));
    assert_eq!((unpushed(&s, shared), unpushed(&s, owner)), (0, 0));

    // Undone once the copy is there and the original gone: it goes back as it came, a copy made
    // in the owner's mailbox and the shared mailbox's taken away once that is there.
    s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap();
    run(&server, &mut s, shared);
    run(&server, &mut s, owner);
    assert!(!has(&server, false, &ids[0]));
    let there = s.mailbox.message(mine).and_then(|m| m.server.clone()).unwrap().id;
    s.run("edit.undo", &json!({})).unwrap();
    let m = s.mailbox.message(mine).unwrap();
    assert_eq!((m.account, m.folder), (owner, inbox(&s, owner)));
    run(&server, &mut s, owner);
    run(&server, &mut s, shared);
    assert!(!has(&server, true, &there), "the shared mailbox's copy went once the owner's was made");
    let m = s.mailbox.message(mine).unwrap();
    assert!(m.account == owner && m.moving_out.is_none() && m.server.is_some(), "{m:?}");
    assert_eq!((unpushed(&s, shared), unpushed(&s, owner)), (0, 0));
}

#[test]
fn a_message_not_downloaded_yet_is_refused_with_why_and_stays() {
    let (_server, mut s, owner, shared, ids) = set_up();
    let mine = here(&s, owner, &ids[0]);
    std::sync::Arc::make_mut(s.mailbox.messages.get_mut(&mine).unwrap()).unfetched = true;
    let e = s.run("mail.move", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap_err().to_string();
    assert!(e.contains("isn't downloaded yet"), "{e}");
    let e = s.run("mail.copy", &json!({"ids": [mine], "folder": inbox(&s, shared)})).unwrap_err().to_string();
    assert!(e.contains("isn't downloaded yet"), "{e}");
    assert!(s.mailbox.message(mine).is_some_and(|m| m.account == owner));
    assert_eq!(unpushed(&s, shared), 0);
}

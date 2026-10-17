//! A shared mailbox's folders and mail through the strict Graph stand-in (#70): asked at
//! `/users/{address}/…` with the owner's token, and nothing of it at `/me`. Nothing here leaves
//! this machine; every address and message is invented.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use mailcraft_model::FolderKind;
use mailcraft_model::sync::{
    DraftUp, FolderChange, GraphFetchRequest, GraphLink, GraphMailRequest, GraphMailResult, MessageChange, SendUp, SharedRefusal,
};

use super::whole_run;
use crate::graph::pace::{Gate, Retry};
use crate::graph::shared_mailbox_path;
use crate::graph::standin::{StandIn, TOKEN};

const QUICK: Retry = Retry { tries: 4, first_wait: Duration::from_millis(5), longest_wait: Duration::from_secs(60) };
const NOW: i64 = 1_790_000_000;
const SHARED: &str = "support@example.org";

fn request(email: &str, links: Vec<GraphLink>) -> GraphMailRequest {
    GraphMailRequest { account: 4, email: email.into(), shared: true, links, ..Default::default() }
}

fn run(server: &StandIn, request: &GraphMailRequest) -> Vec<GraphMailResult> {
    whole_run(&server.url(), TOKEN, request, Arc::new(Gate::new()), QUICK, &AtomicBool::new(false))
}

/// The other mailbox's well-known folder `name`.
fn other_folder(server: &StandIn, name: &str) -> String {
    let mut state = server.state();
    let other = state.other(SHARED).unwrap();
    other.folders.iter().find(|f| f.well_known == name).unwrap().id.clone()
}

#[test]
fn a_shared_mailbox_is_read_at_its_own_address_with_the_owners_token() {
    let server = StandIn::start().unwrap();
    server.state().add_other(SHARED, true);
    let (theirs, mine) = (other_folder(&server, "inbox"), server.state().well_known("inbox"));
    let ids = server.state().other(SHARED).unwrap().mailbox.generate(&theirs, 120, NOW);
    // The owner's own mailbox names its messages the same way: the same ids, other messages.
    let own = server.state().mailbox.generate(&mine, 3, NOW);
    assert_eq!(own[0], ids[0]);

    let pieces = run(&server, &request(SHARED, Vec::new()));
    let last = pieces.last().unwrap();
    assert!(!last.partial && last.whole && last.refused.is_none() && last.errors.is_empty(), "{:?}", last.errors);
    let listing = pieces.iter().find_map(|p| p.listing.as_ref()).unwrap();
    assert!(listing.complete);
    let inbox = listing.folders.iter().find(|f| f.kind == FolderKind::Inbox).unwrap();
    assert_eq!(inbox.id, theirs, "the shared mailbox's own Inbox");
    let came: Vec<&str> = pieces.iter().flat_map(|p| &p.pages).flat_map(|p| &p.messages).map(|m| m.server.id.as_str()).collect();
    assert_eq!(came.len(), 120);
    assert!(pieces.iter().flat_map(|p| &p.pages).all(|p| p.folder != mine));
    assert!(pieces.iter().all(|p| p.email == SHARED && p.account == 4));

    // Every question was the shared mailbox's, by its address, with immutable ids; none was the
    // owner's own.
    let requests = server.state().requests.clone();
    assert!(!requests.is_empty());
    for asked in &requests {
        let path = asked.path.join("/");
        assert!(path == "v1.0/$batch" || path.starts_with(&format!("v1.0/users/{SHARED}/")), "{path}");
        assert!(asked.header("prefer").contains("ImmutableId"), "{path}");
        assert!(!path.contains("/me/") && asked.status < 400, "{path}: {}", asked.status);
    }

    // Next time it goes on from its links, at the same address.
    let mut links: Vec<GraphLink> = Vec::new();
    for page in pieces.iter().flat_map(|p| &p.pages) {
        links.retain(|l| l.folder != page.folder);
        links.push(GraphLink { folder: page.folder.clone(), link: page.link.clone(), caught_up: page.caught_up });
    }
    assert!(links.iter().all(|l| l.link.contains(&format!("/v1.0/users/{SHARED}/mailFolders/"))));
    server.state().other(SHARED).unwrap().mailbox.add(&theirs, "One more", NOW + 60);
    server.state().requests.clear();
    let again = run(&server, &request(SHARED, links));
    let came: Vec<&str> = again.iter().flat_map(|p| &p.pages).flat_map(|p| &p.messages).map(|m| m.subject.as_str()).collect();
    assert_eq!(came, ["One more"]);
    assert!(server.state().requests.iter().all(|r| !r.path.join("/").starts_with("v1.0/me")));

    // Fetched whole at its address too.
    let fetch = GraphFetchRequest { account: 4, email: SHARED.into(), shared: true, ids: vec![ids[0].clone()], background: false };
    let got = super::fetch::fetch_messages_with(&server.url(), TOKEN, &fetch, Arc::new(Gate::new()), QUICK, &AtomicBool::new(false));
    assert_eq!(got.fetched.len(), 1, "{:?}", got.errors);
    assert!(String::from_utf8_lossy(&got.fetched[0].mime).contains("Subject: Message 0\r\n"));
    let asked = server.state().requests.last().cloned().unwrap();
    assert_eq!(asked.path.first().map(String::as_str), Some("v1.0"));
    assert_eq!(asked.path.get(1..3).map(|p| p.join("/")), Some(format!("users/{SHARED}")));
}

#[test]
fn a_shared_mailbox_without_full_access_is_said_plainly_and_not_asked_again_and_again() {
    let server = StandIn::start().unwrap();
    server.state().add_other(SHARED, false);
    let pieces = run(&server, &request(SHARED, Vec::new()));
    assert_eq!(pieces.len(), 1, "no mail is asked for");
    let only = &pieces[0];
    assert!(only.refused == Some(SharedRefusal::NoAccess) && only.listing.is_none() && only.pages.is_empty());
    assert!(only.errors.iter().any(|e| e.contains("Full Access")), "{:?}", only.errors);
    assert!(only.errors.iter().all(|e| !e.contains(SHARED)), "no address in what may be logged: {:?}", only.errors);
    // A refusal is not asked again: the batch, and the one question it left open.
    let requests = server.state().requests.clone();
    let direct: Vec<_> = requests.iter().filter(|r| r.path.join("/") != "v1.0/$batch").collect();
    assert!(requests.len() <= 12 && direct.iter().all(|r| r.status == 403), "{}", requests.len());

    // An address the server has no mailbox for.
    server.state().requests.clear();
    let pieces = run(&server, &request("nobody@example.org", Vec::new()));
    assert!(
        pieces[0].refused == Some(SharedRefusal::NoMailbox) && pieces[0].errors.iter().any(|e| e.contains("no mailbox at this address")),
        "{:?}",
        pieces[0].errors
    );
}

#[test]
fn a_403_is_told_apart_by_graphs_code_not_by_its_status() {
    let server = StandIn::start().unwrap();
    server.state().add_other(SHARED, true);
    // Not reachable through Graph: said as such.
    server.state().other(SHARED).unwrap().refusing = Some((404, "MailboxNotEnabledForRESTAPI"));
    let pieces = run(&server, &request(SHARED, Vec::new()));
    assert_eq!(pieces[0].refused, Some(SharedRefusal::NotThroughGraph), "{:?}", pieces[0].errors);
    assert!(pieces[0].errors.iter().any(|e| e.contains("MailboxNotEnabledForRESTAPI")), "{:?}", pieces[0].errors);
    // A 403 that is a quota, or a consent, is no "no access": the mailbox isn't left alone for it.
    for code in ["ErrorQuotaExceeded", "Authorization_RequestDenied"] {
        server.state().other(SHARED).unwrap().refusing = Some((403, code));
        let pieces = run(&server, &request(SHARED, Vec::new()));
        assert!(pieces[0].refused.is_none() && !pieces[0].errors.is_empty() && pieces[0].listing.is_none(), "{code}: {:?}", pieces[0]);
    }
}

#[test]
fn nothing_meant_for_a_shared_mailbox_reaches_the_owners_mailbox_or_goes_up() {
    let server = StandIn::start().unwrap();
    server.state().add_other(SHARED, true);
    let theirs = other_folder(&server, "inbox");
    let ids = server.state().other(SHARED).unwrap().mailbox.generate(&theirs, 3, NOW);
    // The owner's mailbox has the same ids: a change sent to `/me` would land on its messages.
    let mine = server.state().well_known("inbox");
    server.state().mailbox.generate(&mine, 3, NOW);
    let own_before: Vec<(String, bool, String)> = {
        let state = server.state();
        let mut all: Vec<_> = state.mailbox.messages.iter().map(|(id, m)| (id.clone(), m.read, format!("{m:?}"))).collect();
        all.sort();
        all
    };
    let deleted = other_folder(&server, "deleteditems");
    // A request that asks for every kind of change (the engine asks for none of them for a
    // shared mailbox): a folder, a flag, a move, a draft and a message to send.
    let mut asked = request(SHARED, Vec::new());
    asked.changes = vec![FolderChange::Create { folder: 70, name: "Ours".into(), parent: String::new(), parent_folder: None }];
    asked.messages = vec![
        MessageChange::Update { message: 1, id: ids[0].clone(), read: Some(true), flag: None, categories: None, importance: None },
        MessageChange::Move { message: 2, id: ids[1].clone(), to: deleted.clone(), to_folder: None },
    ];
    asked.drafts = vec![DraftUp { message: 3, marker: "mailcraft-70".into(), subject: Some("Draft".into()), ..Default::default() }];
    asked.sends = vec![SendUp { message: 4, ..Default::default() }];
    let pieces = run(&server, &asked);
    let all = pieces.iter().fold(GraphMailResult::default(), |mut all, piece| {
        all.absorb(piece.clone());
        all
    });
    assert!(all.changes.is_empty() && all.messages.is_empty() && all.drafts.is_empty() && all.sent.is_empty(), "{all:?}");
    assert!(all.errors.iter().any(|e| e.contains("waits on this computer")), "{:?}", all.errors);
    assert!(all.listing.is_some(), "it is still read");
    for r in server.state().requests.iter() {
        let path = r.path.join("/");
        assert!(!path.starts_with("v1.0/me") && !path.contains("/me/"), "{} {path}", r.method);
        assert!(r.method == "GET" || path == "v1.0/$batch", "nothing is written: {} {path}", r.method);
    }
    // And the batches only read.
    assert!(
        server
            .state()
            .requests
            .iter()
            .filter(|r| r.path.join("/") == "v1.0/$batch")
            .all(|r| !r.body.contains("\"PATCH\"") && !r.body.contains("\"DELETE\"") && !r.body.contains("/me/"))
    );
    let own_after: Vec<(String, bool, String)> = {
        let state = server.state();
        let mut all: Vec<_> = state.mailbox.messages.iter().map(|(id, m)| (id.clone(), m.read, format!("{m:?}"))).collect();
        all.sort();
        all
    };
    assert_eq!(own_before, own_after, "the owner's messages are as they were");
}

#[test]
fn an_address_that_isnt_one_is_never_sent() {
    let server = StandIn::start().unwrap();
    for hostile in ["support@example.org/../../me", "../me@example.org", "a@example.org?$expand=x", "a@example.org#x", "", "me", "a\r\n@example.org"]
    {
        let pieces = run(&server, &request(hostile, Vec::new()));
        assert!(pieces.len() == 1 && !pieces[0].errors.is_empty() && pieces[0].listing.is_none(), "{hostile:?}");
        let fetch = GraphFetchRequest { account: 4, email: hostile.into(), shared: true, ids: vec!["AAkA".into()], background: false };
        let got = super::fetch::fetch_messages_with(&server.url(), TOKEN, &fetch, Arc::new(Gate::new()), QUICK, &AtomicBool::new(false));
        assert!(got.fetched.is_empty() && !got.errors.is_empty());
    }
    assert!(server.state().requests.is_empty(), "nothing at all went to the server");
    // What a good one becomes.
    assert_eq!(shared_mailbox_path("Team.Sales+EU@Mail.Example.org").unwrap(), "/users/Team.Sales%2BEU@Mail.Example.org");
}

proptest::proptest! {
    /// Whatever the address, what goes into a path is one piece of it under `/users/`, or nothing.
    #[test]
    fn a_shared_mailboxs_path_is_one_piece_or_nothing(address in "\\PC{0,80}|[a-z.+'/?#%@-]{1,30}@[a-z.-]{1,20}") {
        if let Ok(path) = shared_mailbox_path(&address) {
            let rest = path.strip_prefix("/users/").unwrap_or("/");
            proptest::prop_assert!(!rest.is_empty());
            proptest::prop_assert!(rest.bytes().all(|b| b.is_ascii_alphanumeric() || b"-._~%@".contains(&b)), "{}", path);
            proptest::prop_assert_eq!(rest.matches('@').count(), 1);
        }
    }
}

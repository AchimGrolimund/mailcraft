//! Mail to keep offline (#143), from end to end against the strict Graph stand-in: a Microsoft
//! account's mail within its window is fetched whole in the background, older mail keeps its
//! headers and preview and is fetched when it is opened. Nothing here leaves this machine; every
//! message is invented.
#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::AtomicBool;

use mailcraft_engine::Session;
use mailcraft_engine::cmd::graph_mail::{apply_graph_fetch, apply_graph_mail, graph_fetch_request, graph_mail_request};
use mailcraft_model::{Id, Mailbox, Message};
use mailcraft_net::graph::standin::{StandIn, TOKEN};
use serde_json::{Value, json};

/// Friday 2027-01-15 08:00 UTC.
const NOW: i64 = 1_800_000_000;
const DAY: i64 = 86_400;

struct Rig {
    server: StandIn,
    s: Session,
    account: Id,
}

impl Rig {
    /// A Microsoft account keeping `keep` offline, and a server whose Inbox has a message
    /// received each of `days_ago`; their ids there.
    fn new(keep: &str, days_ago: &[i64]) -> (Rig, Vec<String>) {
        let server = StandIn::start().unwrap();
        let mut s = Session::at(Mailbox::with_local_account("", ""), NOW, 0);
        let account = s.run("account.add", &json!({"email": "offline@example.org", "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
        s.run("account.edit", &json!({"id": account, "keepOffline": keep})).unwrap();
        let ids = {
            let mut state = server.state();
            let inbox = state.well_known("inbox");
            days_ago.iter().map(|d| state.mailbox.add(&inbox, &format!("{d} days ago"), NOW - d * DAY)).collect()
        };
        let mut rig = Rig { server, s, account };
        rig.run();
        (rig, ids)
    }

    fn run(&mut self) {
        let request = graph_mail_request(&self.s, self.account).unwrap();
        let _routed = self.server.route();
        let s = &mut self.s;
        mailcraft_net::graph::sync_mail(TOKEN, &request, &AtomicBool::new(false), &mut |piece| {
            apply_graph_mail(s, piece).unwrap();
        });
    }

    /// The background's next batch, or the message the owner opened: what was asked for, and
    /// what taking it in said.
    fn fetch(&mut self, message: Option<Id>) -> (Vec<String>, Value) {
        let request = graph_fetch_request(&self.s, self.account, message, 100).unwrap();
        let asked = request.ids.clone();
        let _routed = self.server.route();
        let result = mailcraft_net::graph::fetch_messages(TOKEN, &request, &AtomicBool::new(false));
        (asked, apply_graph_fetch(&mut self.s, result).unwrap())
    }

    fn here(&self, id: &str) -> &Message {
        self.s.mailbox.messages.values().find(|m| m.server.as_ref().is_some_and(|s| s.id == id)).unwrap()
    }
}

#[test]
fn the_background_fetches_mail_within_the_window_and_opening_older_mail_fetches_it() {
    let (mut rig, ids) = Rig::new("1m", &[1, 10, 40, 400]);
    assert!(ids.iter().all(|id| rig.here(id).unfetched), "headers and preview first");
    let (asked, said) = rig.fetch(None);
    assert_eq!(asked, [ids[0].clone(), ids[1].clone()], "within a month, newest first");
    assert_eq!((said["waiting"].as_u64(), said["all"].as_u64()), (Some(0), Some(2)), "{said}");
    assert!(!rig.here(&ids[0]).unfetched && !rig.here(&ids[1]).unfetched);
    assert!(rig.here(&ids[2]).unfetched && rig.here(&ids[3]).unfetched);
    assert!(!rig.here(&ids[3]).body.text.is_empty(), "its preview");
    let (asked, _) = rig.fetch(None);
    assert!(asked.is_empty(), "nothing more in the background: {asked:?}");

    // Opened: an older message is fetched whole all the same.
    let old = rig.here(&ids[3]).id;
    let (asked, _) = rig.fetch(Some(old));
    assert_eq!(asked, [ids[3].clone()]);
    assert!(!rig.here(&ids[3]).unfetched);

    // Longer: the newly covered mail comes in the background, newest first.
    rig.s.run("account.edit", &json!({"id": rig.account, "keepOffline": "2y"})).unwrap();
    let (asked, _) = rig.fetch(None);
    assert_eq!(asked, [ids[2].clone()]);
}

#[test]
fn a_shared_mailbox_fetches_as_far_back_as_its_owner_keeps() {
    let (mut rig, _) = Rig::new("1w", &[]);
    rig.s.shared_mailboxes = true;
    let shared =
        rig.s.run("account.addSharedMailbox", &json!({"account": rig.account, "address": "support@example.org"})).unwrap()["id"].as_u64().unwrap();
    let inbox = rig.s.mailbox.folder_of_kind(shared, mailcraft_model::FolderKind::Inbox).unwrap();
    rig.s.mailbox.folder_mut(inbox).unwrap().server = Some(mailcraft_model::ServerFolder { id: "in".into(), ..Default::default() });
    for (n, days) in [(1, 2), (2, 20)] {
        let server = mailcraft_model::sync::ServerMessage { id: format!("S{n}"), folder: "in".into(), ..Default::default() };
        let m = Message { account: shared, folder: inbox, received: NOW - days * DAY, unfetched: true, server: Some(server), ..Default::default() };
        rig.s.mailbox.add_message(m).unwrap();
    }
    let ids = |s: &Session| graph_fetch_request(s, shared, None, 100).unwrap().ids;
    assert_eq!(ids(&rig.s), ["S1"], "a week, as its owner keeps");
    rig.s.run("account.edit", &json!({"id": rig.account, "keepOffline": "1m"})).unwrap();
    assert_eq!(ids(&rig.s), ["S1", "S2"]);
}

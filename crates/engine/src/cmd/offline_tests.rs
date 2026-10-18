//! Mail to keep offline (#143): the setting, which mail is within it, the background fetch
//! within it, and mail outside it going back to its headers and preview.

use mailcraft_model::{Id, KeepOffline};
use serde_json::json;

use crate::Session;

/// A session with a local account and a Microsoft account; their ids.
fn session() -> (Session, Id, Id) {
    let mut s = Session::empty();
    s.shared_mailboxes = true;
    let local = s.run("account.add", &json!({"email": "ada@example.org"})).unwrap()["id"].as_u64().unwrap();
    let owner = s.run("account.add", &json!({"email": "owner@example.org", "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
    (s, local, owner)
}

#[test]
fn a_new_account_keeps_a_year_and_account_edit_changes_it() {
    let (mut s, _, owner) = session();
    assert_eq!(s.mailbox.account(owner).unwrap().keep_offline, KeepOffline::Months12);
    s.run("account.edit", &json!({"id": owner, "keepOffline": "3m"})).unwrap();
    assert_eq!(s.mailbox.account(owner).unwrap().keep_offline, KeepOffline::Months3);
    let listed = s.run("account.list", &json!({})).unwrap();
    let mine = listed["accounts"].as_array().unwrap().iter().find(|a| a["id"] == owner).unwrap().clone();
    assert_eq!(mine["keepOffline"], "3m");
    for nonsense in [json!("forever"), json!(3), json!(null), json!(""), json!("12 months")] {
        let before = s.mailbox.account(owner).unwrap().keep_offline;
        let r = s.run("account.edit", &json!({"id": owner, "keepOffline": nonsense}));
        assert!(r.is_err() || !nonsense.is_string(), "{nonsense}");
        assert_eq!(s.mailbox.account(owner).unwrap().keep_offline, before, "{nonsense}");
    }
    // Undo puts the setting back.
    s.run("account.edit", &json!({"id": owner, "keepOffline": "all"})).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.mailbox.account(owner).unwrap().keep_offline, KeepOffline::Months3);
}

#[test]
fn a_shared_mailbox_has_no_setting_of_its_own() {
    let (mut s, _, owner) = session();
    let shared = s.run("account.addSharedMailbox", &json!({"account": owner, "address": "support@example.org"})).unwrap()["id"].as_u64().unwrap();
    s.run("account.edit", &json!({"id": owner, "keepOffline": "1w"})).unwrap();
    assert_eq!(s.mailbox.keep_offline_of(shared), KeepOffline::Week1);
    let refused = s.run("account.edit", &json!({"id": shared, "keepOffline": "all"})).unwrap_err().to_string();
    assert!(refused.contains("opened through"), "{refused}");
    let listed = s.run("account.list", &json!({})).unwrap();
    let it = listed["accounts"].as_array().unwrap().iter().find(|a| a["id"] == shared).unwrap().clone();
    assert_eq!(it["keepOffline"], "1w", "as its owner's");
}

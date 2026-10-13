//! The Favorites of the folder pane when an account's server has a list of its own (the folders
//! Outlook shows under Favorites): what `sync.applyFavorites` makes of a download, what the pane
//! then lists, what `folder.favorite` says, and what a mailbox file holds of it.

use mailcraft_model::remote::{FavoritesDownload, FavoritesOutcome, MAX_FAVORITES, RemoteFavorite};
use mailcraft_model::{FolderKind, Id, Mailbox};
use proptest::prelude::*;
use serde_json::{Value, json};

use super::folder::{FROM_OUTLOOK, favorites, favorites_from_server};
use super::remote::apply_favorites;
use crate::{CmdError, Session};

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A mailbox with the owner's local account and an account that signs in with Microsoft, which
/// has folders in its Inbox and at the top, and two favourites of the owner's own making.
struct Setup {
    s: Session,
    account: Id,
    inbox: Id,
    sent: Id,
    projects: Id,
    apollo: Id,
    receipts: Id,
    team: Id,
    local_inbox: Id,
}

fn setup() -> Setup {
    let mut s = Session::at(Mailbox::with_local_account("Avery Quinn", "avery@lumen.example"), 1_791_993_600, 0);
    let local = s.mailbox.accounts[0].id;
    let local_inbox = s.mailbox.folder_of_kind(local, FolderKind::Inbox).unwrap();
    let added = run(&mut s, "account.add", json!({"email": "avery@mail.example", "incoming": {"host": "outlook.example.org"}, "auth": "oauth2"}));
    let account = added["id"].as_u64().unwrap();
    let kind = |s: &Session, kind: FolderKind| s.mailbox.folder_of_kind(account, kind).unwrap();
    let (inbox, sent) = (kind(&s, FolderKind::Inbox), kind(&s, FolderKind::Sent));
    let mut folder = |name: &str, parent: Option<Id>| {
        run(&mut s, "folder.new", json!({"name": name, "parent": parent, "account": account}))["id"].as_u64().unwrap()
    };
    let projects = folder("Projects", Some(inbox));
    let apollo = folder("Apollo", Some(projects));
    let receipts = folder("Receipts", None);
    let team = folder("Team Lumen", None);
    // The owner's own favourites of that account: its Inbox (as every account's is) and Apollo.
    run(&mut s, "folder.favorite", json!({"id": apollo, "value": true}));
    Setup { s, account, inbox, sent, projects, apollo, receipts, team, local_inbox }
}

fn favorite(path: &[&str], kind: FolderKind, root: FolderKind) -> RemoteFavorite {
    RemoteFavorite { path: path.iter().map(|name| name.to_string()).collect(), kind, root }
}

fn read(account: Id, folders: Vec<RemoteFavorite>) -> FavoritesDownload {
    FavoritesDownload { account, email: "avery@mail.example".into(), outcome: FavoritesOutcome::Read, folders, skipped: 0, unresolved: 0 }
}

fn marked(s: &Session) -> Vec<Id> {
    s.mailbox.folders.iter().filter(|f| f.favorite).map(|f| f.id).collect()
}

#[test]
fn the_servers_list_is_what_the_pane_shows_and_the_marks_made_here_are_kept() {
    let mut t = setup();
    let own = vec![t.local_inbox, t.inbox, t.apollo];
    assert_eq!((favorites(&t.s), marked(&t.s)), (own.clone(), own.clone()));
    assert!(!favorites_from_server(&t.s, t.account));

    // Outlook's list, in Outlook's order: found by what a folder is first, then by where it is,
    // whatever it is called there and whatever the case; one the mailbox doesn't have is left out.
    let download = read(
        t.account,
        vec![
            favorite(&["RECEIPTS"], FolderKind::Custom, FolderKind::Custom),
            favorite(&["Gesendete Elemente"], FolderKind::Sent, FolderKind::Sent),
            favorite(&["Posteingang", "projects"], FolderKind::Custom, FolderKind::Inbox),
            favorite(&["Nowhere", "At all"], FolderKind::Custom, FolderKind::Custom),
            favorite(&["Posteingang"], FolderKind::Inbox, FolderKind::Inbox),
            favorite(&["Receipts"], FolderKind::Custom, FolderKind::Custom),
            favorite(&[" Team Lumen "], FolderKind::Custom, FolderKind::Custom),
        ],
    );
    let rev = t.s.rev();
    let answer = apply_favorites(&mut t.s, &download).unwrap();
    assert_eq!(
        answer,
        json!({"outcome": "read", "from": "server", "favorites": [t.receipts, t.sent, t.projects, t.inbox, t.team], "absent": 1, "changed": true})
    );
    assert!(t.s.rev() > rev && t.s.dirty);
    assert!(favorites_from_server(&t.s, t.account));
    // That account's list first, in the server's order; the other account's own after it.
    assert_eq!(favorites(&t.s), [t.receipts, t.sent, t.projects, t.inbox, t.team, t.local_inbox]);
    // The marks made here are as they were: kept aside, not lost.
    assert_eq!(marked(&t.s), own);
    let listed = run(&mut t.s, "folder.list", json!({}));
    assert_eq!(listed["favorites"], json!([t.receipts, t.sent, t.projects, t.inbox, t.team, t.local_inbox]));
    let from: Vec<&str> = listed["accounts"].as_array().unwrap().iter().map(|a| a["favoritesFrom"].as_str().unwrap()).collect();
    assert_eq!(from, ["mailbox", "server"]);

    // The list comes from Outlook and is changed there: a folder of that account is not made
    // a favourite or taken off here, and nothing changes for the asking.
    for (id, value) in [(t.apollo, json!(false)), (t.receipts, json!(true)), (t.team, Value::Null), (t.inbox, json!(false))] {
        let params = if value.is_null() { json!({"id": id}) } else { json!({"id": id, "value": value}) };
        assert_eq!(t.s.run("folder.favorite", &params), Err(CmdError::Failed(FROM_OUTLOOK.into())), "{params}");
    }
    assert_eq!(marked(&t.s), own);
    // A folder of an account whose list is its own is marked as ever.
    run(&mut t.s, "folder.favorite", json!({"id": t.local_inbox, "value": false}));
    run(&mut t.s, "folder.favorite", json!({"id": t.local_inbox, "value": true}));

    // The same list again changes nothing; the same folders in another order do.
    let rev = t.s.rev();
    assert_eq!(apply_favorites(&mut t.s, &download).unwrap()["changed"], false);
    assert_eq!(t.s.rev(), rev);

    // The owner would rather have the ones kept here: they are back exactly, and can be changed.
    let shown = run(&mut t.s, "view.favorites", json!({"own": true}));
    assert_eq!(shown["own"], true);
    assert_eq!(
        shown["accounts"],
        json!([{"account": t.s.mailbox.accounts[0].id, "from": "mailbox", "listed": 1}, {"account": t.account, "from": "mailbox", "listed": 2}])
    );
    assert_eq!(favorites(&t.s), own);
    run(&mut t.s, "folder.favorite", json!({"id": t.team, "value": true}));
    assert_eq!(favorites(&t.s), [t.local_inbox, t.inbox, t.apollo, t.team]);
    // And Outlook's again: the server's list was kept meanwhile, and so were the owner's marks.
    let shown = run(&mut t.s, "view.favorites", json!({"own": false}));
    let ids: Vec<u64> = shown["favorites"].as_array().unwrap().iter().map(|f| f["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, [t.receipts, t.sent, t.projects, t.inbox, t.team, t.local_inbox]);
    assert_eq!(shown["favorites"][2]["path"], "Inbox/Projects");
    assert_eq!(marked(&t.s), [t.local_inbox, t.inbox, t.apollo, t.team]);
    assert!(t.s.run("view.favorites", &json!({"own": "yes"})).is_err());
    assert_eq!(run(&mut t.s, "view.favorites", json!({}))["own"], false);
}

#[test]
fn a_favourite_removed_in_outlook_goes_and_a_list_that_cannot_be_had_gives_way_to_the_mailboxs_own() {
    let mut t = setup();
    let own = favorites(&t.s);
    let sent = favorite(&[], FolderKind::Sent, FolderKind::Sent);
    let receipts = favorite(&["Receipts"], FolderKind::Custom, FolderKind::Custom);
    let apollo = favorite(&["Inbox", "Projects", "Apollo"], FolderKind::Custom, FolderKind::Custom);
    apply_favorites(&mut t.s, &read(t.account, vec![sent.clone(), receipts.clone(), apollo.clone()])).unwrap();
    assert_eq!(favorites(&t.s)[..3], [t.sent, t.receipts, t.apollo]);
    // Taken off in Outlook, and put in another order there: so it is here at the next look.
    apply_favorites(&mut t.s, &read(t.account, vec![apollo.clone(), sent.clone()])).unwrap();
    assert_eq!(favorites(&t.s), [t.apollo, t.sent, t.local_inbox]);
    // All of them taken off there: the account has no favourites, which is not "no list".
    apply_favorites(&mut t.s, &read(t.account, Vec::new())).unwrap();
    assert_eq!(favorites(&t.s), [t.local_inbox]);
    assert!(favorites_from_server(&t.s, t.account));

    // It couldn't be read this time: what was kept stays, whatever came along with the word.
    apply_favorites(&mut t.s, &read(t.account, vec![sent.clone()])).unwrap();
    let failed = FavoritesDownload { outcome: FavoritesOutcome::Failed, ..read(t.account, vec![receipts.clone()]) };
    let answer = apply_favorites(&mut t.s, &failed).unwrap();
    assert_eq!((answer["changed"].clone(), answer["from"].clone()), (json!(false), json!("server")));
    assert_eq!(favorites(&t.s), [t.sent, t.local_inbox]);
    // The server has no such list, or it may not be read: the mailbox's own are what there is,
    // exactly as they were, and `folder.favorite` works again.
    for outcome in [FavoritesOutcome::Missing, FavoritesOutcome::NotPermitted] {
        apply_favorites(&mut t.s, &read(t.account, vec![sent.clone()])).unwrap();
        let gone = FavoritesDownload { outcome, ..read(t.account, vec![receipts.clone()]) };
        let answer = apply_favorites(&mut t.s, &gone).unwrap();
        assert_eq!((answer["changed"].clone(), answer["from"].clone(), answer["favorites"].clone()), (json!(true), json!("mailbox"), json!([])));
        assert_eq!(favorites(&t.s), own, "{outcome:?}");
        assert!(t.s.mailbox.favorites_from_server.is_empty());
        // (Nothing to drop the second time.)
        assert_eq!(apply_favorites(&mut t.s, &gone).unwrap()["changed"], false);
    }
    run(&mut t.s, "folder.favorite", json!({"id": t.receipts, "value": true}));
    run(&mut t.s, "folder.favorite", json!({"id": t.receipts, "value": false}));
    assert_eq!(favorites(&t.s), own);
}

#[test]
fn a_download_is_taken_in_for_its_own_account_only() {
    let mut t = setup();
    let own = favorites(&t.s);
    let list = vec![favorite(&["Receipts"], FolderKind::Custom, FolderKind::Custom)];
    // For an account that isn't there, or for another address than the account's (the mailbox
    // was replaced while the read was under way): nobody's.
    assert!(apply_favorites(&mut t.s, &read(999, list.clone())).is_err());
    let other = FavoritesDownload { email: "somebody@else.example".into(), ..read(t.account, list.clone()) };
    assert!(apply_favorites(&mut t.s, &other).is_err());
    assert_eq!(favorites(&t.s), own);
    // A folder of that name in another account is not that account's favourite.
    let local = t.s.mailbox.accounts[0].id;
    let stray = run(&mut t.s, "folder.new", json!({"name": "Strays", "parent": null, "account": local}))["id"].as_u64().unwrap();
    let answer = apply_favorites(&mut t.s, &read(t.account, vec![favorite(&["Strays"], FolderKind::Custom, FolderKind::Custom)])).unwrap();
    assert_eq!((answer["absent"].clone(), answer["favorites"].clone()), (json!(1), json!([])));
    assert!(!favorites(&t.s).contains(&stray));
    // As a command, with the download as JSON; what isn't one is refused in a few words.
    let answer = run(
        &mut t.s,
        "sync.applyFavorites",
        json!({"download": {"account": t.account, "outcome": "read", "folders": [{"path": ["Inbox", "Projects"], "kind": "custom", "root": "inbox"}, {"kind": "deleted"}]}}),
    );
    let deleted = t.s.mailbox.folder_of_kind(t.account, FolderKind::Deleted).unwrap();
    assert_eq!(answer["favorites"], json!([t.projects, deleted]));
    for odd in
        [json!({}), json!({"download": 7}), json!({"download": {"account": t.account, "outcome": "sideways"}}), json!({"download": {"folders": "x"}})]
    {
        assert!(t.s.run("sync.applyFavorites", &odd).is_err(), "{odd}");
    }
    // Without an outcome nothing was read, and nothing changes.
    let before = favorites(&t.s);
    assert_eq!(run(&mut t.s, "sync.applyFavorites", json!({"download": {"account": t.account}}))["changed"], false);
    assert_eq!(favorites(&t.s), before);

    // A folder that goes takes its place in the list along; so does an account.
    run(&mut t.s, "folder.delete", json!({"id": t.projects}));
    run(&mut t.s, "folder.delete", json!({"id": t.projects}));
    assert_eq!(t.s.mailbox.server_favorites(t.account), Some(vec![deleted]));
    run(&mut t.s, "account.remove", json!({"id": t.account}));
    assert!(t.s.mailbox.favorites_from_server.is_empty());
    assert_eq!(favorites(&t.s), [t.local_inbox]);
}

#[test]
fn an_old_mailbox_file_reads_as_before_and_nothing_new_is_written_until_there_is_a_list() {
    let mut t = setup();
    // A mailbox as every one was before this: its file says nothing of a server's Favorites.
    let old = crate::io::write_mailbox(&t.s.mailbox).unwrap();
    let text = String::from_utf8(old.clone()).unwrap();
    assert!(!text.contains("favoritesFromServer") && !text.to_lowercase().contains("favorites\""), "{text}");
    let opened = crate::io::read_mailbox(&old).unwrap();
    assert_eq!(opened, t.s.mailbox);
    // Opened and saved again, it is the same file to the byte.
    assert_eq!(crate::io::write_mailbox(&opened).unwrap(), old);
    // Nor do the settings say anything new until the owner changes them.
    let prefs = serde_json::to_string(&t.s.prefs).unwrap();
    assert!(!prefs.contains("ownFavorites"), "{prefs}");
    let before: crate::Prefs = serde_json::from_str(r#"{"twelveHour": false}"#).unwrap();
    assert!(!before.own_favorites);

    // A list that couldn't be had writes nothing either.
    for outcome in [FavoritesOutcome::Missing, FavoritesOutcome::NotPermitted, FavoritesOutcome::Failed] {
        apply_favorites(&mut t.s, &FavoritesDownload { outcome, ..read(t.account, Vec::new()) }).unwrap();
        assert_eq!(crate::io::write_mailbox(&t.s.mailbox).unwrap(), old, "{outcome:?}");
    }
    // Once there is one, it is written, and read back as it was; the marks made here beside it.
    apply_favorites(&mut t.s, &read(t.account, vec![favorite(&["Receipts"], FolderKind::Custom, FolderKind::Custom)])).unwrap();
    let new = String::from_utf8(crate::io::write_mailbox(&t.s.mailbox).unwrap()).unwrap();
    assert!(new.contains(&format!(r#""favoritesFromServer":[{{"account":{},"folders":[{}]}}]"#, t.account, t.receipts)), "{new}");
    let opened = crate::io::read_mailbox(new.as_bytes()).unwrap();
    assert_eq!(opened, t.s.mailbox);
    assert_eq!(opened.server_favorites(t.account), Some(vec![t.receipts]));
    run(&mut t.s, "view.favorites", json!({"own": true}));
    assert!(serde_json::to_string(&t.s.prefs).unwrap().contains(r#""ownFavorites":true"#));

    // A file may say anything: a list for nobody, folders of another account, the same folder
    // over and over, numbers out of all range, or no list at all where one should be.
    let mut json: Value = serde_json::from_str(&new).unwrap();
    for (hostile, want) in [
        (json!([{"account": 999_999, "folders": [t.receipts]}]), None),
        (
            json!([{"account": t.account, "folders": [t.local_inbox, t.receipts, 0, 18_446_744_073_709_551_615u64, t.receipts, t.inbox]}]),
            Some(vec![t.receipts, t.inbox]),
        ),
        (
            json!([{"account": t.account, "folders": [t.inbox]}, {"account": t.account, "folders": [t.sent]}, 7, "x", {"folders": "none"}]),
            Some(vec![t.inbox]),
        ),
        (json!("not a list"), None),
        (json!({"account": t.account}), None),
        (json!([{"account": t.account, "folders": (0..5_000u64).collect::<Vec<_>>()}]), None),
    ] {
        json["favoritesFromServer"] = hostile.clone();
        let opened = crate::io::read_mailbox(json.to_string().as_bytes()).unwrap();
        let listed = opened.server_favorites(t.account);
        match want {
            Some(want) => assert_eq!(listed, Some(want), "{hostile}"),
            // (No list, or a list of which nothing is a folder of the account.)
            None => {
                assert!(listed.is_none_or(|listed| listed.iter().all(|id| opened.folder(*id).is_some_and(|f| f.account == t.account))), "{hostile}")
            }
        }
        assert!(opened.favorites_from_server.len() <= 1 && opened.favorites_from_server.iter().all(|list| list.folders.len() <= MAX_FAVORITES));
        // And the pane has something to list whatever it said.
        let s = Session::at(opened, 1_791_993_600, 0);
        assert!(favorites(&s).iter().all(|id| s.mailbox.folder(*id).is_some()));
    }
}

fn any_favorite() -> impl Strategy<Value = RemoteFavorite> {
    let kind = prop_oneof![
        Just(FolderKind::Custom),
        Just(FolderKind::Custom),
        Just(FolderKind::Inbox),
        Just(FolderKind::Sent),
        Just(FolderKind::Deleted),
        Just(FolderKind::Outbox),
    ];
    let name = prop_oneof![
        Just("Inbox".to_string()),
        Just("projects".to_string()),
        Just("Receipts".to_string()),
        Just("Apollo".to_string()),
        "\\PC{0,12}",
        ".{0,300}"
    ];
    (proptest::collection::vec(name, 0..40), kind.clone(), kind).prop_map(|(path, kind, root)| RemoteFavorite { path, kind, root })
}

proptest! {
    #[test]
    fn whatever_a_download_says_the_list_is_folders_of_its_account_each_once(
        folders in proptest::collection::vec(any_favorite(), 0..60),
        outcome in prop_oneof![Just(FavoritesOutcome::Read), Just(FavoritesOutcome::Missing), Just(FavoritesOutcome::NotPermitted), Just(FavoritesOutcome::Failed)],
        own in any::<bool>(),
    ) {
        let mut t = setup();
        let marks = marked(&t.s);
        t.s.prefs.own_favorites = own;
        let download = FavoritesDownload { outcome, ..read(t.account, folders) };
        let answer = apply_favorites(&mut t.s, &download).unwrap();
        let listed = favorites(&t.s);
        let mut seen = std::collections::HashSet::new();
        for id in &listed {
            prop_assert!(t.s.mailbox.folder(*id).is_some() && seen.insert(*id));
        }
        if let Some(list) = t.s.mailbox.server_favorites(t.account) {
            prop_assert!(outcome == FavoritesOutcome::Read && list.len() <= MAX_FAVORITES);
            prop_assert!(list.iter().all(|id| t.s.mailbox.folder(*id).is_some_and(|f| f.account == t.account)));
        }
        prop_assert_eq!(answer["from"] == "server", outcome == FavoritesOutcome::Read && !own);
        // The owner's own marks are never touched by any of it.
        prop_assert_eq!(marked(&t.s), marks);
    }
}

/// 31 March 2027 and 1 April 2027, noon UTC: the last day Exchange Web Services is there, and
/// the first day it isn't.
const LAST_DAY: i64 = 1_806_494_400;
const FIRST_DAY_WITHOUT: i64 = LAST_DAY + 86_400;

#[test]
fn the_owner_chooses_how_outlooks_favorites_are_read_and_nothing_is_written_until_then() {
    use mailcraft_model::{Directory, FavoritesSource};
    let mut t = setup();
    // Never chosen: nothing of it is in the mailbox file, and the answer says so.
    let before = String::from_utf8(crate::io::write_mailbox(&t.s.mailbox).unwrap()).unwrap();
    assert!(!before.contains("favoritesSource") && !before.contains("directory"), "{before}");
    let state = run(&mut t.s, "account.favoritesSource", json!({}));
    assert_eq!(
        state,
        json!({"account": t.account, "source": null, "directory": null, "offered": ["graph", "ews", "none"], "ewsEnds": "2027-04-01", "ewsEnded": false, "from": "mailbox"})
    );
    // Asking is no change: the file is the same, byte for byte, and nothing was noted to save.
    assert_eq!(String::from_utf8(crate::io::write_mailbox(&t.s.mailbox).unwrap()).unwrap(), before);

    // Each of the three, with what kind of account it is when the caller knows.
    let rev = t.s.rev();
    let chosen = run(&mut t.s, "account.favoritesSource", json!({"account": t.account, "source": "ews", "directory": "organisation"}));
    assert_eq!((chosen["source"].clone(), chosen["directory"].clone()), (json!("ews"), json!("organisation")));
    assert!(t.s.rev() > rev && t.s.dirty);
    let account = t.s.mailbox.account(t.account).unwrap();
    assert_eq!((account.favorites_source, account.oauth.as_ref().unwrap().directory), (FavoritesSource::Ews, Directory::Organisation));
    let file = String::from_utf8(crate::io::write_mailbox(&t.s.mailbox).unwrap()).unwrap();
    assert!(file.contains(r#""favoritesSource":"ews""#) || file.contains(r#""favoritesSource": "ews""#), "{file}");
    // It survives the file.
    let reopened = crate::io::read_mailbox(file.as_bytes()).unwrap();
    assert_eq!(reopened.account(t.account).unwrap().favorites_source, FavoritesSource::Ews);
    // It is a setting of the account like its others: undone and redone as one step.
    run(&mut t.s, "edit.undo", json!({}));
    assert_eq!(t.s.mailbox.account(t.account).unwrap().favorites_source, FavoritesSource::Unset);
    run(&mut t.s, "edit.redo", json!({}));
    assert_eq!(t.s.mailbox.account(t.account).unwrap().favorites_source, FavoritesSource::Ews);
    assert_eq!(run(&mut t.s, "account.favoritesSource", json!({"source": "graph"}))["source"], "graph");
    assert_eq!(run(&mut t.s, "account.list", json!({}))["accounts"][1]["favoritesSource"], "graph");

    // "Not now": what was kept of the server's list goes, the marks made here count again, and
    // a read that was under way brings nothing.
    apply_favorites(&mut t.s, &read(t.account, vec![favorite(&["Receipts"], FolderKind::Custom, FolderKind::Custom)])).unwrap();
    assert_eq!(favorites(&t.s), [t.receipts, t.local_inbox]);
    let none = run(&mut t.s, "account.favoritesSource", json!({"account": t.account, "source": "none"}));
    assert_eq!((none["source"].clone(), none["from"].clone()), (json!("none"), json!("mailbox")));
    assert_eq!(favorites(&t.s), [t.local_inbox, t.inbox, t.apollo]);
    let late = apply_favorites(&mut t.s, &read(t.account, vec![favorite(&["Receipts"], FolderKind::Custom, FolderKind::Custom)])).unwrap();
    assert_eq!((late["from"].clone(), late["changed"].clone()), (json!("mailbox"), json!(false)));
    assert!(!favorites_from_server(&t.s, t.account));
    // And the folders can be marked here again.
    run(&mut t.s, "folder.favorite", json!({"id": t.receipts, "value": true}));

    // What it can't be, and whose it can't be.
    for odd in [json!({"source": "imap"}), json!({"source": 7}), json!({"source": ""}), json!({"directory": "elsewhere"}), json!({"account": "x"})] {
        assert!(matches!(t.s.run("account.favoritesSource", &odd), Err(CmdError::Params(_))), "{odd}");
    }
    let local = t.s.mailbox.accounts[0].id;
    assert!(t.s.run("account.favoritesSource", &json!({"account": local, "source": "graph"})).is_err());
    assert!(t.s.run("account.favoritesSource", &json!({"account": 9_999, "source": "graph"})).is_err());
    // A personal Microsoft account has Microsoft Graph and nothing else.
    let personal = t.s.run("account.favoritesSource", &json!({"account": t.account, "source": "ews", "directory": "personal"}));
    assert!(personal.unwrap_err().to_string().contains("personal"));
    assert_eq!(run(&mut t.s, "account.favoritesSource", json!({"directory": "personal"}))["offered"], json!(["graph", "none"]));
    // (Saying what kind it is without choosing changes nothing.)
    assert_eq!(t.s.mailbox.account(t.account).unwrap().oauth.as_ref().unwrap().directory, Directory::Organisation);
}

#[test]
fn exchange_web_services_is_not_offered_from_the_day_microsoft_switches_it_off() {
    use mailcraft_model::FavoritesSource;
    let mailbox = setup().s.mailbox;
    let account = mailbox.accounts[1].id;
    // The day before: offered, and taken.
    let mut s = Session::at(mailbox.clone(), LAST_DAY, 0);
    let state = run(&mut s, "account.favoritesSource", json!({"source": "ews"}));
    assert_eq!(
        (state["ewsEnded"].clone(), state["offered"].clone(), state["source"].clone()),
        (json!(false), json!(["graph", "ews", "none"]), json!("ews"))
    );
    assert!(!super::account::ews_ended(&s));
    // That day, by the clock of the session: not offered, and refused with the reason.
    let mut later = Session::at(s.mailbox.clone(), FIRST_DAY_WITHOUT, 0);
    assert!(super::account::ews_ended(&later));
    let state = run(&mut later, "account.favoritesSource", json!({}));
    assert_eq!((state["ewsEnded"].clone(), state["offered"].clone()), (json!(true), json!(["graph", "none"])));
    // (An account that was set to it stays set: it is told, and offered the other way.)
    assert_eq!(state["source"], "ews");
    let refused = later.run("account.favoritesSource", &json!({"source": "ews"})).unwrap_err().to_string();
    assert!(refused.contains("1 April 2027") && refused.contains("Microsoft Graph"), "{refused}");
    assert_eq!(run(&mut later, "account.favoritesSource", json!({"source": "graph"}))["source"], "graph");
    // The day is the one where the session's clock is: still March for somebody west of UTC…
    let west = Session::at(mailbox.clone(), FIRST_DAY_WITHOUT - 43_200 + 3_600, -600);
    assert!(!super::account::ews_ended(&west));
    // …and April already for somebody east of it.
    let east = Session::at(mailbox, LAST_DAY + 3 * 3_600, 600);
    assert!(super::account::ews_ended(&east));
    assert_eq!(account, later.mailbox.accounts[1].id);
    assert_eq!(FavoritesSource::EWS_ENDS, (2027, 4, 1));
}

#[test]
fn a_mailbox_file_that_says_something_else_in_their_place_still_opens() {
    use mailcraft_model::{Directory, FavoritesSource};
    let t = setup();
    let file = String::from_utf8(crate::io::write_mailbox(&t.s.mailbox).unwrap()).unwrap();
    let mut json: Value = serde_json::from_str(&file).unwrap();
    for (source, directory, expected) in [
        (json!("ews"), json!("organisation"), (FavoritesSource::Ews, Directory::Organisation)),
        (json!("graph"), json!("personal"), (FavoritesSource::Graph, Directory::Personal)),
        (json!("none"), json!(null), (FavoritesSource::None, Directory::Unknown)),
        // From a later version, or from anywhere: not said.
        (json!("carrier pigeon"), json!(7), (FavoritesSource::Unset, Directory::Unknown)),
        (json!({"via": "ews"}), json!(["organisation"]), (FavoritesSource::Unset, Directory::Unknown)),
        (json!(null), json!(""), (FavoritesSource::Unset, Directory::Unknown)),
    ] {
        json["accounts"][1]["favoritesSource"] = source.clone();
        json["accounts"][1]["oauth"]["directory"] = directory;
        let mailbox = crate::io::read_mailbox(json.to_string().as_bytes()).unwrap_or_else(|e| panic!("{source}: {e}"));
        let account = mailbox.account(t.account).unwrap();
        assert_eq!((account.favorites_source, account.oauth.as_ref().unwrap().directory), expected, "{source}");
        assert_eq!(mailbox.folders.len(), t.s.mailbox.folders.len());
    }
}

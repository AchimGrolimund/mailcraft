// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A shared mailbox's calendars in the mailbox (#70): they are the shared mailbox's own, listed
//! with it, read and not changed here, never reminded of, and gone with it.

use mailcraft_model::remote::{CalendarsDownload, EventsDownload, RemoteCalendar, RemoteEvent};
use mailcraft_model::{CalendarOwner, Id, Rights, ShowAs};
use serde_json::json;

use super::{apply_calendars, apply_events, ensure_shared_calendars, is_own_calendar, server_calendar_id};
use crate::Session;
use crate::cmd::calendar_settings::calendar_groups;
use crate::cmd::view::due_reminders;

const NOW: i64 = 1_791_993_600;
const OWNER: &str = "owner@example.org";
const SHARED: &str = "support@example.org";

/// An owner's account that signs in with Microsoft, with its own calendars listed, and a shared
/// mailbox opened under it. Their ids.
fn served() -> (Session, Id, Id) {
    let mut s = Session::at(mailcraft_model::Mailbox::with_local_account("", ""), NOW, 0);
    let owner = s.run("account.add", &json!({"email": OWNER, "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
    let all = vec![RemoteCalendar {
        id: "own-main".into(),
        name: "Calendar".into(),
        main: true,
        rights: Rights::Edit,
        owner: Some(CalendarOwner { address: OWNER.into(), name: "Owner".into() }),
        ..Default::default()
    }];
    apply_calendars(&mut s, &CalendarsDownload { account: owner, email: OWNER.into(), complete: true, calendars: all }).unwrap();
    let shared = s.run("account.addSharedMailbox", &json!({"address": SHARED})).unwrap()["id"].as_u64().unwrap();
    (s, owner, shared)
}

fn listing(account: Id, rights: Rights) -> CalendarsDownload {
    let owner = Some(CalendarOwner { address: SHARED.into(), name: "Support".into() });
    let all = vec![
        RemoteCalendar { id: "sh-main".into(), name: "Calendar".into(), main: true, rights, owner: owner.clone(), ..Default::default() },
        RemoteCalendar { id: "sh-rota".into(), name: "Rota".into(), rights, owner, ..Default::default() },
    ];
    CalendarsDownload { account, email: SHARED.into(), complete: true, calendars: all }
}

fn event(id: &str, title: &str, start: i64) -> RemoteEvent {
    RemoteEvent { id: id.into(), title: title.into(), start, end: start + 3_600, reminder_minutes: Some(15), ..Default::default() }
}

fn download(account: Id, email: &str, calendar: &str, events: Vec<RemoteEvent>) -> EventsDownload {
    EventsDownload {
        account,
        email: email.into(),
        calendar: calendar.into(),
        from: NOW - 86_400,
        to: NOW + 7 * 86_400,
        complete: true,
        events,
        ..Default::default()
    }
}

fn calendar_named(s: &Session, account: Id, name: &str) -> mailcraft_model::Calendar {
    s.mailbox.calendars.iter().find(|c| c.origin.account == account && c.name == name).cloned().unwrap_or_else(|| panic!("no {name}"))
}

#[test]
fn a_shared_mailbox_has_a_calendar_to_tick_and_its_own_list_is_unticked_under_my_calendars() {
    let (mut s, owner, shared) = served();
    // Added: its main calendar is there to tick, unticked and read only, named for its address.
    let placeholder = calendar_named(&s, shared, SHARED);
    assert!(!placeholder.visible && placeholder.origin.id.is_empty() && placeholder.rights == Rights::Read);
    assert!(!ensure_shared_calendars(&mut s), "once is enough");
    // Only the account's own main calendar is ticked to begin with.
    assert!(calendar_named(&s, owner, "Calendar").visible);

    // The server lists it: the main one is the placeholder as the server names it, and the rest
    // come unticked.
    apply_calendars(&mut s, &listing(shared, Rights::Edit)).unwrap();
    let mine: Vec<(String, bool)> = s.mailbox.calendars.iter().filter(|c| c.origin.account == shared).map(|c| (c.name.clone(), c.visible)).collect();
    assert_eq!(mine, vec![("Calendar".to_string(), false), ("Rota".to_string(), false)]);
    // Listed under My Calendars with the mailbox's address, not under Shared Calendars.
    let groups: Vec<(String, Vec<String>)> = calendar_groups(&s)
        .iter()
        .filter(|g| g.account != 0)
        .map(|g| (g.title.clone(), g.calendars.iter().map(|id| s.mailbox.calendar(*id).map(|c| c.name.clone()).unwrap_or_default()).collect()))
        .collect();
    assert_eq!(
        groups,
        vec![
            ("My Calendars (owner@example.org)".to_string(), vec!["Calendar".to_string()]),
            ("My Calendars (support@example.org)".to_string(), vec!["Calendar".to_string(), "Rota".to_string()]),
        ]
    );
    // A list of the mailbox's is its own: it doesn't take the owner's calendars away, nor the other way.
    apply_calendars(
        &mut s,
        &CalendarsDownload {
            account: owner,
            email: OWNER.into(),
            complete: true,
            calendars: vec![RemoteCalendar { id: "x".into(), name: "Calendar".into(), main: true, ..Default::default() }],
        },
    )
    .unwrap();
    assert_eq!(s.mailbox.calendars.iter().filter(|c| c.origin.account == shared).count(), 2);
    // A list for the wrong address is not taken in.
    let mut stranger = listing(shared, Rights::Edit);
    stranger.email = "other@example.org".into();
    assert!(apply_calendars(&mut s, &stranger).is_err());
}

#[test]
fn a_shared_mailbox_is_never_where_new_items_go() {
    let (mut s, owner, shared) = served();
    apply_calendars(&mut s, &listing(shared, Rights::Edit)).unwrap();
    let main_of = |account: Id| s.mailbox.calendars.iter().find(|c| c.origin.account == account && c.origin.id.is_empty()).map(|c| c.id);
    assert_eq!(server_calendar_id(&s.mailbox), main_of(owner));
    // Even where the owner's own has not been listed (yet): there is nowhere on a server to put it.
    s.mailbox.calendars.retain(|c| c.origin.account != owner);
    assert_eq!(server_calendar_id(&s.mailbox), None);
}

#[test]
fn what_is_in_a_shared_mailboxs_calendar_is_read_here_and_never_changed() {
    let (mut s, _, shared) = served();
    apply_calendars(&mut s, &listing(shared, Rights::Edit)).unwrap();
    let events = vec![event("e-1", "Stand-up", NOW + 3_600)];
    apply_events(&mut s, &download(shared, SHARED, "", events)).unwrap();
    let ev = s.mailbox.events.values().find(|e| e.title == "Stand-up").cloned().unwrap();
    assert_eq!(ev.origin.account, shared);
    let calendar = ev.calendar;
    let before = s.mailbox.event_changes.clone();
    for (command, params) in [
        ("calendar.edit", json!({"id": ev.id, "title": "Changed"})),
        ("calendar.delete", json!({"id": ev.id})),
        ("calendar.showAs", json!({"id": ev.id, "showAs": "free"})),
        ("calendar.reminder", json!({"id": ev.id, "reminder": 5})),
        ("calendar.private", json!({"id": ev.id, "private": true})),
        ("calendar.new", json!({"title": "New", "calendar": calendar, "start": "2026-10-14T10:00", "end": "2026-10-14T11:00"})),
    ] {
        let said = s.run(command, &params).unwrap_err().to_string();
        assert!(said.contains("shared mailbox"), "{command}: {said}");
    }
    assert_eq!(s.mailbox.event_changes, before, "nothing waits to be sent to a server that was never asked");
    assert_eq!(s.mailbox.event(ev.id).map(|e| e.title.as_str()), Some("Stand-up"));
    // The list says it can only be read.
    let list = s.run("calendar.calendars", &json!({})).unwrap().to_string();
    assert!(list.contains("\"readOnly\":true"));
}

#[test]
fn a_calendar_that_shows_only_when_its_owner_is_busy_is_drawn_without_subjects_and_read_again_at_every_listing() {
    let (mut s, _, shared) = served();
    apply_calendars(&mut s, &listing(shared, Rights::Read)).unwrap();
    let blocks = EventsDownload {
        free_busy: true,
        ..download(shared, SHARED, "", vec![RemoteEvent { show_as: ShowAs::Tentative, ..event("freebusy:1:2:tentative", "", NOW + 7_200) }])
    };
    apply_events(&mut s, &blocks).unwrap();
    assert_eq!(calendar_named(&s, shared, "Calendar").rights, Rights::FreeBusy);
    let block = s.mailbox.events.values().find(|e| e.origin.id.starts_with("freebusy:")).cloned().unwrap();
    assert!(block.title.is_empty() && block.show_as == ShowAs::Tentative);
    let said = s.run("calendar.delete", &json!({"id": block.id})).unwrap_err().to_string();
    assert!(said.contains("only shows when its owner is busy"), "{said}");
    // The list says so.
    let list = s.run("calendar.calendars", &json!({})).unwrap().to_string();
    assert!(list.contains("\"rights\":\"freeBusy\""), "{list}");
    // The owner gets more rights, and the list says it: what the server says now is what stands.
    apply_calendars(&mut s, &listing(shared, Rights::Read)).unwrap();
    apply_events(&mut s, &download(shared, SHARED, "", vec![event("e-9", "Stand-up", NOW + 3_600)])).unwrap();
    assert_eq!(calendar_named(&s, shared, "Calendar").rights, Rights::Read);
    // The blocks are gone with the next complete download that doesn't have them.
    assert!(s.mailbox.events.values().all(|e| !e.origin.id.starts_with("freebusy:")));
}

#[test]
fn only_the_owners_own_calendars_remind() {
    let (mut s, owner, shared) = served();
    apply_calendars(&mut s, &listing(shared, Rights::Edit)).unwrap();
    // A calendar a colleague shares with the owner, which the server lists for the owner.
    let colleague = RemoteCalendar {
        id: "bo".into(),
        name: "Bo".into(),
        rights: Rights::Read,
        owner: Some(CalendarOwner { address: "bo@example.org".into(), name: "Bo".into() }),
        ..Default::default()
    };
    let mut own = CalendarsDownload { account: owner, email: OWNER.into(), complete: true, calendars: vec![colleague] };
    own.calendars
        .insert(0, RemoteCalendar { id: "own-main".into(), name: "Calendar".into(), main: true, rights: Rights::Edit, ..Default::default() });
    apply_calendars(&mut s, &own).unwrap();
    let soon = NOW + 600;
    apply_events(&mut s, &download(owner, OWNER, "", vec![event("mine", "Mine", soon)])).unwrap();
    apply_events(&mut s, &download(owner, OWNER, "bo", vec![event("bos", "Bo's", soon)])).unwrap();
    apply_events(&mut s, &download(shared, SHARED, "", vec![event("sup", "Support's", soon)])).unwrap();
    apply_events(&mut s, &download(shared, SHARED, "sh-rota", vec![event("rota", "Rota", soon)])).unwrap();
    let blocks = EventsDownload {
        free_busy: true,
        ..download(shared, SHARED, "", vec![RemoteEvent { show_as: ShowAs::Busy, ..event("freebusy:3:4:busy", "", soon) }])
    };
    apply_events(&mut s, &blocks).unwrap();
    assert!(s.mailbox.events.values().filter(|e| e.reminder_minutes.is_some() || e.title.is_empty()).count() >= 4);
    let due: Vec<String> = due_reminders(&s, s.local_now()).into_iter().map(|r| r.title).collect();
    assert_eq!(due, ["Mine"], "nothing of a shared mailbox's or of a colleague's");
    let calendars: Vec<(String, bool)> = s.mailbox.calendars.iter().map(|c| (c.name.clone(), is_own_calendar(&s.mailbox, c))).collect();
    assert!(calendars.contains(&("Calendar".to_string(), true)) && calendars.contains(&("Bo".to_string(), false)), "{calendars:?}");
    assert!(s.mailbox.calendars.iter().filter(|c| c.origin.account == shared).all(|c| !is_own_calendar(&s.mailbox, c)));
}

#[test]
fn removing_the_shared_mailbox_removes_its_calendars_and_events_from_this_computer() {
    let (mut s, owner, shared) = served();
    apply_calendars(&mut s, &listing(shared, Rights::Edit)).unwrap();
    apply_events(&mut s, &download(owner, OWNER, "", vec![event("mine", "Mine", NOW + 600)])).unwrap();
    apply_events(&mut s, &download(shared, SHARED, "", vec![event("sup", "Support's", NOW + 600)])).unwrap();
    apply_events(&mut s, &download(shared, SHARED, "sh-rota", vec![event("rota", "Rota", NOW + 600)])).unwrap();
    assert_eq!(s.mailbox.events.values().filter(|e| e.origin.account == shared).count(), 2);
    s.run("account.removeSharedMailbox", &json!({"id": shared})).unwrap();
    assert!(s.mailbox.calendars.iter().all(|c| c.origin.account != shared), "its calendars are gone");
    assert!(s.mailbox.events.values().all(|e| e.origin.account != shared), "and what was in them");
    // The owner's own are as they were.
    assert!(s.mailbox.events.values().any(|e| e.title == "Mine"));
    assert!(s.mailbox.calendars.iter().any(|c| c.origin.account == owner && c.visible));
    // The same when the owner's account goes, with the shared mailboxes under it.
    let again = s.run("account.addSharedMailbox", &json!({"address": SHARED})).unwrap()["id"].as_u64().unwrap();
    apply_calendars(&mut s, &listing(again, Rights::Edit)).unwrap();
    apply_events(&mut s, &download(again, SHARED, "", vec![event("sup", "Support's", NOW + 600)])).unwrap();
    s.run("account.remove", &json!({"id": owner})).unwrap();
    assert!(s.mailbox.calendars.iter().all(|c| c.origin.account != again) && s.mailbox.events.values().all(|e| e.origin.account != again));
}

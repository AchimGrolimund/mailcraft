// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Time zones through the engine: a mailbox from before zones were kept, a computer that is
//! carried to another zone, invitations from elsewhere, and what a server is and is not told.

use mailcraft_model::remote::{EventChangeKind, EventsDownload, RemoteEvent, RemoteEventKind};
use mailcraft_model::{Address, Attachment, Event, FolderKind, Freq, Id, Mailbox, Message, Recurrence};
use mailcraft_time::{Date, DateTime, Timestamp, Weekday, Zone};
use serde_json::{Value, json};

use super::remote::{apply_events, event_upload, events_waiting};
use super::view::due_reminders;
use crate::Session;

fn zone(name: &str) -> Zone {
    Zone::named(name).unwrap()
}

fn clock(y: i32, m: u8, d: u8, h: u8, min: u8) -> DateTime {
    DateTime::new(Date::new(y, m, d).unwrap(), h, min, 0).unwrap()
}

/// An instant, by the clock in UTC.
fn at(y: i32, m: u8, d: u8, h: u8, min: u8) -> Timestamp {
    clock(y, m, d, h, min).to_timestamp(0)
}

/// Monday 2026-10-12, 15:00 UTC: ten in the morning in Chicago, four in the afternoon in London.
const NOW: Timestamp = 1_791_817_200;

/// A calendar of the mailbox's own, which no server keeps.
fn own_calendar(s: &mut Session) -> Id {
    match s.mailbox.calendars.iter().find(|c| c.origin.is_local()) {
        Some(calendar) => calendar.id,
        None => run(s, "calendar.addCalendar", json!({"name": "On this computer"}))["id"].as_u64().unwrap(),
    }
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

/// A mailbox as one written before zones were kept: an account that signs in with Microsoft and
/// its calendar as a download left it (a meeting, a whole-day event, a weekly series with a day
/// skipped), an appointment of the owner's own, and a change to the meeting that waits to be
/// sent. Its times are Chicago's, where it was written, and it says nothing of zones.
fn old_mailbox() -> (Mailbox, Id) {
    let chicago = zone("America/Chicago");
    let mut s = Session::in_zone(Mailbox::with_local_account("", ""), NOW, chicago);
    let account =
        run(&mut s, "account.add", json!({"email": "ada@example.org", "incoming": {"host": "outlook.example.org"}, "auth": "oauth2"}))["id"]
            .as_u64()
            .unwrap();
    apply_events(&mut s, &october(account, as_the_server_has_it(false))).unwrap();
    let meeting = s.mailbox.events.values().find(|e| e.origin.id == "meeting").unwrap().id;
    run(&mut s, "calendar.move", json!({"id": meeting, "start": "2026-10-13T10:00"}));
    let own = own_calendar(&mut s);
    run(&mut s, "calendar.new", json!({"title": "Dentist", "start": "2026-10-15T16:30", "end": "2026-10-15T17:30", "calendar": own}));
    assert_eq!(s.mailbox.event_changes.len(), 1);
    // As the file had it: no zone anywhere.
    let mut mailbox = s.mailbox.clone();
    mailbox.zone.clear();
    for event in mailbox.events.values_mut() {
        let event = std::sync::Arc::make_mut(event);
        (event.start_zone, event.end_zone) = (String::new(), String::new());
    }
    let text = serde_json::to_string(&mailbox).unwrap();
    assert!(!text.to_lowercase().contains("zone"), "{text}");
    (serde_json::from_str(&text).unwrap(), account)
}

/// What the server has in October 2026. `zones`: whether it says which zones the events were
/// made in (a server that has always been asked for them does).
fn as_the_server_has_it(zones: bool) -> Vec<RemoteEvent> {
    let named = |name: &str| if zones { name.to_string() } else { String::new() };
    let rule = Recurrence { by_day: vec![Weekday::Mon], ..Recurrence::every(Freq::Weekly) };
    vec![
        // Tuesday at nine in Chicago.
        RemoteEvent {
            id: "meeting".into(),
            change_key: "ck-1".into(),
            title: "Review".into(),
            start: at(2026, 10, 13, 14, 0),
            end: at(2026, 10, 13, 15, 0),
            start_zone: named("Central Standard Time"),
            end_zone: named("Central Standard Time"),
            reminder_minutes: Some(15),
            ..Default::default()
        },
        RemoteEvent {
            id: "day".into(),
            change_key: "ck-1".into(),
            title: "Holiday".into(),
            start: at(2026, 10, 14, 0, 0),
            end: at(2026, 10, 15, 0, 0),
            all_day: true,
            ..Default::default()
        },
        // Mondays at half past ten at night in Chicago, which is Tuesday in UTC; the 19th is skipped.
        RemoteEvent {
            id: "series".into(),
            change_key: "ck-1".into(),
            title: "Late call".into(),
            start: at(2026, 10, 6, 3, 30),
            end: at(2026, 10, 6, 4, 0),
            kind: RemoteEventKind::SeriesMaster,
            recurrence: Some(rule),
            series_day: Date::new(2026, 10, 5),
            start_zone: named("Central Standard Time"),
            ..Default::default()
        },
        RemoteEvent {
            id: "o-1".into(),
            series: "series".into(),
            kind: RemoteEventKind::Occurrence,
            title: "Late call".into(),
            start: at(2026, 10, 6, 3, 30),
            end: at(2026, 10, 6, 4, 0),
            start_zone: named("Central Standard Time"),
            ..Default::default()
        },
        RemoteEvent {
            id: "o-2".into(),
            series: "series".into(),
            kind: RemoteEventKind::Occurrence,
            title: "Late call".into(),
            start: at(2026, 10, 13, 3, 30),
            end: at(2026, 10, 13, 4, 0),
            start_zone: named("Central Standard Time"),
            ..Default::default()
        },
        RemoteEvent {
            id: "o-4".into(),
            series: "series".into(),
            kind: RemoteEventKind::Occurrence,
            title: "Late call".into(),
            start: at(2026, 10, 27, 3, 30),
            end: at(2026, 10, 27, 4, 0),
            start_zone: named("Central Standard Time"),
            ..Default::default()
        },
    ]
}

fn october(account: Id, events: Vec<RemoteEvent>) -> EventsDownload {
    EventsDownload {
        account,
        email: "ada@example.org".into(),
        from: at(2026, 10, 1, 0, 0),
        to: at(2026, 11, 1, 0, 0),
        complete: true,
        events,
        ..Default::default()
    }
}

/// When every timed event of the mailbox is, by its id.
fn instants(s: &Session) -> Vec<(Id, Timestamp, Timestamp)> {
    let here = super::remote::offset_of(s);
    s.mailbox.events.values().filter(|e| !e.all_day).map(|e| (e.id, e.instants(&here).0, e.instants(&here).1)).collect()
}

fn by_remote(s: &Session, remote: &str) -> Event {
    s.mailbox.events.values().find(|e| e.origin.id == remote).map(|e| e.as_ref().clone()).unwrap_or_else(|| panic!("no event {remote}"))
}

#[test]
fn a_mailbox_from_before_zones_opens_as_it_was_and_sends_nothing() {
    let (old, account) = old_mailbox();
    let mut s = Session::in_zone(old.clone(), NOW, zone("America/Chicago"));
    // It is the same mailbox, but that it now says which zone its times are in. Opening it
    // made nothing to save and nothing to undo.
    assert_eq!(s.mailbox.zone, "America/Chicago");
    let mut unstamped = s.mailbox.clone();
    unstamped.zone.clear();
    assert_eq!(unstamped, old);
    assert!(!s.dirty && !s.can_undo());
    // What waited for the server before is what waits now, and it says what it said: the
    // meeting moved to ten, as instants.
    let waiting = events_waiting(&s, account);
    assert_eq!((s.mailbox.event_changes.len(), waiting.total, waiting.due), (1, 1, 1));
    let upload = event_upload(&s, account).unwrap();
    assert_eq!(upload.ops.len(), 1);
    assert_eq!((upload.ops[0].kind, upload.ops[0].remote.as_str()), (EventChangeKind::Update, "meeting"));
    assert_eq!(
        serde_json::to_value(&upload.ops[0].fields).unwrap(),
        json!({"time": {"start": at(2026, 10, 13, 15, 0), "end": at(2026, 10, 13, 16, 0), "allDay": false}})
    );
    // Reading, looking around and listing change nothing.
    for (id, params) in [
        ("calendar.list", json!({})),
        ("calendar.calendars", json!({})),
        ("view.reminders", json!({})),
        ("calendar.timeZones", json!({})),
        ("view.timeZone", json!({})),
    ] {
        run(&mut s, id, params);
    }
    assert_eq!(event_upload(&s, account).unwrap(), upload);
    assert!(!s.dirty);
    // The server's calendar comes down as it was left (it doesn't say zones here): nothing is
    // different, and nothing more is to be sent.
    let merged = apply_events(&mut s, &october(account, as_the_server_has_it(false))).unwrap();
    assert_eq!((merged["added"].as_u64(), merged["updated"].as_u64(), merged["removed"].as_u64()), (Some(0), Some(0), Some(0)));
    assert_eq!(event_upload(&s, account).unwrap(), upload);
    // Asked for the zones too, the server says where the series was made: that is taken in
    // (the series was Chicago's all along), and still nothing more is to be sent.
    let before = instants(&s);
    let merged = apply_events(&mut s, &october(account, as_the_server_has_it(true))).unwrap();
    assert_eq!((merged["added"].as_u64(), merged["updated"].as_u64(), merged["removed"].as_u64()), (Some(0), Some(1), Some(0)));
    assert_eq!(by_remote(&s, "series").start_zone, "America/Chicago");
    assert_eq!(by_remote(&s, "meeting").start_zone, "");
    assert_eq!((instants(&s), event_upload(&s, account).unwrap()), (before, upload));
}

#[test]
fn a_computer_carried_to_another_zone_keeps_every_meeting_at_its_instant() {
    let (old, account) = old_mailbox();
    let (chicago, london) = (zone("America/Chicago"), zone("Europe/London"));
    let mut s = Session::in_zone(old, NOW, chicago);
    let (before, upload) = (instants(&s), event_upload(&s, account).unwrap());
    let whole_day = by_remote(&s, "day");
    let reminders = |s: &Session| -> Vec<(Id, i64)> { due_reminders(s, s.local_now()).into_iter().map(|r| (r.event, r.minutes)).collect() };
    s.clock.now = Some(at(2026, 10, 13, 14, 50));
    let due = reminders(&s);
    assert_eq!(due.len(), 1);
    s.clock.now = Some(NOW);

    // The computer is in London now. The next thing that looks at the mailbox finds it there.
    s.clock.zone = Some(london);
    assert!(s.settle_zone());
    assert_eq!((s.mailbox.zone.as_str(), s.zone()), ("Europe/London", london));
    // Every timed event is at the instant it was at, and reads six hours later on the clock.
    assert_eq!(instants(&s), before);
    assert_eq!((by_remote(&s, "meeting").start, by_remote(&s, "meeting").end), (clock(2026, 10, 13, 16, 0), clock(2026, 10, 13, 17, 0)));
    // The whole-day event is the same day.
    assert_eq!(by_remote(&s, "day"), whole_day);
    // The series goes on by Chicago's clock: Monday night there is Tuesday morning here, and
    // the Monday it skips is named by its Tuesday.
    let series = by_remote(&s, "series");
    assert_eq!(
        (series.start, series.start_zone.as_str(), series.exceptions.clone()),
        (clock(2026, 10, 6, 4, 30), "America/Chicago", vec![Date::new(2026, 10, 20).unwrap()])
    );
    let listed = run(&mut s, "calendar.list", json!({"from": "2026-10-20", "to": "2026-11-11"}));
    let late: Vec<&str> =
        listed["events"].as_array().unwrap().iter().filter(|e| e["title"] == "Late call").map(|e| e["start"].as_str().unwrap()).collect();
    // London puts its clocks back on October 25, Chicago on November 1: half past three for a
    // week, then half past four again.
    assert_eq!(late, ["2026-10-27T03:30:00", "2026-11-03T04:30:00", "2026-11-10T04:30:00"]);
    // Nothing more waits for the server than before, and what waits says the same (the upload
    // itself names the zone the calendar is shown in, which is another).
    assert_eq!(event_upload(&s, account).unwrap().ops, upload.ops);
    // The reminder is due at the same instant.
    s.clock.now = Some(at(2026, 10, 13, 14, 50));
    assert_eq!(reminders(&s), due);
    s.clock.now = Some(NOW);
    // "Today" and "now" are London's.
    assert_eq!(s.local_now(), clock(2026, 10, 12, 16, 0));
    // It was a change to save, not one to undo.
    assert!(s.dirty && !s.can_undo());

    // The server's calendar, as it is: everything is as the server has it. (It names the zone
    // the meeting was made in, which is another than this computer's now.)
    let merged = apply_events(&mut s, &october(account, as_the_server_has_it(true))).unwrap();
    assert_eq!((merged["added"].as_u64(), merged["removed"].as_u64()), (Some(0), Some(0)));
    assert_eq!(instants(&s), before);
    assert_eq!(by_remote(&s, "series"), series);
    // (The meeting has a change waiting, so the server's copy of it is the older one.)
    assert_eq!(event_upload(&s, account).unwrap().ops, upload.ops);

    // A new appointment is at the time it is typed, in London; and an undo doesn't carry the
    // mailbox back to Chicago's clock.
    let own = own_calendar(&mut s);
    let id = run(&mut s, "calendar.new", json!({"title": "Tea", "start": "2026-10-14T16:00", "calendar": own}))["id"].as_u64().unwrap();
    assert_eq!(london.instant(s.mailbox.event(id).unwrap().start), at(2026, 10, 14, 15, 0));
    s.clock.zone = Some(chicago);
    run(&mut s, "edit.undo", json!({}));
    assert!(s.mailbox.event(id).is_none());
    assert_eq!((s.mailbox.zone.as_str(), instants(&s)), ("America/Chicago", before.clone()));
    assert_eq!(by_remote(&s, "meeting").start, clock(2026, 10, 13, 10, 0));
    // Home again, the series is where it was, and still says where it was made.
    let home = by_remote(&s, "series");
    assert_eq!(
        (home.start, home.exceptions.clone(), home.start_zone.as_str()),
        (clock(2026, 10, 5, 22, 30), vec![Date::new(2026, 10, 19).unwrap()], "America/Chicago")
    );
    assert_eq!(event_upload(&s, account).unwrap().ops, upload.ops);
}

#[test]
fn a_change_made_while_away_goes_to_the_server_as_the_instant_it_is() {
    let (old, account) = old_mailbox();
    let mut s = Session::in_zone(old, NOW, zone("America/Chicago"));
    // The change that waited is sent and taken on; then the owner flies to Tokyo.
    s.mailbox.event_changes.clear();
    s.clock.zone = Some(zone("Asia/Tokyo"));
    s.settle_zone();
    assert!(event_upload(&s, account).unwrap().ops.is_empty());
    // The holiday is moved a day, the series an hour later: by Tokyo's clock, where the series
    // is at half past twelve on Tuesdays.
    let (day, series) = (by_remote(&s, "day").id, by_remote(&s, "series").id);
    assert_eq!(by_remote(&s, "series").start, clock(2026, 10, 6, 12, 30));
    run(&mut s, "calendar.move", json!({"id": day, "start": "2026-10-15"}));
    run(&mut s, "calendar.move", json!({"id": series, "start": "2026-10-06T13:30"}));
    let upload = event_upload(&s, account).unwrap();
    let fields = |remote: &str| serde_json::to_value(&upload.ops.iter().find(|op| op.remote == remote).unwrap().fields).unwrap();
    // The day is a day; the series is told by its own clock, Chicago's, where it is now at
    // half past eleven at night, and that is the day its rule counts from.
    assert_eq!(fields("day"), json!({"time": {"start": at(2026, 10, 15, 0, 0), "end": at(2026, 10, 16, 0, 0), "allDay": true}}));
    assert_eq!(
        fields("series"),
        json!({
            "time": {"start": at(2026, 10, 6, 4, 30), "end": at(2026, 10, 6, 5, 0), "allDay": false},
            "wall": {"start": clock(2026, 10, 5, 23, 30), "end": clock(2026, 10, 6, 0, 0)},
            "zones": {"start": "America/Chicago", "end": "America/Chicago"},
        })
    );
}

/// A message with a calendar part in the Inbox of the account `ada@example.org`.
fn invitation(s: &mut Session, ics: &str) -> Id {
    let account = s.mailbox.accounts.iter().find(|a| a.email == "ada@example.org").unwrap().id;
    let inbox = s.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    let message = Message {
        folder: inbox,
        message_id: format!("{}@example.org", ics.len()),
        from: Address::new("Bo Marsh", "bo@example.org"),
        to: vec![Address::new("Ada", "ada@example.org")],
        subject: "Design review".into(),
        attachments: vec![Attachment::new("invite.ics", "text/calendar", ics.as_bytes().to_vec())],
        ..Default::default()
    };
    s.mailbox.add_message(message).unwrap()
}

#[test]
fn an_invitation_from_another_zone_is_at_the_right_time_here() {
    let mut s = Session::in_zone(Mailbox::with_local_account("", ""), NOW, zone("America/Chicago"));
    run(&mut s, "account.add", json!({"email": "ada@example.org", "name": "Ada", "incoming": {"host": "imap.example.org"}}));
    // Bo is in Berlin and invites to four in the afternoon there, every Thursday.
    let request = |more: &str| {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nMETHOD:REQUEST\r\nBEGIN:VEVENT\r\nUID:review-1\r\nSUMMARY:Design review\r\nSEQUENCE:0\r\n\
ORGANIZER;CN=Bo Marsh:mailto:bo@example.org\r\nATTENDEE;PARTSTAT=NEEDS-ACTION;CN=Ada:mailto:ada@example.org\r\n{more}END:VEVENT\r\nEND:VCALENDAR\r\n"
        )
    };
    let once = invitation(
        &mut s,
        &request("DTSTART;TZID=W. Europe Standard Time:20261015T160000\r\nDTEND;TZID=W. Europe Standard Time:20261015T170000\r\n"),
    );
    let said = run(&mut s, "calendar.invitation", json!({"message": once}));
    assert_eq!((said["start"].as_str(), said["end"].as_str()), (Some("2026-10-15T09:00:00"), Some("2026-10-15T10:00:00")));
    assert_eq!(
        (said["startZone"].as_str(), said["ownTime"].as_str(), said["notes"].clone()),
        (Some("Europe/Berlin"), Some("4:00 PM – 5:00 PM (W. Europe Time)"), json!([]))
    );
    // Accepted, it is on the calendar at nine, and says where it is at home.
    let accepted = run(&mut s, "calendar.respond", json!({"message": once, "response": "accept"}));
    let event = s.mailbox.event(accepted["event"].as_u64().unwrap()).unwrap().clone();
    assert_eq!((event.start, event.end, event.start_zone.as_str()), (clock(2026, 10, 15, 9, 0), clock(2026, 10, 15, 10, 0), "Europe/Berlin"));
    // The series: nine here, ten in the week when only Berlin has put its clocks back.
    let weekly = invitation(
        &mut s,
        &request("DTSTART;TZID=Europe/Berlin:20261015T160000\r\nDTEND;TZID=Europe/Berlin:20261015T170000\r\nRRULE:FREQ=WEEKLY\r\n")
            .replace("review-1", "review-2"),
    );
    let accepted = run(&mut s, "calendar.respond", json!({"message": weekly, "response": "accept"}));
    let series = accepted["event"].as_u64().unwrap();
    let listed = run(&mut s, "calendar.list", json!({"from": "2026-10-16", "to": "2026-11-06"}));
    let starts: Vec<&str> = listed["events"].as_array().unwrap().iter().filter(|e| e["id"] == series).map(|e| e["start"].as_str().unwrap()).collect();
    assert_eq!(starts, ["2026-10-22T09:00:00", "2026-10-29T10:00:00", "2026-11-05T09:00:00"]);
    // A zone nobody knows: as it is written, and the message says so.
    let odd = invitation(
        &mut s,
        &request("DTSTART;TZID=Customized Time Zone:20261016T160000\r\nDTEND;TZID=Customized Time Zone:20261016T170000\r\n")
            .replace("review-1", "review-3"),
    );
    let said = run(&mut s, "calendar.invitation", json!({"message": odd}));
    assert_eq!((said["start"].as_str(), said["startZone"].is_null()), (Some("2026-10-16T16:00:00"), true));
    assert_eq!(said["notes"], json!(["The time zone \"Customized Time Zone\" isn't one that is known: its times are kept as they are written."]));
    // A file of events is read the same way, and written back with its zones.
    let file = request("DTSTART;TZID=Asia/Kolkata:20261020T193000\r\nDTEND;TZID=Asia/Kolkata:20261020T203000\r\n")
        .replace("review-1", "review-4")
        .replace("METHOD:REQUEST\r\n", "");
    let read = crate::io::import(&mut s, "meeting.ics", file.as_bytes(), None).unwrap();
    let id = read["events"].as_u64().map(|_| s.sel.event.unwrap()).unwrap();
    assert_eq!((s.mailbox.event(id).unwrap().start, read["notes"].clone()), (clock(2026, 10, 20, 9, 0), json!([])));
    let written = run(&mut s, "file.exportCalendar", json!({"id": id}));
    let text = String::from_utf8(crate::cmd::base64_decode(written["data"].as_str().unwrap()).unwrap()).unwrap();
    assert!(text.contains("DTSTART;TZID=Asia/Kolkata:20261020T193000\r\n") && text.contains("BEGIN:VTIMEZONE\r\nTZID:Asia/Kolkata\r\n"), "{text}");
}

#[test]
fn the_mailboxs_working_hours_are_the_hours_of_its_own_zone() {
    let mut s = Session::in_zone(Mailbox::with_local_account("", ""), NOW, zone("America/Chicago"));
    let account = run(&mut s, "account.add", json!({"email": "ada@example.org", "name": "Ada", "incoming": {"host": "imap.example.org"}}))["id"]
        .as_u64()
        .unwrap();
    let hours =
        |zone: &str| json!({"account": account, "workingHours": {"days": ["monday", "tuesday"], "start": "09:00", "end": "17:00", "zone": zone}});
    // Nine to five in Berlin is two to ten in the morning in Chicago (seven hours behind in
    // October), whatever the server calls the zone.
    for name in ["W. Europe Standard Time", "Europe/Berlin"] {
        let said = run(&mut s, "sync.applyMailboxSettings", hours(name));
        assert_eq!((said["start"].as_str(), said["end"].as_str(), said["from"].as_str()), (Some("02:00"), Some("10:00"), Some("mailbox")), "{name}");
    }
    // In the week when only Berlin has put its clocks back, it is three to eleven.
    s.clock.now = Some(at(2026, 10, 27, 15, 0));
    assert_eq!(run(&mut s, "view.workHours", json!({}))["start"], "03:00");
    s.clock.now = Some(NOW);
    // The mailbox's own zone is this computer's: as they are. A zone nobody knows: as they
    // are too (and an offset a host gave still counts).
    for name in ["Central Standard Time", "Customized Time Zone", ""] {
        assert_eq!(run(&mut s, "sync.applyMailboxSettings", hours(name))["start"], "09:00", "{name}");
    }
    let mut given = hours("Customized Time Zone");
    given["workingHours"]["offset"] = json!(60);
    assert_eq!(run(&mut s, "sync.applyMailboxSettings", given)["start"], "03:00");
    // Shown in another zone, the hours follow.
    run(&mut s, "sync.applyMailboxSettings", hours("Europe/Berlin"));
    run(&mut s, "view.timeZone", json!({"zone": "Asia/Kolkata"}));
    assert_eq!(run(&mut s, "view.workHours", json!({}))["start"], "12:30");
}

#[test]
fn a_whole_day_made_far_away_is_the_day_it_is_there() {
    // A holiday and a weekly day off, made in New York: the server tells them as New York's
    // midnights, which is one in the afternoon in Tokyo. They are the 20th all the same.
    let eastern = |instant: Timestamp, id: &str, kind: RemoteEventKind| RemoteEvent {
        id: id.into(),
        change_key: "ck-1".into(),
        title: if kind == RemoteEventKind::Single { "Holiday".into() } else { "Day off".into() },
        start: instant,
        end: instant + 86_400,
        all_day: true,
        kind,
        series: if kind == RemoteEventKind::Occurrence { "off".into() } else { String::new() },
        start_zone: "Eastern Standard Time".into(),
        end_zone: "Eastern Standard Time".into(),
        ..Default::default()
    };
    let rule = Recurrence { by_day: vec![Weekday::Tue], ..Recurrence::every(Freq::Weekly) };
    let server = vec![
        eastern(at(2026, 10, 20, 4, 0), "day", RemoteEventKind::Single),
        RemoteEvent {
            recurrence: Some(rule),
            series_day: Date::new(2026, 10, 20),
            ..eastern(at(2026, 10, 20, 4, 0), "off", RemoteEventKind::SeriesMaster)
        },
        eastern(at(2026, 10, 20, 4, 0), "o-1", RemoteEventKind::Occurrence),
        eastern(at(2026, 10, 27, 4, 0), "o-2", RemoteEventKind::Occurrence),
    ];
    for there in ["Asia/Tokyo", "America/New_York", "Pacific/Honolulu", "Pacific/Kiritimati", "Asia/Kathmandu"] {
        let mut s = Session::in_zone(Mailbox::with_local_account("", ""), NOW, zone(there));
        let account = run(&mut s, "account.add", json!({"email": "ada@example.org", "incoming": {"host": "outlook.example.org"}, "auth": "oauth2"}))
            ["id"]
            .as_u64()
            .unwrap();
        apply_events(&mut s, &october(account, server.clone())).unwrap();
        let day = by_remote(&s, "day");
        assert_eq!((day.start, day.end, day.all_day), (clock(2026, 10, 20, 0, 0), clock(2026, 10, 21, 0, 0), true), "{there}");
        // The series is one event that skips nothing: each of its days is the server's day.
        let off = by_remote(&s, "off");
        assert_eq!((off.start, off.exceptions.len(), s.mailbox.events.len()), (clock(2026, 10, 20, 0, 0), 0, 2), "{there}");
        // The same again changes nothing, and nothing is to be sent.
        let merged = apply_events(&mut s, &october(account, server.clone())).unwrap();
        assert_eq!((merged["added"].as_u64(), merged["updated"].as_u64(), merged["removed"].as_u64()), (Some(0), Some(0), Some(0)), "{there}");
        assert!(event_upload(&s, account).unwrap().ops.is_empty());
    }
    // A server that doesn't say where a day was made, or names a zone nobody knows: the
    // midnight nearest here, as before.
    let mut s = Session::in_zone(Mailbox::with_local_account("", ""), NOW, zone("Europe/Berlin"));
    let account =
        run(&mut s, "account.add", json!({"email": "ada@example.org", "incoming": {"host": "outlook.example.org"}, "auth": "oauth2"}))["id"]
            .as_u64()
            .unwrap();
    let unsaid: Vec<RemoteEvent> = ["", "Customized Time Zone"]
        .iter()
        .map(|name| RemoteEvent {
            id: format!("day{}", name.len()),
            start_zone: (*name).into(),
            end_zone: (*name).into(),
            ..eastern(at(2026, 10, 20, 4, 0), "", RemoteEventKind::Single)
        })
        .collect();
    apply_events(&mut s, &october(account, unsaid)).unwrap();
    assert!(s.mailbox.events.values().all(|e| (e.start, e.end) == (clock(2026, 10, 20, 0, 0), clock(2026, 10, 21, 0, 0))));
}

#[test]
fn what_was_sent_before_a_journey_is_found_on_the_server_after_it() {
    use mailcraft_model::remote::{EventOutcome, EventResult, EventsUploaded};
    let (old, account) = old_mailbox();
    let mut s = Session::in_zone(old, NOW, zone("America/Chicago"));
    // The move of the meeting goes to the server, and the holiday is deleted there: the server
    // says yes to both, and no download has shown either yet.
    let day = by_remote(&s, "day").id;
    run(&mut s, "calendar.delete", json!({"id": day}));
    let upload = event_upload(&s, account).unwrap();
    assert_eq!(upload.ops.len(), 2);
    let said = |op: &mailcraft_model::remote::EventOp| EventOutcome {
        op: op.clone(),
        result: EventResult::Done,
        remote: op.remote.clone(),
        change_key: "ck-2".into(),
        ..Default::default()
    };
    let answer = EventsUploaded { account, email: "ada@example.org".into(), outcomes: upload.ops.iter().map(said).collect(), ..Default::default() };
    super::remote::apply_event_outcomes(&mut s, &answer).unwrap();
    assert_eq!((s.mailbox.event_changes.len(), s.mailbox.event_checks.len()), (0, 2));
    let noted = s.mailbox.event_checks.clone();

    // The computer is in Tokyo before the next download. What is looked for is the same
    // moments; the day that was deleted is the same day.
    s.clock.zone = Some(zone("Asia/Tokyo"));
    assert!(s.settle_zone());
    let times = |checks: &[mailcraft_model::remote::EventCheck]| checks.iter().map(|c| (c.kind, c.remote.clone(), c.fields.time)).collect::<Vec<_>>();
    assert_eq!(times(&s.mailbox.event_checks), times(&noted));
    let gone = s.mailbox.event_checks.iter().find_map(|check| check.base.clone()).unwrap();
    assert_eq!((gone.all_day, gone.start, gone.end), (true, clock(2026, 10, 14, 0, 0), clock(2026, 10, 15, 0, 0)));

    // The server's calendar as it is now: the meeting at ten in Chicago, and no holiday. Both
    // are seen to have arrived: nothing is put back, nothing is said, and nothing waits.
    let mut server = as_the_server_has_it(true);
    server.retain(|event| event.id != "day");
    for event in server.iter_mut().filter(|event| event.id == "meeting") {
        (event.start, event.end, event.change_key) = (at(2026, 10, 13, 15, 0), at(2026, 10, 13, 16, 0), "ck-2".into());
    }
    let merged = apply_events(&mut s, &october(account, server)).unwrap();
    assert_eq!((merged["verified"].as_u64(), merged["putBack"].as_u64(), merged["unverified"].as_u64()), (Some(2), Some(0), Some(0)), "{merged}");
    assert!(s.mailbox.event_checks.is_empty() && event_upload(&s, account).unwrap().ops.is_empty());
    // Ten in the morning in Chicago is midnight in Tokyo.
    assert_eq!(by_remote(&s, "meeting").start, clock(2026, 10, 14, 0, 0));
    assert!(s.mailbox.events.values().all(|event| event.origin.id != "day"));
}

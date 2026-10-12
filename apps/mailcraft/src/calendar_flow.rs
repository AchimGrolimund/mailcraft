//! The whole way of a calendar change, from the command to the server and back: the engine, the
//! window's calendar timer, this host's Microsoft Graph job and `mailcraft-net`, against a
//! stand-in that takes only what the documented service takes
//! (`mailcraft_net::graph::standin`). Nothing here leaves this machine, and nobody is signed in.
//!
//! The host's job runs on the test's own thread (the stand-in is spoken to in Graph's place on
//! that thread only), so a job is "in flight" from the look that starts it to the look that
//! takes its answer in: what the owner does in between is done while it is under way.

use mailcraft_engine::Session;
use mailcraft_model::remote::{CalendarsDownload, EventChange, EventChangeKind, EventTime};
use mailcraft_model::sync::Tokens;
use mailcraft_model::{Address, Event, Id, Mailbox, Response};
use mailcraft_net::graph::standin::{Routed, StandIn, Stored, TOKEN, zone_offset};
use mailcraft_time::{Date, DateTime};
use mailcraft_ui_egui::{CalendarCheckRequest, GraphRequest, MailApp, Services};
use serde_json::{Value, json};

/// Wednesday 2026-10-14, 16:00 UTC: noon in New York.
const NOW: i64 = 1_791_993_600;
const SECOND: f64 = 1_000.0;
/// The owner's zone, by the name Windows has for it; four hours behind UTC all October.
const EASTERN: &str = "Eastern Standard Time";
const OFFSET: i32 = -240;

/// An instant, by the clock in UTC.
fn utc(month: u8, day: u8, hour: u8, minute: u8) -> i64 {
    DateTime::new(Date::new(2026, month, day).unwrap(), hour, minute, 0).unwrap().to_timestamp(0)
}

/// A wall-clock time here.
fn clock(month: u8, day: u8, hour: u8, minute: u8) -> DateTime {
    DateTime::new(Date::new(2026, month, day).unwrap(), hour, minute, 0).unwrap()
}

/// The app with an account that signs in with Microsoft, the calendar in front, and the
/// stand-in in Graph's place.
struct Flow {
    app: MailApp,
    server: StandIn,
    account: Id,
    /// The time of the last look at the calendar (milliseconds).
    at: f64,
    _route: Routed,
}

impl Flow {
    /// With "Budget review" on the server, Monday the 12th from nine to ten in New York; the
    /// first look at the calendar has brought it down.
    fn start() -> Flow {
        Flow::start_after(|_| {})
    }

    /// [`Flow::start`], with what `before` does to the app before its account's calendar first
    /// comes down (what was in the mailbox before the account was connected).
    fn start_after(before: impl FnOnce(&mut MailApp)) -> Flow {
        let server = StandIn::start().unwrap();
        let main = server.state().main_calendar();
        server.state().owner = "avery@mail.example".into();
        server.state().add_event(&main, "Budget review", utc(10, 12, 13, 0));
        let route = server.route();
        let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, OFFSET), Services::default());
        app.autosave = false;
        // The host's own job, as the app runs it, in the owner's zone.
        app.services.graph = Some(Box::new(|request: GraphRequest| {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(super::graph_in(request, EASTERN));
            rx
        }));
        app.services.check_calendar = Some(Box::new(|request: CalendarCheckRequest| {
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(super::check_calendar_in(request, EASTERN, &|instant| zone_offset(EASTERN, instant).unwrap_or(0)));
            rx
        }));
        let added = app.execute(
            "account.add",
            json!({"email": "avery@mail.example", "auth": "oauth2", "provider": "microsoft", "incoming": {"host": "imap.mail.example"}}),
        );
        let account = added.unwrap()["id"].as_u64().unwrap();
        let fresh = Tokens { access: TOKEN.into(), refresh: "ref".into(), expires_at: mailcraft_time::now() + 3_600, ..Default::default() };
        app.seed_graph_token(account, fresh);
        app.execute("view.calendar", json!({})).unwrap();
        before(&mut app);
        let mut flow = Flow { app, server, account, at: mailcraft_ui_egui::now_ms(), _route: route };
        assert!(flow.look(), "the calendar is asked for to begin with");
        flow.take_in();
        assert_eq!(flow.here("Budget review").start, clock(10, 12, 9, 0));
        flow
    }

    /// Look at the calendar half a minute on: a job starts when one is due. Whether one did.
    fn look(&mut self) -> bool {
        self.at += 31.0 * SECOND;
        self.app.calendar_tick_at(true, self.at)
    }

    /// Take in the answer of the job in flight.
    fn take_in(&mut self) {
        for _ in 0..4 {
            self.app.poll_calendar();
        }
    }

    /// One round: what waits goes to the server, and what the server has comes down.
    fn sync(&mut self) {
        self.look();
        self.take_in();
    }

    fn run(&mut self, id: &str, params: Value) -> Value {
        self.app.execute(id, params.clone()).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
    }

    /// The events called `title` here, in order.
    fn all_here(&self, title: &str) -> Vec<Event> {
        let mut events: Vec<Event> = self.app.session.mailbox.events.values().filter(|e| e.title == title).map(|e| e.as_ref().clone()).collect();
        events.sort_by_key(|e| (e.start, e.id));
        events
    }

    fn here(&self, title: &str) -> Event {
        self.all_here(title).into_iter().next().unwrap_or_else(|| panic!("no event \"{title}\" here"))
    }

    /// The events called `title` on the server, in order.
    fn all_there(&self, title: &str) -> Vec<Stored> {
        let mut events: Vec<Stored> = self.server.state().events.iter().filter(|e| e.subject == title).cloned().collect();
        events.sort_by_key(|e| (e.start, e.id.clone()));
        events
    }

    fn there(&self, title: &str) -> Stored {
        self.all_there(title).into_iter().next().unwrap_or_else(|| panic!("no event \"{title}\" on the server"))
    }

    /// What the server answered with anything but a yes, so far.
    fn refusals(&self) -> Vec<(String, String, u16)> {
        let state = self.server.state();
        state.requests.iter().filter(|asked| asked.status >= 400).map(|asked| (asked.method.clone(), asked.target.clone(), asked.status)).collect()
    }

    /// Nothing waits, nothing is yet to be seen on the server, and nothing went wrong.
    fn settled(&self) -> bool {
        let mailbox = &self.app.session.mailbox;
        mailbox.event_changes.is_empty() && mailbox.event_checks.is_empty() && !self.app.has_sync_problems()
    }

    fn status(&self) -> String {
        self.app.status_msg.as_ref().map(|(text, _)| text.clone()).unwrap_or_default()
    }
}

#[test]
fn an_appointment_is_made_dragged_resized_edited_and_deleted_on_the_server() {
    let mut flow = Flow::start();
    // Made: on the server at once, under the name the server gave it, and here once.
    let lunch = flow.run("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00"}))["id"].as_u64().unwrap();
    flow.sync();
    let there = flow.there("Lunch");
    assert_eq!(
        (there.start, there.end, there.all_day, there.calendar.clone()),
        (utc(10, 15, 16, 0), utc(10, 15, 16, 30), false, flow.server.state().main_calendar())
    );
    let here = flow.here("Lunch");
    assert_eq!((here.id, here.origin.id.as_str(), here.origin.account, flow.all_here("Lunch").len()), (lunch, there.id.as_str(), flow.account, 1));
    assert!(flow.settled());

    // Dragged to Friday at nine: the server's copy is at nine in New York on Friday.
    flow.run("calendar.move", json!({"id": lunch, "start": "2026-10-16T09:00"}));
    assert_eq!(flow.app.session.mailbox.event_changes.len(), 1);
    flow.sync();
    assert_eq!((flow.there("Lunch").start, flow.there("Lunch").end), (utc(10, 16, 13, 0), utc(10, 16, 13, 30)));
    assert_eq!(flow.here("Lunch").start, clock(10, 16, 9, 0));
    assert!(flow.settled());
    // The one that came from the server is dragged too: it is the server's event that moves.
    let review = flow.here("Budget review").id;
    flow.run("calendar.move", json!({"id": review, "start": "2026-10-13T14:00"}));
    flow.sync();
    assert_eq!((flow.there("Budget review").start, flow.all_there("Budget review").len()), (utc(10, 13, 18, 0), 1));

    // Resized: an hour and a half.
    flow.run("calendar.move", json!({"id": lunch, "start": "2026-10-16T09:00", "end": "2026-10-16T10:30"}));
    flow.sync();
    assert_eq!((flow.there("Lunch").start, flow.there("Lunch").end), (utc(10, 16, 13, 0), utc(10, 16, 14, 30)));
    // Edited in its form: everything the form has goes, and comes back as it went.
    let fields = json!({
        "id": lunch, "title": "Long lunch", "location": "Canteen", "description": "Bring\nthe numbers.", "showAs": "tentative",
        "private": true, "reminder": 30, "categories": ["Team"],
    });
    flow.run("calendar.edit", fields);
    flow.sync();
    let there = flow.there("Long lunch");
    assert_eq!(
        (there.location.as_str(), there.body.as_str(), there.show_as.as_str(), there.sensitivity.as_str()),
        ("Canteen", "Bring\nthe numbers.", "tentative", "private")
    );
    assert_eq!((there.reminder_on, there.reminder_minutes, there.categories.as_slice()), (true, 30, &["Team".to_string()][..]));
    let here = flow.here("Long lunch");
    assert_eq!(
        (here.location.as_str(), here.description.as_str(), here.private, here.reminder_minutes),
        ("Canteen", "Bring\nthe numbers.", true, Some(30))
    );
    assert!(flow.settled());

    // Deleted: gone on the server.
    flow.run("calendar.delete", json!({"id": lunch}));
    flow.sync();
    assert!(flow.all_there("Long lunch").is_empty() && flow.all_here("Long lunch").is_empty());
    assert!(flow.settled());
    // Not one request on the whole way was anything the service doesn't take.
    assert_eq!(flow.refusals(), []);
    // And every change went to the event by its own id, written as one piece of the address.
    let writes = flow.server.state().writes();
    let kinds: Vec<&str> = writes.iter().map(|(method, _)| method.as_str()).collect();
    assert_eq!(kinds, ["POST", "PATCH", "PATCH", "PATCH", "PATCH", "DELETE"]);
    assert!(writes.iter().skip(1).all(|(_, path)| path.len() == 4 && path[..3] == ["v1.0", "me", "events"] && path[3].len() > 100), "{writes:?}");
}

#[test]
fn a_whole_day_a_series_and_one_of_its_days_are_what_the_server_has() {
    let mut flow = Flow::start();
    // A whole day: a day in the owner's zone on the server, and still that day when it comes back.
    let offsite = flow.run("calendar.new", json!({"title": "Offsite", "start": "2026-10-20", "allDay": true}))["id"].as_u64().unwrap();
    flow.sync();
    let there = flow.there("Offsite");
    assert_eq!((there.all_day, there.zone.as_str(), there.start, there.end), (true, EASTERN, utc(10, 20, 4, 0), utc(10, 21, 4, 0)));
    let here = flow.here("Offsite");
    assert_eq!((here.all_day, here.start, here.end), (true, clock(10, 20, 0, 0), clock(10, 21, 0, 0)));
    // Dragged to the day after, and made two days long.
    flow.run("calendar.move", json!({"id": offsite, "start": "2026-10-21", "end": "2026-10-22"}));
    flow.sync();
    assert_eq!((flow.there("Offsite").start, flow.there("Offsite").end), (utc(10, 21, 4, 0), utc(10, 23, 4, 0)));
    assert_eq!((flow.here("Offsite").start, flow.here("Offsite").end), (clock(10, 21, 0, 0), clock(10, 23, 0, 0)));
    assert!(flow.settled());

    // A series: nine in the morning on Mondays, Wednesdays and Fridays, by the clock in New York.
    let rule = json!({"freq": "weekly", "byDay": ["mon", "wed", "fri"]});
    let made = flow.run("calendar.new", json!({"title": "Stand-up", "start": "2026-10-05T09:00", "end": "2026-10-05T09:30", "recurrence": rule}));
    let series = made["id"].as_u64().unwrap();
    flow.sync();
    let master = flow.there("Stand-up");
    assert_eq!((master.start, master.zone.as_str(), master.recurrence.is_some()), (utc(10, 5, 13, 0), EASTERN, true));
    // One event here, which repeats and is the server's; its days in view are the server's days.
    let all = flow.all_here("Stand-up");
    assert_eq!(
        (all.len(), all[0].id, all[0].origin.id.as_str(), all[0].recurrence.is_some(), all[0].exceptions.len()),
        (1, series, master.id.as_str(), true, 0)
    );

    // Friday's alone, an hour later: an exception on the server under the id the occurrence had.
    flow.run("calendar.move", json!({"id": series, "start": "2026-10-05T10:00", "occurrence": "2026-10-16"}));
    flow.sync();
    let exception = flow.all_there("Stand-up").into_iter().find(|e| !e.series.is_empty()).expect("the changed day is an event of its own there");
    assert_eq!((exception.series.as_str(), exception.original, exception.start), (master.id.as_str(), utc(10, 16, 13, 0), utc(10, 16, 14, 0)));
    let all = flow.all_here("Stand-up");
    assert_eq!(all.len(), 2);
    let one = all.iter().find(|e| e.recurrence.is_none()).unwrap();
    assert_eq!((one.start, one.origin.id.as_str(), one.origin.series.as_str()), (clock(10, 16, 10, 0), exception.id.as_str(), master.id.as_str()));
    assert_eq!(all.iter().find(|e| e.recurrence.is_some()).unwrap().exceptions, [Date::new(2026, 10, 16).unwrap()]);
    assert!(flow.settled());
    // Monday's alone is deleted: the series no longer has it there, and the rest stands.
    flow.run("calendar.delete", json!({"id": series, "occurrence": "2026-10-12"}));
    flow.sync();
    assert!(flow.there("Stand-up").gone.contains(&utc(10, 12, 13, 0)));
    let kept = flow.all_here("Stand-up").into_iter().find(|e| e.recurrence.is_some()).unwrap();
    assert_eq!(kept.exceptions, [Date::new(2026, 10, 12).unwrap(), Date::new(2026, 10, 16).unwrap()]);
    assert!(flow.settled());
    // The whole series half an hour later: every day of it, on the server too.
    flow.run("calendar.move", json!({"id": series, "start": "2026-10-05T09:30", "scope": "series"}));
    flow.sync();
    assert_eq!(flow.there("Stand-up").start, utc(10, 5, 13, 30));
    assert!(flow.settled());
    // And deleted as a whole.
    flow.run("calendar.delete", json!({"id": series, "scope": "series"}));
    flow.sync();
    assert!(flow.all_there("Stand-up").is_empty() && flow.all_here("Stand-up").is_empty());
    assert!(flow.settled());
    assert_eq!(flow.refusals(), []);
}

#[test]
fn an_item_of_another_calendar_goes_through_that_calendar() {
    let mut flow = Flow::start();
    let (team, shared) = {
        let mut state = flow.server.state();
        let (team, shared) = (state.add_calendar("Team", true, false), state.add_calendar("Bo's calendar", true, true));
        state.add_event(&team, "Retro", utc(10, 15, 17, 0));
        state.add_event(&shared, "Their review", utc(10, 14, 15, 0));
        (team, shared)
    };
    // The account's calendars as a Send/Receive lists them, both ticked.
    let list: CalendarsDownload = mailcraft_net::graph::calendars(TOKEN).unwrap();
    let account = flow.app.session.mailbox.account(flow.account).cloned().unwrap();
    flow.run("sync.applyCalendars", json!({"download": list.for_account(&account)}));
    let local = |flow: &Flow, name: &str| flow.app.session.mailbox.calendars.iter().find(|c| c.name == name).map(|c| c.id).unwrap();
    for name in ["Team", "Bo's calendar"] {
        let id = local(&flow, name);
        flow.run("calendar.showCalendar", json!({"id": id, "value": true}));
    }
    flow.sync();
    assert_eq!((flow.here("Retro").start, flow.here("Their review").start), (clock(10, 15, 13, 0), clock(10, 14, 11, 0)));

    // An event of the calendar that somebody else shares is moved: through that calendar, which
    // is the only way the service knows it by.
    let theirs = flow.here("Their review").id;
    flow.run("calendar.move", json!({"id": theirs, "start": "2026-10-14T15:00"}));
    flow.sync();
    assert_eq!(flow.there("Their review").start, utc(10, 14, 19, 0));
    assert!(flow.settled(), "{}", flow.status());
    // A new one in the second calendar, a change to one there, and a deletion.
    let team_here = local(&flow, "Team");
    let poker =
        flow.run("calendar.new", json!({"title": "Planning poker", "start": "2026-10-16T10:00", "calendar": team_here}))["id"].as_u64().unwrap();
    let retro = flow.here("Retro").id;
    flow.run("calendar.move", json!({"id": retro, "start": "2026-10-15T15:00"}));
    flow.sync();
    assert_eq!((flow.there("Planning poker").calendar, flow.there("Retro").start), (team.clone(), utc(10, 15, 19, 0)));
    flow.run("calendar.delete", json!({"id": poker}));
    flow.run("calendar.delete", json!({"id": theirs}));
    flow.sync();
    assert!(flow.all_there("Planning poker").is_empty() && flow.all_there("Their review").is_empty());
    assert!(flow.settled());
    assert_eq!(flow.refusals(), []);
    // Every one of those went through its calendar.
    let writes = flow.server.state().writes();
    let through: Vec<&str> = writes
        .iter()
        .filter(|(_, path)| path.get(2).is_some_and(|piece| piece == "calendars"))
        .filter_map(|(_, path)| path.get(3).map(String::as_str))
        .collect();
    assert_eq!(through, [shared.as_str(), team.as_str(), team.as_str(), team.as_str(), shared.as_str()]);
}

#[test]
fn a_change_made_while_a_refresh_is_in_flight_is_not_lost() {
    let mut flow = Flow::start();
    let review = flow.here("Budget review").id;
    // A refresh is under way (the server answered it with the event at nine on Monday)…
    assert!(flow.look());
    // …and before its answer is taken in, the owner drags the event to Tuesday at two.
    flow.run("calendar.move", json!({"id": review, "start": "2026-10-13T14:00"}));
    flow.take_in();
    // The answer is older than the change: the event stays where the owner put it, and the
    // change still waits.
    assert_eq!(flow.here("Budget review").start, clock(10, 13, 14, 0));
    assert_eq!(flow.app.session.mailbox.event_changes.iter().map(|c| c.kind).collect::<Vec<_>>(), [EventChangeKind::Update]);
    assert_eq!(flow.there("Budget review").start, utc(10, 12, 13, 0));
    // The next look sends it.
    flow.sync();
    assert_eq!(flow.there("Budget review").start, utc(10, 13, 18, 0));
    assert_eq!(flow.here("Budget review").start, clock(10, 13, 14, 0));
    assert!(flow.settled());

    // The same with a change sent in the very job whose refresh then lags: a second change
    // made while that job is in flight waits for the next, and goes after it.
    flow.run("calendar.move", json!({"id": review, "start": "2026-10-14T10:00"}));
    assert!(flow.look());
    flow.run("calendar.edit", json!({"id": review, "title": "Budget review, final"}));
    flow.take_in();
    assert_eq!(flow.there("Budget review").start, utc(10, 14, 14, 0));
    assert_eq!(flow.here("Budget review, final").start, clock(10, 14, 10, 0));
    flow.sync();
    assert_eq!(flow.there("Budget review, final").start, utc(10, 14, 14, 0));
    assert!(flow.settled());
    // One moved out of the days in view while a refresh was in flight is neither lost here nor
    // forgotten for the server: its move goes with the next look.
    assert!(flow.look());
    flow.run("calendar.move", json!({"id": review, "start": "2026-11-20T10:00"}));
    flow.take_in();
    assert_eq!(flow.here("Budget review, final").start, DateTime::new(Date::new(2026, 11, 20).unwrap(), 10, 0, 0).unwrap());
    flow.sync();
    // (Ten o'clock by this session's clock, which stands four hours behind UTC all year.)
    assert_eq!(flow.there("Budget review, final").start, utc(11, 20, 14, 0));
    assert_eq!(flow.refusals(), []);
}

#[test]
fn a_refresh_that_is_older_than_the_change_it_follows_puts_nothing_back() {
    let mut flow = Flow::start();
    let review = flow.here("Budget review").id;
    // The view that follows the change is answered as the calendar was before it.
    flow.server.state().stale_views = 1;
    flow.run("calendar.move", json!({"id": review, "start": "2026-10-13T14:00"}));
    flow.sync();
    // The server has the change; what came down didn't show it yet. The event stays where the
    // owner put it, nothing is said, and the change is noted as yet to be seen.
    assert_eq!(flow.there("Budget review").start, utc(10, 13, 18, 0));
    assert_eq!(flow.here("Budget review").start, clock(10, 13, 14, 0));
    assert_eq!((flow.app.session.mailbox.event_checks.len(), flow.status(), flow.app.has_sync_problems()), (1, String::new(), false));
    // It is asked for again within seconds, not when the calendar is next due; then it is seen.
    flow.at += 11.0 * SECOND;
    assert!(flow.app.calendar_tick_at(true, flow.at), "a change that isn't seen yet is looked for again soon");
    flow.take_in();
    assert!(flow.settled());
    assert_eq!(flow.here("Budget review").start, clock(10, 13, 14, 0));

    // A new appointment that the lagging view doesn't have yet is not taken for one the server lost.
    flow.server.state().stale_views = 1;
    let lunch = flow.run("calendar.new", json!({"title": "Lunch", "start": "2026-10-15T12:00"}))["id"].as_u64().unwrap();
    flow.sync();
    assert_eq!((flow.all_here("Lunch").len(), flow.all_there("Lunch").len()), (1, 1));
    flow.sync();
    assert!(flow.settled() && flow.here("Lunch").origin.id == flow.there("Lunch").id);
    // And one that was deleted, and is still in the lagging view, doesn't come back.
    flow.server.state().stale_views = 1;
    flow.run("calendar.delete", json!({"id": lunch}));
    flow.sync();
    assert!(flow.all_here("Lunch").is_empty() && flow.all_there("Lunch").is_empty());
    flow.sync();
    assert!(flow.settled() && flow.all_here("Lunch").is_empty());
    assert_eq!(flow.refusals(), []);
}

#[test]
fn a_server_that_says_yes_and_keeps_the_old_time_is_found_out_and_said() {
    let mut flow = Flow::start();
    let review = flow.here("Budget review").id;
    // The server answers the change with a yes (200, a new version of the event) and has the
    // time it had: the very case of a change that "didn't reflect".
    flow.server.state().keeps_times = true;
    flow.run("calendar.move", json!({"id": review, "start": "2026-10-13T14:00"}));
    flow.sync();
    assert_eq!(flow.there("Budget review").start, utc(10, 12, 13, 0));
    // It doesn't pass: the event is back where the server has it, and the owner is told which
    // one and what of it, in the status bar and in the Send/Receive problems.
    assert_eq!(flow.here("Budget review").start, clock(10, 12, 9, 0));
    let line = "The server took the change to \"Budget review\" but has its time otherwise than it was sent. It is shown here as the server has it.";
    assert_eq!(flow.status(), line);
    assert!(flow.app.has_sync_problems());
    assert!(flow.app.session.mailbox.event_changes.is_empty() && flow.app.session.mailbox.event_checks.is_empty());
}

#[test]
fn what_is_only_on_this_computer_is_moved_to_the_account_and_arrives() {
    // From before the account was connected: an appointment in the calendar the mailbox was made
    // with, and a copy of what the server has too, at the same time. They are on this computer.
    let mut flow = Flow::start_after(|app| {
        app.execute("calendar.new", json!({"title": "Dentist", "start": "2026-10-15T15:00"})).unwrap();
        app.execute("calendar.new", json!({"title": "Budget review", "start": "2026-10-12T09:00", "end": "2026-10-12T10:00"})).unwrap();
    });
    let own = flow.app.session.mailbox.calendars.iter().find(|c| c.origin.is_local()).map(|c| (c.id, c.name.clone())).unwrap();
    assert_eq!(own.1, "On this computer");
    let own = own.0;
    let dentist = flow.here("Dentist").id;
    assert_eq!(flow.all_here("Budget review").len(), 2);
    flow.sync();
    assert!(flow.all_there("Dentist").is_empty() && flow.app.session.mailbox.event_changes.is_empty());
    assert!(mailcraft_engine::cmd::remote::stays_here(&flow.app.session.mailbox, dentist));
    // A change to it stays here, and says so (the status bar has it, as the owner's command would).
    flow.app.status_msg = None;
    flow.app.run("calendar.move", json!({"id": dentist, "start": "2026-10-15T16:00"})).unwrap();
    assert_eq!(flow.status(), "Saved on this computer only: this item is not in your Outlook calendar");
    flow.sync();
    assert!(flow.all_there("Dentist").is_empty());

    // Moved to the account: it is made on the server; the copy of what is there already goes;
    // and the calendar that is left empty goes with it.
    let moved = flow.run("calendar.moveToAccount", json!({"send": false}));
    assert_eq!((moved["moved"].as_u64(), moved["already"].as_u64(), moved["removed"].as_u64()), (Some(1), Some(1), Some(1)));
    flow.sync();
    assert_eq!(
        (flow.there("Dentist").start, flow.all_there("Budget review").len(), flow.all_here("Budget review").len()),
        (utc(10, 15, 20, 0), 1, 1)
    );
    assert!(flow.app.session.mailbox.calendar(own).is_none() && flow.settled());
    assert!(!mailcraft_engine::cmd::remote::stays_here(&flow.app.session.mailbox, dentist));
    assert_eq!(flow.refusals(), []);
}

#[test]
fn the_connection_check_runs_on_the_account_and_reports_every_step() {
    let mut flow = Flow::start();
    let before: Vec<Stored> = flow.server.state().events.clone();
    let asked = flow.server.state().requests.len();
    // The owner is asked first; a script has to say that it means it.
    assert_eq!(flow.app.run("ui.checkCalendar", json!({})).unwrap()["pending"], "confirm");
    flow.app.dialog = None;
    assert!(flow.app.execute("ui.checkCalendar", json!({})).is_err());
    assert_eq!(flow.server.state().requests.len(), asked, "nothing is asked of the server before the yes");
    // Yes.
    assert_eq!(flow.run("ui.checkCalendar", json!({"confirmed": true}))["pending"], "calendarCheck");
    flow.app.poll_calendar_check();
    let report: Vec<String> = flow.app.calendar_check_report().to_vec();
    assert_eq!(report.len(), 9, "{report:#?}");
    assert_eq!(report[0], "avery@mail.example: The calendar connection works: what was sent arrived, and the server has it as it was sent.");
    for (line, starts) in report[1..8].iter().zip([
        "1. Make an appointment: OK (the server answered 201). sent: ",
        "2. Read it back: OK (the server answered 200). the server has: ",
        "3. Change its time: OK (the server answered 200). sent: ",
        "4. Read it back again: OK (the server answered 200). the server has: ",
        "5. Change its time once more, said in your time zone: OK (the server answered 200). sent: ",
        "6. Read it back a third time: OK (the server answered 200). the server has: ",
        "7. Delete it: OK (the server answered 204). deleted",
    ]) {
        assert!(line.starts_with(starts), "{line}");
    }
    assert!(report[5].contains("in the zone \"Eastern Standard Time\"") && report[6].ends_with("as it was sent"));
    assert_eq!(report[8], "The test appointment is gone again. Nothing else in the calendar was read or changed.");
    // Seven requests, about the one appointment it made; the calendar is as it was.
    let state = flow.server.state();
    let methods: Vec<&str> = state.requests.iter().skip(asked).map(|asked| asked.method.as_str()).collect();
    assert_eq!(methods, ["POST", "GET", "PATCH", "GET", "PATCH", "GET", "DELETE"]);
    assert_eq!(state.events, before);
    drop(state);

    // A server that keeps the time it had: the report says what was sent and what it has instead.
    flow.server.state().keeps_times = true;
    flow.run("ui.closeDialog", json!({}));
    flow.run("ui.checkCalendar", json!({"confirmed": true}));
    flow.app.poll_calendar_check();
    let report: Vec<String> = flow.app.calendar_check_report().to_vec();
    assert_eq!(report[0], "avery@mail.example: Something did not go through, or did not arrive as it was sent. The steps below say what.");
    assert!(
        report[4].starts_with("4. Read it back again: FAILED (the server answered 200). sent: ")
            && report[4].ends_with("(the start and the end differ)"),
        "{}",
        report[4]
    );
    assert_eq!(flow.status(), "The calendar connection check found a problem: see its report");
    assert_eq!(flow.server.state().events, before);
}

/// A second calendar of the account's own on the server, "Team", listed and ticked here: what
/// the server calls it, and its id here.
fn team(flow: &mut Flow) -> (String, Id) {
    let team = flow.server.state().add_calendar("Team", true, false);
    let list: CalendarsDownload = mailcraft_net::graph::calendars(TOKEN).unwrap();
    let account = flow.app.session.mailbox.account(flow.account).cloned().unwrap();
    flow.run("sync.applyCalendars", json!({"download": list.for_account(&account)}));
    let here = flow.app.session.mailbox.calendars.iter().find(|c| c.name == "Team").map(|c| c.id).unwrap();
    flow.run("calendar.showCalendar", json!({"id": here, "value": true}));
    flow.sync();
    (team, here)
}

#[test]
fn an_appointment_is_moved_to_another_calendar_of_the_account_and_back() {
    let mut flow = Flow::start();
    let (team, team_here) = team(&mut flow);
    let review = flow.here("Budget review");
    let main = flow.server.state().main_calendar();
    let (main_here, original) = (review.calendar, flow.there("Budget review").id);
    // Into the other calendar. The service has no way of moving an event: it is made there...
    assert_eq!(flow.run("calendar.edit", json!({"id": review.id, "calendar": team_here}))["moved"], true);
    flow.sync();
    // ...and seen there in what came down after it, while the original is still on the server.
    let there: Vec<String> = flow.all_there("Budget review").iter().map(|e| e.calendar.clone()).collect();
    assert_eq!((there.len(), flow.all_here("Budget review").len()), (2, 1));
    assert_eq!(flow.here("Budget review").calendar, team_here);
    // Then the original goes, out of the calendar it was in; and that is seen too.
    flow.sync();
    let there = flow.all_there("Budget review");
    assert_eq!(there.iter().map(|e| e.calendar.as_str()).collect::<Vec<_>>(), [team.as_str()]);
    let here = flow.all_here("Budget review");
    assert_eq!(here.iter().map(|e| (e.id, e.calendar, e.origin.id.as_str())).collect::<Vec<_>>(), [(review.id, team_here, there[0].id.as_str())]);
    // The created copy was verified, the original is gone, and nothing was put back or said.
    assert!(flow.settled(), "{}", flow.status());
    assert_eq!(flow.refusals(), []);
    let copy = there[0].id.clone();
    let paths = |flow: &Flow| -> Vec<(String, Vec<String>)> { flow.server.state().writes() };
    let piece = |text: &str| text.to_string();
    assert_eq!(
        paths(&flow),
        [
            (piece("POST"), vec![piece("v1.0"), piece("me"), piece("calendars"), team.clone(), piece("events")]),
            (piece("DELETE"), vec![piece("v1.0"), piece("me"), piece("events"), original.clone()]),
        ]
    );

    // And back, with a server whose answers lag: the views that follow the copy being made are
    // answered as the calendars were before it. The original is not deleted on the server's
    // word that it made the copy: it stays until the copy is seen.
    flow.server.state().stale_views = 2;
    flow.run("calendar.edit", json!({"id": review.id, "calendar": main_here}));
    flow.sync();
    assert_eq!((flow.all_there("Budget review").len(), paths(&flow).len()), (2, 3));
    assert_eq!((flow.all_here("Budget review").len(), flow.here("Budget review").calendar), (1, main_here));
    assert_eq!((flow.app.session.mailbox.event_checks.len(), flow.app.session.mailbox.event_changes.len()), (1, 1));
    // It is looked for again within seconds, and nothing is sent with that look.
    flow.at += 11.0 * SECOND;
    assert!(flow.app.calendar_tick_at(true, flow.at), "a copy that isn't seen yet is looked for again soon");
    flow.take_in();
    assert_eq!((flow.all_there("Budget review").len(), paths(&flow).len(), flow.app.session.mailbox.event_checks.len()), (2, 3, 0));
    // Seen: now the one in the other calendar goes, through that calendar.
    flow.sync();
    let there = flow.all_there("Budget review");
    assert_eq!(there.iter().map(|e| e.calendar.as_str()).collect::<Vec<_>>(), [main.as_str()]);
    let here = flow.all_here("Budget review");
    assert_eq!(here.iter().map(|e| (e.id, e.calendar, e.origin.id.as_str())).collect::<Vec<_>>(), [(review.id, main_here, there[0].id.as_str())]);
    assert!(flow.settled(), "{}", flow.status());
    assert_eq!(flow.refusals(), []);
    let writes = paths(&flow);
    assert_eq!(writes[2], (piece("POST"), vec![piece("v1.0"), piece("me"), piece("events")]));
    assert_eq!(writes[3], (piece("DELETE"), vec![piece("v1.0"), piece("me"), piece("calendars"), team.clone(), piece("events"), copy]));
    assert_eq!(writes.len(), 4);
}

#[test]
fn a_move_the_server_refuses_half_of_puts_the_appointment_back_and_says_so() {
    let mut flow = Flow::start();
    let (team, team_here) = team(&mut flow);
    let review = flow.here("Budget review");
    let main = flow.server.state().main_calendar();
    let original = flow.there("Budget review").id;
    let as_it_was = |flow: &Flow| {
        let there = flow.all_there("Budget review");
        assert_eq!(there.iter().map(|e| (e.id.as_str(), e.calendar.as_str())).collect::<Vec<_>>(), [(original.as_str(), main.as_str())]);
        let here = flow.all_here("Budget review");
        assert_eq!(
            here.iter().map(|e| (e.id, e.calendar, e.origin.id.as_str(), e.start)).collect::<Vec<_>>(),
            [(review.id, review.calendar, original.as_str(), review.start)]
        );
        let mailbox = &flow.app.session.mailbox;
        assert!(mailbox.event_changes.is_empty() && mailbox.event_checks.is_empty() && mailbox.clone().repair().is_empty());
    };

    // The copy is refused: the appointment is back where it was, and the owner is told.
    flow.server.state().fail_next.push(("POST".into(), 400, "ErrorInvalidRequest".into()));
    flow.run("calendar.edit", json!({"id": review.id, "calendar": team_here}));
    assert_eq!(flow.here("Budget review").calendar, team_here);
    flow.sync();
    let status = flow.status();
    assert!(
        status.starts_with("The server wouldn't take \"Budget review\" into the other calendar")
            && status.ends_with("so it wasn't moved: it is back where it was."),
        "{status}"
    );
    assert!(flow.app.has_sync_problems());
    as_it_was(&flow);
    assert_eq!(flow.server.state().writes().len(), 1, "the original was not touched");

    // The copy is made and seen, and then the original won't go: the copy is taken out again,
    // here and on the server, and the appointment is back where it was.
    flow.run("calendar.edit", json!({"id": review.id, "calendar": team_here}));
    flow.sync();
    assert_eq!(flow.all_there("Budget review").len(), 2);
    flow.server.state().fail_next.push(("DELETE".into(), 400, "ErrorInvalidRequest".into()));
    flow.sync();
    let status = flow.status();
    assert!(
        status.starts_with("The server wouldn't take \"Budget review\" out of the calendar it was in")
            && status.ends_with("so it wasn't moved: it is back there, and the copy in the other calendar was removed."),
        "{status}"
    );
    assert_eq!((flow.all_here("Budget review").len(), flow.here("Budget review").calendar), (1, review.calendar));
    assert_eq!(flow.all_there("Budget review").len(), 2, "the copy is still to go on the server");
    flow.sync();
    as_it_was(&flow);
    // The copy went through the calendar it was in.
    let last = flow.server.state().writes().last().cloned().unwrap();
    assert_eq!(
        (last.0.as_str(), last.1[..4].to_vec()),
        ("DELETE", vec!["v1.0".to_string(), "me".to_string(), "calendars".to_string(), team.clone()])
    );
}

#[test]
fn a_meeting_is_forwarded_and_another_time_proposed_only_on_the_owners_word() {
    let mut flow = Flow::start();
    // On the server: a meeting the owner organizes, and one of Bo's that the owner is invited to.
    {
        let mut state = flow.server.state();
        let main = state.main_calendar();
        let (planning, board) = (state.add_event(&main, "Planning", utc(10, 15, 14, 0)), state.add_event(&main, "Board", utc(10, 16, 14, 0)));
        for event in state.events.iter_mut() {
            if event.id == planning {
                event.attendees = vec![json!({"emailAddress": {"address": "bo@example.org", "name": "Bo"}, "type": "required"})];
            }
            if event.id == board {
                (event.is_organizer, event.response) = (false, "none".into());
                event.attendees = vec![json!({"emailAddress": {"address": "avery@mail.example", "name": "Avery"}, "type": "required"})];
            }
        }
    }
    flow.sync();
    let (planning, board) = (flow.here("Planning"), flow.here("Board"));
    assert!(board.origin.guest && !planning.origin.guest);
    let written = |flow: &Flow| {
        let state = flow.server.state();
        (state.writes().len(), state.mail.len())
    };

    // Without the owner's yes the commands refuse, and no request reaches the server.
    let forward = json!({"id": planning.id, "to": "Eve <eve@example.org>", "comment": "Do join us."});
    let propose = json!({"id": board.id, "start": "2026-10-16T11:00", "end": "2026-10-16T12:00", "comment": "An hour later?"});
    for (command, params, why) in [
        ("calendar.forward", &forward, "Forwarding writes to 1 person. Nothing was sent."),
        ("calendar.propose", &propose, "Proposing another time writes to the meeting's organizer. Nothing was sent."),
    ] {
        let refused = flow.app.execute(command, params.clone()).unwrap_err().to_string();
        assert!(refused.contains(why), "{command}: {refused}");
    }
    assert!(flow.app.session.mailbox.event_changes.is_empty());
    flow.sync();
    assert_eq!(written(&flow), (0, 0));
    // Nor when a mailbox file notes them without it: they are given up, with a word to the
    // owner, and still nothing is asked of the server.
    let note = |kind: EventChangeKind, event: &Event| EventChange {
        account: event.origin.account,
        kind,
        event: event.id,
        remote: event.origin.id.clone(),
        token: "0f8fad5b-d9cb-469f-a165-70867728950e".into(),
        base: Some(Box::new(event.clone())),
        ..Default::default()
    };
    let time = EventTime { start: utc(10, 16, 15, 0), end: utc(10, 16, 16, 0), all_day: false };
    flow.app.session.mailbox.event_changes = vec![
        EventChange { to: vec![Address::new("Eve", "eve@example.org")], ..note(EventChangeKind::Forward, &planning) },
        EventChange { response: Response::Tentative, proposed: Some(time), ..note(EventChangeKind::Respond, &board) },
    ];
    flow.sync();
    assert_eq!(written(&flow), (0, 0));
    assert!(flow.app.session.mailbox.event_changes.is_empty() && flow.app.has_sync_problems());

    // With it: the meeting is forwarded by the server, and the organizer hears the answer and
    // the other time, each asked of the service as its reference has it.
    let with = |params: &Value| {
        let mut params = params.clone();
        params["send"] = json!(true);
        params
    };
    flow.run("calendar.forward", with(&forward));
    flow.run("calendar.propose", with(&propose));
    flow.sync();
    assert_eq!(
        flow.server.state().mail,
        [
            "forward:Planning:1:Do join us.",
            "response:Board:tentativelyAccept:An hour later?",
            "proposal:Board:2026-10-16T15:00:00Z/2026-10-16T16:00:00Z"
        ]
    );
    let asked: Vec<(String, Value)> = {
        let state = flow.server.state();
        state
            .requests
            .iter()
            .filter(|asked| asked.method == "POST")
            .map(|asked| (asked.path.last().cloned().unwrap_or_default(), asked.json()))
            .collect()
    };
    let slot =
        json!({"start": {"dateTime": "2026-10-16T15:00:00", "timeZone": "UTC"}, "end": {"dateTime": "2026-10-16T16:00:00", "timeZone": "UTC"}});
    assert_eq!(
        asked,
        [
            (
                "forward".to_string(),
                json!({"ToRecipients": [{"EmailAddress": {"Address": "eve@example.org", "Name": "Eve"}}], "Comment": "Do join us."})
            ),
            ("tentativelyAccept".to_string(), json!({"comment": "An hour later?", "sendResponse": true, "proposedNewTime": slot})),
        ]
    );
    assert_eq!(flow.there("Board").response, "tentativelyAccepted");
    assert_eq!(flow.refusals(), []);
    let mailbox = &flow.app.session.mailbox;
    assert!(mailbox.event_changes.is_empty() && mailbox.event_checks.is_empty());
    // An organizer who takes no proposals is not proposed to, whatever is said.
    flow.server.state().events.iter_mut().find(|event| event.subject == "Board").unwrap().no_new_times = true;
    flow.sync();
    let refused = flow.app.execute("calendar.propose", with(&propose)).unwrap_err().to_string();
    assert!(refused.contains("The organizer of this meeting doesn't take proposals of another time."), "{refused}");
    assert_eq!(written(&flow), (2, 3));
}

#[test]
fn a_teams_meeting_its_importance_and_what_is_asked_of_the_people_invited_are_what_the_server_has() {
    let mut flow = Flow::start();
    // A meeting that matters, held online, to which no answers are wanted and no other times.
    let meeting = json!({
        "title": "Launch", "start": "2026-10-15T10:00", "end": "2026-10-15T11:00", "attendees": "Bo <bo@example.org>", "importance": "high",
        "requestResponses": false, "allowProposals": false, "online": true, "send": true,
    });
    let launch = flow.run("calendar.newMeeting", meeting)["id"].as_u64().unwrap();
    flow.sync();
    let there = flow.there("Launch");
    assert_eq!(
        (there.importance.as_str(), there.no_response_requested, there.no_new_times, there.online, there.online_provider.as_str()),
        ("high", true, true, true, "teamsForBusiness")
    );
    assert_eq!(flow.server.state().mail, ["invitation:Launch:1"]);
    // The link to join by is the server's, and is here as it is there.
    let here = flow.here("Launch");
    assert_eq!(here.join_link(), Some(there.join_url.as_str()));
    assert_eq!(flow.run("calendar.join", json!({"id": launch}))["url"], there.join_url.as_str());
    assert_eq!(
        (here.importance, here.response_options.request_responses, here.response_options.allow_proposals),
        (mailcraft_model::Importance::High, false, false)
    );
    assert!(flow.settled(), "{}", flow.status());
    assert_eq!(flow.refusals(), []);

    // What it matters, and that answers are wanted after all: changes like any other, and what
    // was verified stays as it is here when it comes down again.
    flow.run("calendar.importance", json!({"id": launch, "value": "low", "send": true}));
    flow.run("calendar.responseOptions", json!({"id": launch, "requestResponses": true, "send": true}));
    flow.sync();
    let there = flow.there("Launch");
    assert_eq!((there.importance.as_str(), there.no_response_requested, there.online), ("low", false, true));
    flow.sync();
    let here = flow.here("Launch");
    assert_eq!(
        (here.importance, here.response_options.request_responses, here.join_link()),
        (mailcraft_model::Importance::Low, true, Some(there.join_url.as_str()))
    );
    assert!(flow.settled(), "{}", flow.status());
    assert_eq!(flow.refusals(), []);

    // Whether another time may be proposed is said when a meeting is made; the reference
    // doesn't list it among what an update may change. A service that goes by that refuses,
    // and it is put back here with a word, not left looking as if it had been changed.
    flow.run("calendar.responseOptions", json!({"id": launch, "allowProposals": true, "send": true}));
    flow.sync();
    let status = flow.status();
    assert!(status.starts_with("The server wouldn't take the change to \"Launch\"") && status.ends_with("It was put back as it was."), "{status}");
    assert!(!flow.here("Launch").response_options.allow_proposals && flow.there("Launch").no_new_times);
    assert!(flow.app.session.mailbox.event_changes.is_empty() && flow.app.session.mailbox.event_checks.is_empty());
}

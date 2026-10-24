// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A shared mailbox's calendars from the window to the server and back (#70): the window's
//! calendar timer, this host's Microsoft Graph job and `mailcraft-net`, against the stand-in
//! (`mailcraft_net::graph::standin`). Nothing here leaves this machine, and nobody is signed in.

use mailcraft_engine::Session;
use mailcraft_model::sync::Tokens;
use mailcraft_model::{Id, Mailbox, Rights};
use mailcraft_net::graph::standin::{CalendarAccess, StandIn, TOKEN};
use mailcraft_ui_egui::{GraphRequest, MailApp, Services};
use serde_json::json;

const NOW: i64 = 1_791_993_600;
const SHARED: &str = "support@example.org";
const OWNER: &str = "avery@mail.example";

fn utc(day: u8, hour: u8) -> i64 {
    mailcraft_time::DateTime::new(mailcraft_time::Date::new(2026, 10, day).unwrap(), hour, 0, 0).unwrap().to_timestamp(0)
}

struct Flow {
    app: MailApp,
    server: StandIn,
    shared: Id,
    at: f64,
    _route: mailcraft_net::graph::standin::Routed,
}

/// The window, a Microsoft account (whose own calendar has been looked at already) with a shared
/// mailbox under it, and the stand-in in Graph's place with `access` to the shared mailbox's
/// calendars.
fn flow(access: CalendarAccess) -> Flow {
    let server = StandIn::start().unwrap();
    {
        let mut state = server.state();
        state.owner = OWNER.into();
        state.add_other(SHARED, true);
        let main = state.add_other_calendar(SHARED, "Calendar", true);
        let team = state.add_other_calendar(SHARED, "Team", false);
        state.add_other_event(SHARED, &main, "Stand-up", utc(14, 17), "busy");
        state.add_other_event(SHARED, &main, "Away day", utc(15, 9), "outOfOffice");
        state.add_other_event(SHARED, &team, "Rota", utc(14, 18), "busy");
        state.share_calendars(SHARED, access);
        let own = state.main_calendar();
        state.add_event(&own, "Dentist", utc(14, 19));
    }
    let route = server.route();
    let mut app = MailApp::new(Session::at(Mailbox::with_local_account("", ""), NOW, 0), Services::default());
    app.autosave = false;
    app.services.graph = Some(Box::new(|request: GraphRequest| {
        let (tx, rx) = std::sync::mpsc::channel();
        let _ = tx.send(super::graph_in(request, "UTC"));
        rx
    }));
    let owner = app.execute("account.add", json!({"email": OWNER, "kind": "microsoft"})).unwrap()["id"].as_u64().unwrap();
    let fresh = Tokens { access: TOKEN.into(), refresh: "ref".into(), expires_at: mailcraft_time::now() + 3_600, ..Default::default() };
    app.seed_graph_token(owner, fresh.clone());
    let shared = app.execute("account.addSharedMailbox", json!({"address": SHARED})).unwrap()["id"].as_u64().unwrap();
    app.seed_shared_calendar_token(shared, fresh);
    app.execute("view.calendar", json!({})).unwrap();
    let mut flow = Flow { app, server, shared, at: mailcraft_ui_egui::now_ms(), _route: route };
    // The owner's own calendar comes down first (nothing of it is what these look at).
    assert!(flow.look());
    flow.take_in();
    flow.server.state().requests.clear();
    flow
}

impl Flow {
    fn look(&mut self) -> bool {
        // (A second on: the owner's own calendar, asked for half a minute apart, is not due.)
        self.at += 1_000.0;
        self.app.calendar_tick_at(true, self.at)
    }

    fn take_in(&mut self) {
        for _ in 0..4 {
            self.app.poll_calendar();
        }
    }

    fn calendar(&self, name: &str) -> mailcraft_model::Calendar {
        let all = &self.app.session.mailbox.calendars;
        all.iter().find(|c| c.origin.account == self.shared && c.name == name).cloned().unwrap_or_else(|| panic!("no calendar {name}"))
    }

    fn titles(&self) -> Vec<String> {
        let mut titles: Vec<String> =
            self.app.session.mailbox.events.values().filter(|e| e.origin.account == self.shared).map(|e| e.title.clone()).collect();
        titles.sort();
        titles
    }

    /// Every address the stand-in was asked, by method and path.
    fn asked(&self) -> Vec<String> {
        self.server.state().requests.iter().map(|asked| format!("{} {}", asked.method, asked.path.join("/"))).collect()
    }
}

#[test]
fn a_shared_mailboxs_calendars_come_down_when_ticked_and_nothing_is_asked_at_me() {
    let mut flow = flow(CalendarAccess::Open);
    // Nothing is asked of the server for a calendar nobody ticked.
    assert!(!flow.look() && flow.asked().is_empty());
    let placeholder = flow.calendar(SHARED).id;
    flow.app.execute("calendar.showCalendar", json!({"id": placeholder})).unwrap();
    assert!(flow.look());
    flow.take_in();
    // The mailbox's calendars are listed, unticked but for the one that was ticked, and the
    // ticked one's events are there; the other one's are not asked for.
    assert_eq!(flow.calendar("Calendar").rights, Rights::Edit);
    assert!(flow.calendar("Calendar").visible && !flow.calendar("Team").visible);
    assert_eq!(flow.titles(), ["Away day", "Stand-up"]);
    assert_eq!(flow.calendar("Team").rights, Rights::Read, "read where the owner may not change it");
    let seen = flow.asked();
    assert!(!seen.is_empty() && seen.iter().all(|line| line.contains("users/support@example.org")), "{seen:?}");
    assert!(seen.iter().all(|line| !line.contains("/me/") && !line.ends_with("/me")), "nothing at /me: {seen:?}");
    assert!(
        seen.iter().all(|line| !line.contains("contacts") && !line.contains("masterCategories") && !line.contains("mailboxSettings")),
        "{seen:?}"
    );
    // The owner's own calendar was never mixed with it.
    assert!(!flow.titles().iter().any(|t| t == "Dentist"));
    // A change to one is refused: nothing waits to go, and nothing is written to the server.
    let stand_up = flow.app.session.mailbox.events.values().find(|e| e.title == "Stand-up").map(|e| e.id).unwrap();
    assert!(flow.app.execute("calendar.edit", json!({"id": stand_up, "title": "Changed"})).is_err());
    assert!(flow.app.session.mailbox.event_changes.is_empty());
    assert!(flow.server.state().writes().is_empty());
}

#[test]
fn a_calendar_shared_for_free_and_busy_only_is_blocks_without_subjects_that_are_only_read() {
    let mut flow = flow(CalendarAccess::FreeBusy);
    let placeholder = flow.calendar(SHARED).id;
    flow.app.execute("calendar.showCalendar", json!({"id": placeholder})).unwrap();
    assert!(flow.look());
    flow.take_in();
    // Its list is refused; the main calendar is its free and busy times.
    assert_eq!(flow.calendar(SHARED).rights, Rights::FreeBusy);
    let blocks: Vec<(String, mailcraft_model::ShowAs)> =
        flow.app.session.mailbox.events.values().filter(|e| e.origin.account == flow.shared).map(|e| (e.title.clone(), e.show_as)).collect();
    assert_eq!(blocks.len(), 2, "{blocks:?}");
    assert!(blocks.iter().all(|(title, _)| title.is_empty()), "no subjects: {blocks:?}");
    assert!(blocks.iter().any(|(_, show)| *show == mailcraft_model::ShowAs::OutOfOffice));
    let id = flow.app.session.mailbox.events.values().find(|e| e.origin.account == flow.shared).map(|e| e.id).unwrap();
    assert!(flow.app.execute("calendar.delete", json!({"id": id})).is_err());
    let seen = flow.asked();
    assert!(seen.iter().any(|line| line.ends_with("calendar/getSchedule")), "{seen:?}");
    assert!(seen.iter().all(|line| line.contains("users/support@example.org")), "{seen:?}");
    assert!(!flow.app.has_sync_problems());
}

#[test]
fn a_shared_mailbox_nothing_is_shared_from_says_so_and_asks_nothing_at_me() {
    let mut flow = flow(CalendarAccess::Denied);
    let placeholder = flow.calendar(SHARED).id;
    flow.app.execute("calendar.showCalendar", json!({"id": placeholder})).unwrap();
    assert!(flow.look());
    flow.take_in();
    assert!(flow.titles().is_empty());
    let seen = flow.asked();
    assert!(!seen.is_empty() && seen.iter().all(|line| line.contains("users/support@example.org")), "{seen:?}");
    // It is said in Send/Receive's problems.
    assert!(flow.app.has_sync_problems());
}

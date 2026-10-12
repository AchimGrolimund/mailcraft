//! Writing events and to-dos as iCalendar text.

use mailcraft_model::{Address, Event, Importance, Response, ShowAs, Task};
use mailcraft_time::DateTime;

use crate::WriteOptions;
use crate::lines::{fold, param_value};
use crate::rrule;
use crate::values::{escape_text, format_date, format_local, format_utc, valid_time};

/// Write a `VCALENDAR` (`VERSION:2.0`, `PRODID:-//MailCraft//MailCraft//EN`) with CRLF line endings
/// and lines folded at 75 octets on char boundaries.
///
/// Times are written floating (no zone), as the model keeps them; all-day events as dates. An
/// event without a uid gets `mailcraft-<id>@mailcraft.local`, a to-do
/// `mailcraft-task-<id>@mailcraft.local`. Text is escaped; control characters other than line
/// breaks and tabs are dropped. A to-do's link to a message isn't written.
pub fn write(events: &[&Event], tasks: &[&Task], opts: &WriteOptions) -> String {
    let mut out = String::new();
    fold(&mut out, "BEGIN:VCALENDAR");
    fold(&mut out, "VERSION:2.0");
    fold(&mut out, "PRODID:-//MailCraft//MailCraft//EN");
    let method: String = opts.method.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-').map(|c| c.to_ascii_uppercase()).collect();
    if !method.is_empty() {
        fold(&mut out, &format!("METHOD:{method}"));
    }
    let stamp = format_utc(opts.now);
    for event in events {
        write_event(&mut out, event, &stamp);
    }
    for task in tasks {
        write_task(&mut out, task, &stamp);
    }
    fold(&mut out, "END:VCALENDAR");
    out
}

/// `NAME:escaped text`.
fn text(out: &mut String, name: &str, value: &str) {
    fold(out, &format!("{name}:{}", escape_text(value)));
}

fn categories(out: &mut String, list: &[String]) {
    let names: Vec<String> = list.iter().map(|name| escape_text(name.trim())).filter(|name| !name.is_empty()).collect();
    if !names.is_empty() {
        fold(out, &format!("CATEGORIES:{}", names.join(",")));
    }
}

/// `;CN=Name:mailto:address`, the tail of an `ORGANIZER` or `ATTENDEE` line.
fn person(address: &Address) -> String {
    let name = address.name.trim();
    let email: String = address.email.trim().chars().filter(|c| !c.is_control()).collect();
    if name.is_empty() { format!(":mailto:{email}") } else { format!(";CN={}:mailto:{email}", param_value(name)) }
}

fn is_blank(address: &Address) -> bool {
    address.name.trim().is_empty() && address.email.trim().is_empty()
}

fn write_event(out: &mut String, e: &Event, stamp: &str) {
    fold(out, "BEGIN:VEVENT");
    let uid = e.uid.trim();
    if uid.is_empty() {
        text(out, "UID", &format!("mailcraft-{}@mailcraft.local", e.id));
    } else {
        text(out, "UID", uid);
    }
    fold(out, &format!("DTSTAMP:{stamp}"));
    let start = valid_time(e.start);
    let end = valid_time(e.end).max(start);
    if e.all_day {
        // The end is exclusive: a time left on it still counts its day in.
        let mut last = if end.seconds_of_day() > 0 { end.date.add_days(1) } else { end.date };
        if last <= start.date {
            last = start.date.add_days(1);
        }
        fold(out, &format!("DTSTART;VALUE=DATE:{}", format_date(start.date)));
        fold(out, &format!("DTEND;VALUE=DATE:{}", format_date(last)));
    } else {
        fold(out, &format!("DTSTART:{}", format_local(start)));
        fold(out, &format!("DTEND:{}", format_local(end)));
    }
    if let Some(recurrence) = &e.recurrence {
        fold(out, &format!("RRULE:{}", rrule::format_for(recurrence, start, e.all_day)));
    }
    for day in &e.exceptions {
        // The same value type as DTSTART, so other programs match the day to an occurrence.
        if e.all_day {
            fold(out, &format!("EXDATE;VALUE=DATE:{}", format_date(*day)));
        } else {
            fold(out, &format!("EXDATE:{}", format_local(DateTime { date: *day, ..start })));
        }
    }
    text(out, "SUMMARY", &e.title);
    if !e.location.is_empty() {
        text(out, "LOCATION", &e.location);
    }
    if !e.description.is_empty() {
        text(out, "DESCRIPTION", &e.description);
    }
    categories(out, &e.categories);
    if e.private {
        fold(out, "CLASS:PRIVATE");
    }
    if e.cancelled {
        fold(out, "STATUS:CANCELLED");
    }
    fold(out, if e.show_as == ShowAs::Free { "TRANSP:TRANSPARENT" } else { "TRANSP:OPAQUE" });
    let busy = match e.show_as {
        ShowAs::Free => "FREE",
        ShowAs::Tentative => "TENTATIVE",
        ShowAs::Busy => "BUSY",
        ShowAs::OutOfOffice => "OOF",
        ShowAs::WorkingElsewhere => "WORKINGELSEWHERE",
    };
    fold(out, &format!("X-MICROSOFT-CDO-BUSYSTATUS:{busy}"));
    if let Some(organizer) = e.organizer.as_ref().filter(|o| !is_blank(o)) {
        fold(out, &format!("ORGANIZER{}", person(organizer)));
    }
    for attendee in e.attendees.iter().filter(|a| !is_blank(&a.address)) {
        let role = if attendee.optional { "OPT-PARTICIPANT" } else { "REQ-PARTICIPANT" };
        let status = match attendee.response {
            Response::None => "NEEDS-ACTION;RSVP=TRUE",
            Response::Accepted => "ACCEPTED",
            Response::Tentative => "TENTATIVE",
            Response::Declined => "DECLINED",
        };
        fold(out, &format!("ATTENDEE;ROLE={role};PARTSTAT={status}{}", person(&attendee.address)));
    }
    if let Some(minutes) = e.reminder_minutes {
        fold(out, "BEGIN:VALARM");
        fold(out, "ACTION:DISPLAY");
        fold(out, "DESCRIPTION:Reminder");
        fold(out, &format!("TRIGGER:-PT{minutes}M"));
        fold(out, "END:VALARM");
    }
    fold(out, "END:VEVENT");
}

fn write_task(out: &mut String, t: &Task, stamp: &str) {
    fold(out, "BEGIN:VTODO");
    let uid = t.uid.trim();
    if uid.is_empty() {
        text(out, "UID", &format!("mailcraft-task-{}@mailcraft.local", t.id));
    } else {
        text(out, "UID", uid);
    }
    fold(out, &format!("DTSTAMP:{stamp}"));
    text(out, "SUMMARY", &t.title);
    if !t.notes.is_empty() {
        text(out, "DESCRIPTION", &t.notes);
    }
    if let Some(start) = t.start {
        fold(out, &format!("DTSTART;VALUE=DATE:{}", format_date(start)));
    }
    if let Some(due) = t.due {
        fold(out, &format!("DUE;VALUE=DATE:{}", format_date(due)));
    }
    match t.completed {
        Some(at) => {
            fold(out, &format!("COMPLETED:{}", format_utc(at)));
            fold(out, "PERCENT-COMPLETE:100");
            fold(out, "STATUS:COMPLETED");
        }
        None => fold(out, "STATUS:NEEDS-ACTION"),
    }
    match t.importance {
        Importance::High => fold(out, "PRIORITY:1"),
        Importance::Low => fold(out, "PRIORITY:9"),
        Importance::Normal => {}
    }
    categories(out, &t.categories);
    if let Some(at) = t.reminder {
        fold(out, "BEGIN:VALARM");
        fold(out, "ACTION:DISPLAY");
        fold(out, "DESCRIPTION:Reminder");
        fold(out, &format!("TRIGGER;VALUE=DATE-TIME:{}", format_utc(at)));
        fold(out, "END:VALARM");
    }
    fold(out, "END:VTODO");
}

#[cfg(test)]
mod tests {
    use mailcraft_model::{Attendee, Freq, Recurrence};
    use mailcraft_time::{Date, Weekday};

    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    fn at(y: i32, m: u8, day: u8, h: u8, min: u8) -> DateTime {
        DateTime::new(d(y, m, day), h, min, 0).unwrap()
    }

    fn options() -> WriteOptions {
        WriteOptions { method: String::new(), now: at(2026, 10, 11, 12, 0).to_timestamp(0) }
    }

    /// The unfolded lines of what `write` gives for one event.
    fn event_lines(e: &Event) -> Vec<String> {
        crate::lines::Lines::new(&write(&[e], &[], &options())).collect()
    }

    fn task_lines(t: &Task) -> Vec<String> {
        crate::lines::Lines::new(&write(&[], &[t], &options())).collect()
    }

    fn meeting() -> Event {
        Event {
            id: 42,
            calendar: 3,
            uid: "planning-2026@example.org".into(),
            title: "Quarterly planning; budget, hiring".into(),
            location: "Room 4.12, North Building".into(),
            description: "Agenda:\n1. Budget\n2. Hiring \\ open roles".into(),
            start: at(2026, 10, 12, 9, 0),
            end: at(2026, 10, 12, 10, 30),
            organizer: Some(Address::new("Okafor, Nadia", "nadia.okafor@example.org")),
            attendees: vec![
                Attendee { address: Address::new("Tomas Lindqvist", "tomas.lindqvist@example.org"), optional: false, response: Response::None },
                Attendee { address: Address::new("Mei \"MT\" Tanaka", "mei.tanaka@example.org"), optional: true, response: Response::Tentative },
                Attendee { address: Address::new("", "ibrahim@example.org"), optional: false, response: Response::Declined },
                Attendee { address: Address::new("Zoë: ops", "zoe@example.org"), optional: false, response: Response::Accepted },
                Attendee::default(),
            ],
            recurrence: Some(Recurrence { interval: 2, by_day: vec![Weekday::Mon, Weekday::Wed], count: Some(8), ..Recurrence::every(Freq::Weekly) }),
            exceptions: vec![d(2026, 10, 26)],
            reminder_minutes: Some(15),
            show_as: ShowAs::OutOfOffice,
            categories: vec!["Work".into(), " Planning, 2026 ".into(), "  ".into()],
            private: true,
            cancelled: true,
            ..Default::default()
        }
    }

    #[test]
    fn writes_a_whole_event() {
        assert_eq!(
            event_lines(&meeting()),
            [
                "BEGIN:VCALENDAR",
                "VERSION:2.0",
                "PRODID:-//MailCraft//MailCraft//EN",
                "BEGIN:VEVENT",
                "UID:planning-2026@example.org",
                "DTSTAMP:20261011T120000Z",
                "DTSTART:20261012T090000",
                "DTEND:20261012T103000",
                "RRULE:FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=8",
                "EXDATE:20261026T090000",
                "SUMMARY:Quarterly planning\\; budget\\, hiring",
                "LOCATION:Room 4.12\\, North Building",
                "DESCRIPTION:Agenda:\\n1. Budget\\n2. Hiring \\\\ open roles",
                "CATEGORIES:Work,Planning\\, 2026",
                "CLASS:PRIVATE",
                "STATUS:CANCELLED",
                "TRANSP:OPAQUE",
                "X-MICROSOFT-CDO-BUSYSTATUS:OOF",
                "ORGANIZER;CN=\"Okafor, Nadia\":mailto:nadia.okafor@example.org",
                "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=NEEDS-ACTION;RSVP=TRUE;CN=Tomas Lindqvist:mailto:tomas.lindqvist@example.org",
                "ATTENDEE;ROLE=OPT-PARTICIPANT;PARTSTAT=TENTATIVE;CN=Mei ^'MT^' Tanaka:mailto:mei.tanaka@example.org",
                "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=DECLINED:mailto:ibrahim@example.org",
                "ATTENDEE;ROLE=REQ-PARTICIPANT;PARTSTAT=ACCEPTED;CN=\"Zoë: ops\":mailto:zoe@example.org",
                "BEGIN:VALARM",
                "ACTION:DISPLAY",
                "DESCRIPTION:Reminder",
                "TRIGGER:-PT15M",
                "END:VALARM",
                "END:VEVENT",
                "END:VCALENDAR",
            ]
        );
    }

    #[test]
    fn lines_end_in_crlf_and_stay_within_75_octets() {
        let mut e = meeting();
        e.description = "Ünïcödé — 東京 🎉 ".repeat(60);
        let out = write(&[&e], &[], &options());
        assert!(out.ends_with("END:VCALENDAR\r\n"));
        assert!(!out.replace("\r\n", "").contains(['\r', '\n']));
        assert!(out.split("\r\n").all(|line| line.len() <= 75), "{out}");
        assert!(out.contains("\r\n "));
    }

    #[test]
    fn a_plain_event_writes_only_what_it_has() {
        let e = Event { id: 7, start: at(2026, 10, 12, 9, 0), end: at(2026, 10, 12, 9, 0), ..Default::default() };
        assert_eq!(
            event_lines(&e).get(3..12).unwrap(),
            [
                "BEGIN:VEVENT",
                "UID:mailcraft-7@mailcraft.local",
                "DTSTAMP:20261011T120000Z",
                "DTSTART:20261012T090000",
                "DTEND:20261012T090000",
                "SUMMARY:",
                "TRANSP:OPAQUE",
                "X-MICROSOFT-CDO-BUSYSTATUS:BUSY",
                "END:VEVENT"
            ]
        );
        let free = Event { show_as: ShowAs::Free, uid: "   ".into(), ..e.clone() };
        let lines = event_lines(&free);
        assert!(lines.contains(&"TRANSP:TRANSPARENT".to_string()) && lines.contains(&"X-MICROSOFT-CDO-BUSYSTATUS:FREE".to_string()));
        assert!(lines.contains(&"UID:mailcraft-7@mailcraft.local".to_string()));
        // An organizer with nothing in it isn't one.
        let blank = Event { organizer: Some(Address::default()), ..e };
        assert!(!event_lines(&blank).iter().any(|l| l.starts_with("ORGANIZER")));
    }

    #[test]
    fn all_day_events_are_dates() {
        let e = Event {
            all_day: true,
            start: at(2026, 10, 12, 0, 0),
            end: at(2026, 10, 15, 0, 0),
            recurrence: Some(Recurrence { until: Some(d(2030, 10, 12)), ..Recurrence::every(Freq::Yearly) }),
            exceptions: vec![d(2027, 10, 12), d(2028, 10, 12)],
            ..Default::default()
        };
        let lines = event_lines(&e);
        for want in [
            "DTSTART;VALUE=DATE:20261012",
            "DTEND;VALUE=DATE:20261015",
            "RRULE:FREQ=YEARLY;UNTIL=20301012",
            "EXDATE;VALUE=DATE:20271012",
            "EXDATE;VALUE=DATE:20281012",
        ] {
            assert!(lines.contains(&want.to_string()), "{want} in {lines:?}");
        }
        // An end that isn't after the start, or has a time left on it, still gives whole days.
        let same_day = Event { end: at(2026, 10, 12, 0, 0), recurrence: None, ..e.clone() };
        assert!(event_lines(&same_day).contains(&"DTEND;VALUE=DATE:20261013".to_string()));
        let with_time = Event { end: at(2026, 10, 13, 17, 0), ..e.clone() };
        assert!(event_lines(&with_time).contains(&"DTEND;VALUE=DATE:20261014".to_string()));
        let backwards = Event { end: at(2026, 10, 1, 0, 0), ..e };
        assert!(event_lines(&backwards).contains(&"DTEND;VALUE=DATE:20261013".to_string()));
    }

    #[test]
    fn a_timed_series_ends_with_its_last_day_and_spells_out_its_weekday() {
        let e = Event {
            start: at(2026, 10, 13, 9, 30),
            end: at(2026, 10, 13, 8, 0),
            recurrence: Some(Recurrence { nth: Some(2), until: Some(d(2027, 6, 30)), ..Recurrence::every(Freq::Monthly) }),
            ..Default::default()
        };
        let lines = event_lines(&e);
        assert!(lines.contains(&"RRULE:FREQ=MONTHLY;BYDAY=2TU;UNTIL=20270630T235959".to_string()), "{lines:?}");
        // An end before the start is the start.
        assert!(lines.contains(&"DTEND:20261013T093000".to_string()));
    }

    #[test]
    fn the_method_is_a_clean_token() {
        let with = |method: &str| write(&[], &[], &WriteOptions { method: method.into(), now: 0 });
        assert_eq!(with("request"), "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//MailCraft//MailCraft//EN\r\nMETHOD:REQUEST\r\nEND:VCALENDAR\r\n");
        assert!(with(" REPLY\r\nBEGIN:VEVENT ").contains("METHOD:REPLYBEGINVEVENT\r\n"));
        assert!(!with("").contains("METHOD"));
        assert!(!with(" \r\n:;").contains("METHOD"));
    }

    #[test]
    fn text_cannot_inject_lines() {
        let e = Event {
            uid: "id\r\nATTENDEE:mailto:evil@example.org".into(),
            title: "Hello\r\nEND:VEVENT\r\nBEGIN:VEVENT".into(),
            location: "x\rEND:VCALENDAR".into(),
            organizer: Some(Address { name: "A\r\nX-INJECTED:1".into(), email: "a@example.org\r\nX-INJECTED:2".into() }),
            categories: vec!["a\nb".into()],
            ..Default::default()
        };
        let lines = event_lines(&e);
        assert_eq!(lines.iter().filter(|l| l.starts_with("BEGIN:") || l.starts_with("END:")).count(), 4, "{lines:?}");
        assert!(!lines.iter().any(|l| l.starts_with("ATTENDEE") || l.starts_with("X-INJECTED")));
        assert!(lines.contains(&"SUMMARY:Hello\\nEND:VEVENT\\nBEGIN:VEVENT".to_string()));
        assert!(lines.contains(&"ORGANIZER;CN=\"A^nX-INJECTED:1\":mailto:a@example.orgX-INJECTED:2".to_string()), "{lines:?}");
    }

    #[test]
    fn hostile_dates_are_written_as_real_ones() {
        let hostile = DateTime { date: Date { year: -5, month: 200, day: 0 }, hour: 77, minute: 88, second: 99 };
        let e = Event {
            start: hostile,
            end: hostile,
            all_day: true,
            exceptions: vec![Date { year: i32::MAX, month: 0, day: 255 }],
            recurrence: Some(Recurrence { until: Some(hostile.date), ..Default::default() }),
            ..Default::default()
        };
        let back = crate::parse(&write(&[&e, &Event { all_day: false, ..e.clone() }], &[], &options()), &|_| 0);
        assert_eq!(back.events.len(), 2);
        assert!(back.events.iter().all(|e| e.exceptions.len() == 1 && e.recurrence.as_ref().is_some_and(|r| r.until.is_some())));
        let t =
            Task { start: Some(hostile.date), due: Some(hostile.date), completed: Some(i64::MIN), reminder: Some(i64::MAX), ..Default::default() };
        let back = crate::parse(&write(&[], &[&t], &options()), &|_| 0);
        assert!(
            back.tasks[0].start.is_some() && back.tasks[0].due.is_some() && back.tasks[0].completed.is_some() && back.tasks[0].reminder.is_some()
        );
    }

    #[test]
    fn writes_tasks() {
        let done = Task {
            id: 9,
            title: "Send the budget draft".into(),
            notes: "Numbers first; then the summary.\nAsk Mei.".into(),
            start: Some(d(2026, 10, 10)),
            due: Some(d(2026, 10, 16)),
            completed: Some(at(2026, 10, 15, 17, 45).to_timestamp(0)),
            importance: Importance::High,
            categories: vec!["Work".into(), "Finance".into()],
            reminder: Some(at(2026, 10, 15, 16, 0).to_timestamp(0)),
            message: Some(77),
            uid: "task-1@example.org".into(),
        };
        assert_eq!(
            task_lines(&done).get(3..19).unwrap(),
            [
                "BEGIN:VTODO",
                "UID:task-1@example.org",
                "DTSTAMP:20261011T120000Z",
                "SUMMARY:Send the budget draft",
                "DESCRIPTION:Numbers first\\; then the summary.\\nAsk Mei.",
                "DTSTART;VALUE=DATE:20261010",
                "DUE;VALUE=DATE:20261016",
                "COMPLETED:20261015T174500Z",
                "PERCENT-COMPLETE:100",
                "STATUS:COMPLETED",
                "PRIORITY:1",
                "CATEGORIES:Work,Finance",
                "BEGIN:VALARM",
                "ACTION:DISPLAY",
                "DESCRIPTION:Reminder",
                "TRIGGER;VALUE=DATE-TIME:20261015T160000Z",
            ]
        );
        let plain = Task { id: 5, importance: Importance::Low, ..Default::default() };
        assert_eq!(
            task_lines(&plain).get(3..9).unwrap(),
            ["BEGIN:VTODO", "UID:mailcraft-task-5@mailcraft.local", "DTSTAMP:20261011T120000Z", "SUMMARY:", "STATUS:NEEDS-ACTION", "PRIORITY:9"]
        );
    }

    #[test]
    fn events_come_before_tasks_in_one_calendar() {
        let (e, t) = (meeting(), Task { title: "t".into(), ..Default::default() });
        let out = write(&[&e, &e], &[&t], &WriteOptions { method: "PUBLISH".into(), now: 0 });
        assert_eq!(out.matches("BEGIN:VCALENDAR").count(), 1);
        assert_eq!((out.matches("BEGIN:VEVENT").count(), out.matches("BEGIN:VTODO").count()), (2, 1));
        assert!(out.find("BEGIN:VTODO").unwrap() > out.rfind("END:VEVENT").unwrap());
        assert!(out.contains("DTSTAMP:19700101T000000Z"));
    }
}

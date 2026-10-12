//! Whole-crate properties: what is written reads back the same, and no text makes the reader panic.

use mailcraft_model::{Address, Attendee, Event, Freq, Importance, Recurrence, Response, ShowAs, Task};
use mailcraft_time::{DAY, Date, DateTime, Weekday};
use proptest::prelude::*;

use crate::{IcsFile, WriteOptions, parse, write};

fn d(y: i32, m: u8, day: u8) -> Date {
    Date::new(y, m, day).unwrap()
}

fn at(y: i32, m: u8, day: u8, h: u8, min: u8) -> DateTime {
    DateTime::new(d(y, m, day), h, min, 0).unwrap()
}

fn options() -> WriteOptions {
    WriteOptions { method: "PUBLISH".into(), now: 1_760_000_000 }
}

/// What reading back gives for a written event: no ids, and a uid where there was none.
fn as_read(e: &Event) -> Event {
    let uid = if e.uid.is_empty() { format!("mailcraft-{}@mailcraft.local", e.id) } else { e.uid.clone() };
    Event { id: 0, calendar: 0, uid, ..e.clone() }
}

fn task_as_read(t: &Task) -> Task {
    let uid = if t.uid.is_empty() { format!("mailcraft-task-{}@mailcraft.local", t.id) } else { t.uid.clone() };
    Task { id: 0, message: None, uid, ..t.clone() }
}

fn round_trip(events: &[Event], tasks: &[Task]) -> IcsFile {
    let text = write(&events.iter().collect::<Vec<_>>(), &tasks.iter().collect::<Vec<_>>(), &options());
    // Whatever the local zone is, what this crate writes doesn't depend on it.
    parse(&text, &|ts| if ts % 2 == 0 { -480 } else { 345 })
}

fn sample_events() -> Vec<Event> {
    let meeting = Event {
        id: 1,
        calendar: 2,
        uid: "planning@example.org".into(),
        title: "Quarterly planning; budget, hiring".into(),
        location: "Room 4.12, North Building".into(),
        description: "Agenda:\n\t1. Budget \\ phase two\n\t2. Hiring: \"open\" roles\n".into(),
        start: at(2026, 10, 12, 9, 0),
        end: at(2026, 10, 12, 10, 30),
        organizer: Some(Address::new("Okafor, Nadia", "nadia.okafor@example.org")),
        attendees: vec![
            Attendee { address: Address::new("Tomas Lindqvist", "tomas.lindqvist@example.org"), optional: false, response: Response::None },
            Attendee { address: Address::new("Mei \"MT\" Tanaka ^_^", "mei.tanaka@example.org"), optional: true, response: Response::Tentative },
            Attendee { address: Address::new("", "ibrahim@example.org"), optional: false, response: Response::Declined },
            Attendee { address: Address::new("Zoë: ops; night", "zoe@example.org"), optional: true, response: Response::Accepted },
        ],
        reminder_minutes: Some(15),
        show_as: ShowAs::Tentative,
        categories: vec!["Work".into(), "Planning, 2026".into()],
        ..Default::default()
    };
    let trip = Event {
        id: 2,
        title: "Conference in Riverton".into(),
        start: at(2026, 10, 12, 0, 0),
        end: at(2026, 10, 15, 0, 0),
        all_day: true,
        show_as: ShowAs::OutOfOffice,
        private: true,
        reminder_minutes: Some(0),
        ..Default::default()
    };
    let standup = Event {
        id: 3,
        uid: "standup@example.org".into(),
        title: "Stand-up".into(),
        start: at(2026, 10, 5, 14, 0),
        end: at(2026, 10, 5, 14, 30),
        recurrence: Some(Recurrence { by_day: vec![Weekday::Mon, Weekday::Thu], until: Some(d(2026, 12, 31)), ..Recurrence::every(Freq::Weekly) }),
        exceptions: vec![d(2026, 10, 19), d(2026, 11, 2)],
        show_as: ShowAs::WorkingElsewhere,
        ..Default::default()
    };
    let moved = Event {
        id: 4,
        uid: "standup@example.org#20261019T140000".into(),
        title: "Stand-up (moved)".into(),
        start: at(2026, 10, 20, 10, 0),
        end: at(2026, 10, 20, 10, 30),
        cancelled: true,
        ..Default::default()
    };
    let birthday = Event {
        id: 5,
        title: "Mei's birthday 🎂".into(),
        start: at(2026, 2, 28, 0, 0),
        end: at(2026, 3, 1, 0, 0),
        all_day: true,
        recurrence: Some(Recurrence::every(Freq::Yearly)),
        exceptions: vec![d(2028, 2, 28)],
        show_as: ShowAs::Free,
        ..Default::default()
    };
    let review = Event {
        id: 6,
        title: "Monthly review".into(),
        start: at(2026, 10, 30, 16, 0),
        end: at(2026, 10, 30, 17, 0),
        recurrence: Some(Recurrence { interval: 2, by_day: vec![Weekday::Fri], nth: Some(-1), count: Some(6), ..Recurrence::every(Freq::Monthly) }),
        ..Default::default()
    };
    vec![meeting, trip, standup, moved, birthday, review, Event::default()]
}

fn sample_tasks() -> Vec<Task> {
    vec![
        Task {
            id: 1,
            title: "Send the budget draft".into(),
            notes: "Numbers first; then the summary.\nAsk Mei, or Tomas.".into(),
            start: Some(d(2026, 10, 10)),
            due: Some(d(2026, 10, 16)),
            importance: Importance::High,
            categories: vec!["Work".into(), "Finance, Q4".into()],
            reminder: Some(at(2026, 10, 15, 16, 0).to_timestamp(0)),
            message: Some(99),
            uid: "task-1@example.org".into(),
            ..Default::default()
        },
        Task {
            id: 2,
            title: "Book the venue".into(),
            due: Some(d(2026, 10, 3)),
            completed: Some(at(2026, 10, 2, 10, 15).to_timestamp(0)),
            importance: Importance::Low,
            ..Default::default()
        },
        Task { id: 3, title: "Done at the dawn of time".into(), completed: Some(0), ..Default::default() },
        Task::default(),
    ]
}

#[test]
fn sample_calendar_round_trips() {
    let (events, tasks) = (sample_events(), sample_tasks());
    let back = round_trip(&events, &tasks);
    assert_eq!(back.method, "PUBLISH");
    assert_eq!(back.events, events.iter().map(as_read).collect::<Vec<_>>());
    assert_eq!(back.tasks, tasks.iter().map(task_as_read).collect::<Vec<_>>());
    // And what was read writes the same text again.
    let refs: (Vec<&Event>, Vec<&Task>) = (back.events.iter().collect(), back.tasks.iter().collect());
    let again = write(&refs.0, &refs.1, &options());
    assert_eq!(parse(&again, &|_| 0), back);
}

#[test]
fn every_free_busy_state_and_response_round_trips() {
    let base = Event { uid: "x".into(), ..Default::default() };
    for show_as in ShowAs::ALL {
        for response in [Response::None, Response::Accepted, Response::Tentative, Response::Declined] {
            for optional in [false, true] {
                let e =
                    Event { show_as, attendees: vec![Attendee { address: Address::new("A", "a@example.org"), optional, response }], ..base.clone() };
                assert_eq!(round_trip(std::slice::from_ref(&e), &[]).events, [e]);
            }
        }
    }
}

#[test]
fn a_changed_instance_written_out_reads_back_beside_its_series() {
    // Read a series with a changed instance, write both, read again: nothing moves.
    let text = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:s@example.org\r\nDTSTART:20261005T140000\r\nDTEND:20261005T143000\r\nRRULE:FREQ=WEEKLY\r\nEND:VEVENT\r\n\
BEGIN:VEVENT\r\nUID:s@example.org\r\nRECURRENCE-ID:20261012T140000\r\nDTSTART:20261013T090000\r\nDTEND:20261013T093000\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let first = parse(text, &|_| 0);
    assert_eq!(first.events[0].exceptions, [d(2026, 10, 12)]);
    assert_eq!(first.events[1].uid, "s@example.org#20261012T140000");
    let second = round_trip(&first.events, &[]);
    assert_eq!(second.events, first.events);
}

fn text() -> impl Strategy<Value = String> {
    prop_oneof![
        // Anything printable, with the characters the format gives a meaning to.
        "(\\PC|[\n\t;,\\\\:\"^=]){0,40}",
        "[ -~]{0,120}",
        Just(String::new()),
    ]
}

fn name() -> impl Strategy<Value = String> {
    "(\\PC|[;,\\\\:\"^'=]){0,20}"
}

fn date() -> impl Strategy<Value = Date> {
    (1900i32..2200, 1u8..=12, 1u8..=31).prop_map(|(y, m, day)| Date::new(y, m, day).unwrap_or(d(y, m, 28)))
}

fn address() -> impl Strategy<Value = Address> {
    (name(), "[a-z0-9._+-]{1,12}@[a-z0-9-]{1,8}\\.example\\.org").prop_map(|(name, email)| Address::new(name, email))
}

fn category() -> impl Strategy<Value = String> {
    // A category has a name: an empty one gets a character.
    (text(), "[!-~]").prop_map(|(name, fallback)| if name.trim().is_empty() { fallback } else { name.trim().to_string() })
}

fn show_as() -> impl Strategy<Value = ShowAs> {
    proptest::sample::select(ShowAs::ALL.to_vec())
}

/// A rule the model gives a meaning to, as an event that starts on `start` holds it.
fn recurrence() -> impl Strategy<Value = Recurrence> {
    let freq = proptest::sample::select(vec![Freq::Daily, Freq::Weekly, Freq::Monthly, Freq::Yearly]);
    let days = proptest::sample::subsequence(Weekday::ALL.to_vec(), 1..=7).prop_shuffle();
    let nth = proptest::option::of(prop_oneof![1i8..=5, Just(-1i8)]);
    (freq, 1u32..=400, days, any::<bool>(), nth, proptest::option::of(any::<u32>()), proptest::option::of(date())).prop_map(
        |(freq, interval, days, with_days, nth, count, until)| {
            let monthly = matches!(freq, Freq::Monthly | Freq::Yearly);
            let nth = if monthly { nth } else { None };
            // A position always has its weekdays; the model ignores the days of a monthly rule without one.
            let by_day = if (monthly && nth.is_some()) || (!monthly && with_days) { days } else { Vec::new() };
            Recurrence { freq, interval, by_day, nth, count, until }
        },
    )
}

fn event() -> impl Strategy<Value = Event> {
    let texts = (any::<u64>(), any::<u64>(), text().prop_map(|uid| uid.trim().to_string()), text(), text(), text());
    let times = (date(), 0i64..DAY, any::<bool>(), 0i64..20_000, 1i64..12);
    let people = (proptest::option::of(address()), proptest::collection::vec((address(), any::<bool>(), 0usize..4), 0..4));
    let series = (proptest::option::of(recurrence()), proptest::collection::vec(date(), 0..4), proptest::option::of(any::<u32>()));
    let flags = (show_as(), proptest::collection::vec(category(), 0..3), any::<bool>(), any::<bool>());
    (texts, times, people, series, flags).prop_map(|(texts, times, people, series, flags)| {
        let (id, calendar, uid, title, location, description) = texts;
        let (day, second, all_day, minutes, days) = times;
        let start = if all_day { DateTime::at_midnight(day) } else { DateTime::from_civil_seconds(day.to_days() * DAY + second) };
        let end = if all_day { start.add_days(days) } else { start.add_minutes(minutes) };
        let (organizer, attendees) = people;
        let responses = [Response::None, Response::Accepted, Response::Tentative, Response::Declined];
        let attendees =
            attendees.into_iter().map(|(address, optional, response)| Attendee { address, optional, response: responses[response] }).collect();
        let (recurrence, exceptions, reminder_minutes) = series;
        let (show_as, categories, private, cancelled) = flags;
        Event {
            id,
            calendar,
            uid,
            title,
            location,
            description,
            start,
            end,
            all_day,
            organizer,
            attendees,
            recurrence,
            exceptions,
            reminder_minutes,
            show_as,
            categories,
            private,
            cancelled,
            ..Default::default()
        }
    })
}

fn task() -> impl Strategy<Value = Task> {
    let texts = (any::<u64>(), text().prop_map(|uid| uid.trim().to_string()), text(), text());
    let dates = (
        proptest::option::of(date()),
        proptest::option::of(date()),
        proptest::option::of(-2_000_000_000i64..6_000_000_000),
        proptest::option::of(-2_000_000_000i64..6_000_000_000),
    );
    let rest = (
        proptest::sample::select(vec![Importance::Low, Importance::Normal, Importance::High]),
        proptest::collection::vec(category(), 0..3),
        proptest::option::of(any::<u64>()),
    );
    (texts, dates, rest).prop_map(|((id, uid, title, notes), (start, due, completed, reminder), (importance, categories, message))| Task {
        id,
        title,
        notes,
        start,
        due,
        completed,
        importance,
        categories,
        reminder,
        message,
        uid,
    })
}

/// Text that looks like a calendar without being one.
fn soup() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        proptest::sample::select(vec![
            "BEGIN:VCALENDAR",
            "END:VCALENDAR",
            "BEGIN:VEVENT",
            "END:VEVENT",
            "BEGIN:VTODO",
            "END:VTODO",
            "BEGIN:VALARM",
            "END:VALARM",
            "BEGIN:VTIMEZONE",
            "END:VTIMEZONE",
            "BEGIN:",
            "END:",
            "DTSTART",
            "DTEND",
            "DUE",
            "COMPLETED",
            "DURATION",
            "RRULE",
            "EXDATE",
            "RECURRENCE-ID",
            "TRIGGER",
            "SUMMARY",
            "UID",
            "ATTENDEE",
            "ORGANIZER",
            "CATEGORIES",
            "METHOD",
            "STATUS",
            "PRIORITY",
            ":",
            ";",
            ",",
            "=",
            "\"",
            "\\",
            "^",
            "\r\n",
            "\n",
            "\r\n ",
            "\n\t",
            " ",
            "Z",
            "T",
            "-",
            "P",
            "#",
            ";VALUE=DATE",
            ";TZID=Example/Riverton",
            ";CN=",
            ";RELATED=END",
            ";PARTSTAT=ACCEPTED",
            "mailto:",
            "a@example.org",
            "20261012",
            "20261012T090000",
            "20261012T090000Z",
            "99999999T999999",
            "00000000",
            "-PT15M",
            "P1D",
            "PT",
            "P99999999999999999999W",
            "FREQ=WEEKLY",
            "FREQ=MONTHLY",
            "INTERVAL=2",
            "BYDAY=MO,-1FR",
            "BYSETPOS=-1",
            "COUNT=4294967296",
            "UNTIL=20261231T235959Z",
            "COMPLETED",
            "CANCELLED",
            "TRANSPARENT",
            "100",
        ])
        .prop_map(str::to_string),
        "\\PC{0,6}",
        any::<String>(),
    ];
    proptest::collection::vec(piece, 0..60).prop_map(|pieces| pieces.concat())
}

/// A valid calendar with everything in it, to damage.
fn valid_text() -> String {
    let (events, tasks) = (sample_events(), sample_tasks());
    write(&events.iter().collect::<Vec<_>>(), &tasks.iter().collect::<Vec<_>>(), &options())
}

/// Whatever the reader gives must also be writable and readable again.
fn survives(text: &str) -> IcsFile {
    let file = parse(text, &|ts| (ts % 1500) as i32);
    let _ = parse(text, &|_| i32::MAX);
    let _ = parse(text, &|_| i32::MIN);
    let again =
        write(&file.events.iter().collect::<Vec<_>>(), &file.tasks.iter().collect::<Vec<_>>(), &WriteOptions { method: file.method.clone(), now: 0 });
    let _ = parse(&again, &|_| 0);
    file
}

proptest! {
    #[test]
    fn any_event_round_trips(events in proptest::collection::vec(event(), 0..4)) {
        let back = round_trip(&events, &[]);
        prop_assert_eq!(back.events, events.iter().map(as_read).collect::<Vec<_>>());
        prop_assert!(back.tasks.is_empty());
    }

    #[test]
    fn any_task_round_trips(tasks in proptest::collection::vec(task(), 0..4)) {
        let back = round_trip(&[], &tasks);
        prop_assert_eq!(back.tasks, tasks.iter().map(task_as_read).collect::<Vec<_>>());
        prop_assert!(back.events.is_empty());
    }

    #[test]
    fn any_title_and_description_round_trip(title in "(\\PC|[\n\t]){0,300}", description in "(\\PC|[\n\t]){0,300}", location in "\\PC{0,200}") {
        let e = Event { uid: "u".into(), title, description, location, ..Default::default() };
        let t = Task { uid: "t".into(), title: e.description.clone(), notes: e.title.clone(), ..Default::default() };
        let back = round_trip(std::slice::from_ref(&e), std::slice::from_ref(&t));
        prop_assert_eq!(back.events, vec![e]);
        prop_assert_eq!(back.tasks, vec![t]);
    }

    #[test]
    fn text_with_control_characters_reads_back_clean_and_stable(title in any::<String>(), description in any::<String>(), name in any::<String>(), email in any::<String>(), category in any::<String>()) {
        let e = Event { title, description, organizer: Some(Address { name, email }), categories: vec![category], ..Default::default() };
        let once = round_trip(std::slice::from_ref(&e), &[]);
        prop_assert_eq!(once.events.len(), 1);
        // The first pass cleans (control characters, stray spaces); after that nothing changes.
        let twice = round_trip(&once.events, &[]);
        prop_assert_eq!(&twice.events, &once.events);
        prop_assert!(!once.events[0].title.contains('\r'));
    }

    #[test]
    fn arbitrary_text_never_panics(text in any::<String>()) {
        survives(&text);
    }

    #[test]
    fn calendar_shaped_junk_never_panics(text in soup()) {
        survives(&text);
    }

    #[test]
    fn a_damaged_calendar_never_panics(cuts in proptest::collection::vec((any::<prop::sample::Index>(), 0usize..40, 0u8..5, "\\PC{0,8}|[\r\n:;,=\"\\\\ ]{1,4}"), 1..6)) {
        let mut chars: Vec<char> = valid_text().chars().collect();
        for (index, length, kind, junk) in cuts {
            let from = index.index(chars.len().max(1)).min(chars.len());
            let to = from.saturating_add(length).min(chars.len());
            match kind {
                // Cut a piece out, overwrite it, insert junk, double a piece, or stop the file there.
                0 => drop(chars.drain(from..to)),
                1 => drop(chars.splice(from..to, junk.chars())),
                2 => drop(chars.splice(from..from, junk.chars())),
                3 => {
                    let piece: Vec<char> = chars[from..to].to_vec();
                    drop(chars.splice(from..from, piece));
                }
                _ => chars.truncate(from),
            }
        }
        let damaged: String = chars.into_iter().collect();
        let file = survives(&damaged);
        prop_assert!(file.events.len() + file.tasks.len() <= 64);
    }
}

#[test]
fn a_calendar_cut_anywhere_never_panics() {
    let text = valid_text();
    for (cut, _) in text.char_indices().step_by(7) {
        survives(&text[..cut]);
        survives(&text[cut..]);
    }
}

//! The mailbox's own settings, of which the calendar wants one: the working hours the owner set
//! (the days of the week, from when to when, and the time zone that is meant in).
//!
//! `GET /me/mailboxSettings/workingHours` brings them and nothing else of the settings (the
//! automatic replies and their texts stay where they are). It takes a permission of its own,
//! `MailboxSettings.Read`, which a sign-in from before it was asked for doesn't have: the answer
//! is then [`NetError::Forbidden`], and nothing else is the worse for it. The settings are only
//! read, never written.
//!
//! What comes back is hostile JSON like everything else from a server: the answer is capped,
//! and hours that are not hours (no days, a time that isn't one, an end before the start) are
//! no hours at all.

use mailcraft_time::Weekday;
use serde_json::Value;

use super::{ASK_TIMEOUT, DAYS, Doing, GRAPH, Graph, code_of, nonsense, text};
use crate::NetError;

const WORKING_HOURS: &str = "/v1.0/me/mailboxSettings/workingHours";
/// The largest answer that is read: the hours are a few hundred bytes.
const MAX_SETTINGS: usize = 64 * 1024;
/// Most days read from the list of working days, longest time of day read, and the longest name
/// of a time zone that is kept.
const MAX_DAYS: usize = 32;
const MAX_TIME: usize = 32;
const MAX_ZONE: usize = 64;
const DAY_MINUTES: u16 = 24 * 60;

/// When the owner of a mailbox works.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkingHours {
    /// The days of the week, each once, Monday first.
    pub days: Vec<Weekday>,
    /// From and up to, in minutes of the day, by the clock of the zone below.
    pub start: u16,
    pub end: u16,
    /// What the mailbox calls the time zone the hours are in (`Pacific Standard Time`,
    /// `Europe/Berlin`, or `Customized Time Zone` for one it spells out); empty when it says
    /// none. No zone is looked up here, and the times are not turned into any other zone's.
    pub zone: String,
}

/// What of a mailbox's settings is read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MailboxSettings {
    /// The working hours; `None` when the mailbox has none that can be read.
    pub working_hours: Option<WorkingHours>,
}

/// The working hours of the mailbox, for the owner of `token` (an access token for Graph).
/// Without the permission for it the answer is [`NetError::Forbidden`].
pub fn mailbox_settings(token: &str) -> Result<MailboxSettings, NetError> {
    mailbox_settings_at(GRAPH, token)
}

/// [`mailbox_settings`] from Graph at `base`.
pub(super) fn mailbox_settings_at(base: &str, token: &str) -> Result<MailboxSettings, NetError> {
    let graph = Graph::at(base, token)?;
    let reply = graph.ask(&graph.url(WORKING_HOURS), "GET", &[], None, ASK_TIMEOUT, MAX_SETTINGS)?;
    match reply.status {
        200 => {}
        // The permission for it is one an older sign-in doesn't have: said as what it is.
        403 => {
            let code = code_of(&reply.body, &[token]);
            return Err(NetError::Forbidden(format!("this account may not read its mailbox settings through Microsoft Graph{code}")));
        }
        _ => return Err(graph.refusal(Doing::Calendar, &reply, "")),
    }
    let json = serde_json::from_slice::<Value>(&reply.body).ok().filter(Value::is_object);
    let Some(json) = json else { return Err(nonsense("Microsoft Graph's answer isn't the JSON it should be")) };
    let settings = MailboxSettings { working_hours: working_hours_of(&json) };
    log::debug!("mailbox settings: working hours {}", if settings.working_hours.is_some() { "read" } else { "not there" });
    Ok(settings)
}

/// `08:00:00.0000000` (or `08:00`): minutes of the day; midnight at the end of the day is 24:00.
fn time_of(text: &str) -> Option<u16> {
    if text.len() > MAX_TIME {
        return None;
    }
    let mut parts = text.trim().split(':');
    let number =
        |part: Option<&str>| part.filter(|part| !part.is_empty() && part.len() <= 2 && part.bytes().all(|b| b.is_ascii_digit()))?.parse::<u16>().ok();
    let (hour, minute) = (number(parts.next())?, number(parts.next())?);
    // Seconds and their fractions, when they are there, are that and nothing else; and the
    // midnight that ends a day is midnight to the second.
    let rest = parts.next();
    let seconds = rest.is_none_or(|rest| rest.bytes().all(|b| b.is_ascii_digit() || b == b'.'));
    let midnight = hour == 24 && minute == 0 && rest.is_none_or(|rest| rest.bytes().all(|b| b == b'0' || b == b'.'));
    let fits = seconds && parts.next().is_none() && minute < 60 && (hour < 24 || midnight);
    fits.then_some(hour * 60 + minute)
}

/// The working hours in an answer: the object itself (`{"daysOfWeek": […], "startTime": "…",
/// "endTime": "…", "timeZone": {"name": "…"}}`), or the one under `workingHours` when the answer
/// is all of the mailbox's settings. `None` unless there is at least one day, and an end after
/// the start within the day.
pub(super) fn working_hours_of(answer: &Value) -> Option<WorkingHours> {
    let hours = answer.get("workingHours").filter(|inner| inner.is_object()).unwrap_or(answer);
    let listed = hours.get("daysOfWeek").and_then(Value::as_array)?;
    let mut days: Vec<Weekday> = Vec::new();
    for name in listed.iter().take(MAX_DAYS).filter_map(Value::as_str) {
        let known = DAYS.iter().find(|(known, _)| known.eq_ignore_ascii_case(name.trim())).map(|(_, day)| *day);
        if let Some(day) = known.filter(|day| !days.contains(day)) {
            days.push(day);
        }
    }
    days.sort();
    let (start, end) = (time_of(text(hours, "startTime"))?, time_of(text(hours, "endTime"))?);
    if days.is_empty() || start >= end || end > DAY_MINUTES {
        return None;
    }
    // The zone's name on one line; a name too long to be one is none.
    let named = hours.get("timeZone").map_or("", |zone| text(zone, "name")).trim();
    let zone: String = if named.len() <= 4 * MAX_ZONE { named.chars().filter(|c| !c.is_control()).take(MAX_ZONE).collect() } else { String::new() };
    Some(WorkingHours { days, start, end, zone })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use serde_json::json;

    use super::*;
    use crate::fake::{FakeHttp, json_answer};

    const TOKEN: &str = "graph.acc-ess_tok.en-0123456789";

    fn hours(days: &[&str], start: &str, end: &str, zone: Value) -> Value {
        json!({
            "@odata.context": "https://graph.example.org/v1.0/$metadata#users('ada')/mailboxSettings/workingHours",
            "daysOfWeek": days, "startTime": start, "endTime": end, "timeZone": zone,
        })
    }

    fn read(answer: &Value) -> Result<MailboxSettings, NetError> {
        let server = FakeHttp::answering(json_answer("200 OK", &answer.to_string()));
        mailbox_settings_at(&server.url(""), TOKEN)
    }

    #[test]
    fn the_working_hours_come_as_the_mailbox_has_them() {
        let weekdays = ["monday", "tuesday", "wednesday", "thursday", "friday"];
        let answer = hours(&weekdays, "08:00:00.0000000", "17:00:00.0000000", json!({"name": "Pacific Standard Time"}));
        let server = FakeHttp::answering(json_answer("200 OK", &answer.to_string()));
        let settings = mailbox_settings_at(&server.url(""), TOKEN).unwrap();
        // The hours and nothing else of the settings are asked for, with the token, and only read.
        let request = server.request(0);
        assert!(request.starts_with("GET /v1.0/me/mailboxSettings/workingHours HTTP/1.1\r\n"), "{request}");
        assert!(request.contains(&format!("Authorization: Bearer {TOKEN}\r\n")) && server.asked() == 1);
        let read_hours = settings.working_hours.unwrap();
        assert_eq!(read_hours.days, [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri]);
        assert_eq!((read_hours.start, read_hours.end, read_hours.zone.as_str()), (480, 1020, "Pacific Standard Time"));

        // Sunday to Thursday, said out of order and twice, half past the hour, in a zone the
        // mailbox spells out instead of naming; and all of the settings where only the hours
        // were asked for.
        let custom = json!({
            "@odata.type": "#microsoft.graph.customTimeZone", "bias": -200, "name": "Customized Time Zone",
            "standardOffset": {"time": "02:00:00.0000000", "dayOccurrence": 4, "dayOfWeek": "sunday", "month": 5, "year": 0},
            "daylightOffset": {"daylightBias": -100, "time": "02:00:00.0000000", "dayOccurrence": 2, "dayOfWeek": "sunday", "month": 10, "year": 0},
        });
        let inner = hours(&["Thursday", "sunday", " monday ", "TUESDAY", "wednesday", "sunday"], "09:30", "18:30:00", custom);
        let all = json!({"automaticRepliesSetting": {"internalReplyMessage": "Not read"}, "timeZone": "UTC", "workingHours": inner});
        let read_hours = read(&all).unwrap().working_hours.unwrap();
        assert_eq!(read_hours.days, [Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Sun]);
        assert_eq!((read_hours.start, read_hours.end, read_hours.zone.as_str()), (570, 1110, "Customized Time Zone"));
        // The whole of a day is a working day's length too.
        let read_hours = read(&hours(&["saturday"], "00:00:00", "24:00:00", json!({"name": "UTC"}))).unwrap().working_hours.unwrap();
        assert_eq!((read_hours.days.as_slice(), read_hours.start, read_hours.end, read_hours.zone.as_str()), (&[Weekday::Sat][..], 0, 1440, "UTC"));
    }

    #[test]
    fn hostile_working_hours_are_no_hours() {
        let zone = || json!({"name": "Pacific Standard Time"});
        for odd in [
            // No days, days that are none, days that aren't a list.
            hours(&[], "08:00:00", "17:00:00", zone()),
            hours(&["someday", "", "mon"], "08:00:00", "17:00:00", zone()),
            json!({"daysOfWeek": "monday", "startTime": "08:00:00", "endTime": "17:00:00"}),
            json!({"daysOfWeek": [1, null, {"day": "monday"}], "startTime": "08:00:00", "endTime": "17:00:00"}),
            // Times that are none, and an end that isn't after the start.
            hours(&["monday"], "25:00:00.0000000", "26:00:00.0000000", zone()),
            hours(&["monday"], "08:60:00", "17:00:00", zone()),
            hours(&["monday"], "08:00:00", "24:00:01", zone()),
            hours(&["monday"], "08:00:00", "24:30:00", zone()),
            hours(&["monday"], "17:00:00", "08:00:00", zone()),
            hours(&["monday"], "08:00:00", "08:00:00", zone()),
            hours(&["monday"], "-8:00", "17:00", zone()),
            hours(&["monday"], "8 o'clock", "17:00", zone()),
            hours(&["monday"], "08:00:00:00", "17:00", zone()),
            hours(&["monday"], "", "", zone()),
            json!({"daysOfWeek": ["monday"], "startTime": 480, "endTime": 1020}),
            json!({"daysOfWeek": ["monday"], "startTime": "08:00".repeat(10_000), "endTime": "17:00"}),
            // Nothing of the kind.
            json!({}),
            json!({"workingHours": null}),
            json!({"workingHours": "nine to five"}),
            json!({"value": []}),
        ] {
            assert_eq!(read(&odd).unwrap(), MailboxSettings::default(), "{}", odd.to_string().chars().take(200).collect::<String>());
        }
        // A zone nobody has heard of is kept by its name, on one line and no longer than a name
        // is; one that isn't text, or is far too long, is none. The hours are still hours.
        let unknown = read(&hours(&["monday"], "08:00", "17:00", json!({"name": " Atlantis/Lost City\r\nBcc: x "}))).unwrap().working_hours.unwrap();
        assert_eq!(unknown.zone, "Atlantis/Lost CityBcc: x");
        for zone in [json!({"name": 7}), json!("UTC"), json!(null), json!({"name": "Z".repeat(10_000)}), json!([{"name": "UTC"}])] {
            let read_hours = read(&hours(&["monday"], "08:00", "17:00", zone)).unwrap().working_hours.unwrap();
            assert_eq!((read_hours.zone.as_str(), read_hours.start, read_hours.end), ("", 480, 1020));
        }
        let long = read(&hours(&["monday"], "08:00", "17:00", json!({"name": "é".repeat(100)}))).unwrap().working_hours.unwrap();
        assert_eq!(long.zone.chars().count(), 64);
        // A thousand days are a week at most.
        let many: Vec<&str> = std::iter::repeat_n(["monday", "friday", "sunday"], 400).flatten().collect();
        let read_hours = read(&hours(&many, "08:00", "17:00", zone())).unwrap().working_hours.unwrap();
        assert_eq!(read_hours.days, [Weekday::Mon, Weekday::Fri, Weekday::Sun]);
    }

    #[test]
    fn what_graph_says_when_the_settings_are_not_to_be_had() {
        let error = |code: &str| json!({"error": {"code": code, "message": "Access is denied. Check credentials and try again."}}).to_string();
        // The permission for it is one an older sign-in doesn't have: for the host to note and go on.
        let server = FakeHttp::answering(json_answer("403 Forbidden", &error("ErrorAccessDenied")));
        let refused = mailbox_settings_at(&server.url(""), TOKEN).unwrap_err();
        assert_eq!(
            refused.to_string(),
            NetError::Forbidden("this account may not read its mailbox settings through Microsoft Graph (ErrorAccessDenied)".into()).to_string()
        );
        assert!(matches!(refused, NetError::Forbidden(_)));
        // A sign-in that is no longer taken, a Graph that is busy, and anything else it says.
        let server = FakeHttp::answering(json_answer("401 Unauthorized", &error("InvalidAuthenticationToken")));
        assert!(mailbox_settings_at(&server.url(""), TOKEN).unwrap_err().is_refused_sign_in());
        let server = FakeHttp::answering(json_answer("503 Service Unavailable", "{}"));
        assert!(matches!(mailbox_settings_at(&server.url(""), TOKEN), Err(NetError::Busy { .. })));
        let server = FakeHttp::answering(json_answer("404 Not Found", &error("MailboxNotEnabledForRESTAPI")));
        let gone = mailbox_settings_at(&server.url(""), TOKEN).unwrap_err();
        assert!(gone.to_string().contains("status 404"), "{gone}");
        // Nothing of the token in any of it, even when a careless server writes it back.
        for status in ["403 Forbidden", "400 Bad Request", "500 Internal Server Error"] {
            let server = FakeHttp::answering(json_answer(status, &error(TOKEN)));
            let said = mailbox_settings_at(&server.url(""), TOKEN).unwrap_err();
            let shown = format!("{said} {said:?}");
            assert!((0..TOKEN.len() - 8).all(|from| !shown.contains(&TOKEN[from..from + 8])), "{shown}");
        }
        // An answer that isn't JSON, isn't an object, or is megabytes long is an error, not hours.
        for body in ["", "not json", "[]", "7", "\"workingHours\""] {
            let server = FakeHttp::answering(json_answer("200 OK", body));
            assert!(mailbox_settings_at(&server.url(""), TOKEN).is_err(), "{body}");
        }
        let huge = hours(&["monday"], "08:00", "17:00", json!({"name": "Z".repeat(4 * 1024 * 1024)}));
        assert!(read(&huge).is_err());
        // Without a token nothing is asked.
        let server = FakeHttp::answering(json_answer("200 OK", "{}"));
        assert!(mailbox_settings_at(&server.url(""), "").is_err());
        assert_eq!(server.asked(), 0);
    }

    #[test]
    fn times_of_day_are_times_of_day() {
        assert_eq!(time_of("08:00:00.0000000"), Some(480));
        assert_eq!(time_of("17:30"), Some(1050));
        assert_eq!(time_of(" 0:05:59 "), Some(5));
        assert_eq!(time_of("24:00:00"), Some(1440));
        for odd in [
            "",
            ":",
            "8",
            "24:01",
            "25:00",
            "08:60",
            "08:00:00:00",
            "08:00:xx",
            "-1:00",
            "+8:00",
            "0x8:00",
            "８:00",
            "999:00",
            "08:00:00.00000000000000000000000000",
        ] {
            assert_eq!(time_of(odd), None, "{odd}");
        }
    }

    fn any_json() -> impl Strategy<Value = Value> {
        let leaf = prop_oneof![
            Just(Value::Null),
            any::<bool>().prop_map(Value::from),
            any::<i64>().prop_map(Value::from),
            any::<f64>().prop_map(|number| serde_json::Number::from_f64(number).map_or(Value::Null, Value::Number)),
            "\\PC{0,24}".prop_map(Value::from),
            "(monday|tuesday|sunday|[0-2]?[0-9]:[0-6][0-9](:[0-9]{2}(\\.[0-9]{0,7})?)?|UTC)".prop_map(Value::from),
        ];
        leaf.prop_recursive(3, 24, 6, |inner| {
            prop_oneof![
                proptest::collection::vec(inner.clone(), 0..6).prop_map(Value::from),
                proptest::collection::vec(("(daysOfWeek|startTime|endTime|timeZone|name|workingHours|\\PC{0,6})", inner), 0..6)
                    .prop_map(|fields| Value::Object(fields.into_iter().collect())),
            ]
        })
    }

    proptest! {
        #[test]
        fn arbitrary_settings_give_hours_or_none(answer in any_json(), start in "\\PC{0,40}", end in "\\PC{0,40}") {
            let _ = (time_of(&start), time_of(&end));
            for answer in [answer.clone(), json!({"workingHours": answer.clone()}), json!({"daysOfWeek": ["monday"], "startTime": start, "endTime": end, "timeZone": answer})] {
                if let Some(hours) = working_hours_of(&answer) {
                    prop_assert!(!hours.days.is_empty() && hours.days.len() <= 7);
                    prop_assert!(hours.start < hours.end && hours.end <= 1440);
                    prop_assert!(hours.zone.chars().count() <= 64 && !hours.zone.chars().any(char::is_control));
                }
            }
        }
    }
}

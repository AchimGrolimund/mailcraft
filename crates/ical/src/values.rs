// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Property values: dates and times, durations and escaped text.

use mailcraft_time::{DAY, Date, DateTime, Timestamp};

/// Longest duration read, in seconds (some 10 000 years): sums of it can't overflow.
const MAX_DURATION: i64 = 10_000 * 366 * DAY;

/// A DATE or DATE-TIME value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum When {
    /// `20261012`: a whole day.
    Date(Date),
    /// `20261012T090000`: a wall-clock time, floating or in a named zone.
    Local(DateTime),
    /// `20261012T090000Z`.
    Utc(DateTime),
}

impl When {
    /// From `YYYYMMDD`, `YYYYMMDDTHHMMSS` or `YYYYMMDDTHHMMSSZ`; the seconds may be missing, and
    /// the ISO 8601 punctuation some writers add (`2026-10-12T09:00:00Z`) is let through.
    pub(crate) fn parse(s: &str) -> Option<When> {
        let s = s.trim();
        if s.len() > 32 || !s.is_ascii() {
            return None;
        }
        let (date, time) = match s.find(['T', 't']) {
            Some(i) => (s.get(..i)?, Some(s.get(i.saturating_add(1)..)?)),
            None => (s, None),
        };
        let date = Date::parse_iso(date)?;
        let Some(time) = time else { return Some(When::Date(date)) };
        let (clock, utc) = match time.strip_suffix(['Z', 'z']) {
            Some(clock) => (clock, true),
            None => (time, false),
        };
        let digits: String = clock.chars().filter(|c| *c != ':').collect();
        if !matches!(digits.len(), 4 | 6) || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        let part = |at: usize| digits.get(at..at.saturating_add(2)).and_then(|d| d.parse::<u8>().ok());
        // A leap second (60) is read as the last second of the minute.
        let second = if digits.len() == 6 { part(4)?.min(59) } else { 0 };
        let t = DateTime::new(date, part(0)?, part(2)?, second)?;
        Some(if utc { When::Utc(t) } else { When::Local(t) })
    }

    /// The date and time as written, whatever the zone.
    pub(crate) fn wall(self) -> DateTime {
        match self {
            When::Date(d) => DateTime::at_midnight(d),
            When::Local(t) | When::Utc(t) => t,
        }
    }

    /// The local wall-clock time: a UTC time is converted, anything else is taken as written.
    pub(crate) fn local(self, offset_at: &dyn Fn(Timestamp) -> i32) -> DateTime {
        match self {
            When::Utc(t) => {
                let ts = t.to_timestamp(0);
                DateTime::from_timestamp(ts, offset_at(ts))
            }
            other => other.wall(),
        }
    }

    /// The instant: anything but a UTC time is read in the local zone.
    pub(crate) fn timestamp(self, offset_at: &dyn Fn(Timestamp) -> i32) -> Timestamp {
        match self {
            When::Utc(t) => t.to_timestamp(0),
            other => {
                let t = other.wall();
                // Asked twice, so a time near a daylight-saving change still gets its own offset.
                let guess = t.to_timestamp(offset_at(t.to_timestamp(0)));
                t.to_timestamp(offset_at(guess))
            }
        }
    }
}

/// A duration in seconds from `PT15M`, `-PT1H30M`, `P1D`, `P1DT12H`, `P2W`; `None` for junk.
pub(crate) fn parse_duration(s: &str) -> Option<i64> {
    let s = s.trim();
    let (negative, body) = match s.strip_prefix('-') {
        Some(body) => (true, body),
        None => (false, s.strip_prefix('+').unwrap_or(s)),
    };
    let body = body.strip_prefix(['P', 'p'])?;
    let (mut total, mut number, mut in_time, mut any) = (0i64, None::<i64>, false, false);
    for c in body.chars() {
        match c.to_ascii_uppercase() {
            '0'..='9' => {
                let digit = i64::from(c.to_digit(10)?);
                number = Some(number.unwrap_or(0).checked_mul(10)?.checked_add(digit)?);
            }
            'T' if number.is_none() => in_time = true,
            unit => {
                let seconds = match (unit, in_time) {
                    ('W', false) => 7 * DAY,
                    ('D', false) => DAY,
                    ('H', true) => 3600,
                    ('M', true) => 60,
                    ('S', true) => 1,
                    _ => return None,
                };
                total = total.checked_add(number.take()?.checked_mul(seconds)?)?;
                any = true;
            }
        }
    }
    if number.is_some() || !any || total > MAX_DURATION {
        return None;
    }
    Some(if negative { -total } else { total })
}

/// A date that exists: hostile fields (month 13, year 0) are brought back into the calendar.
fn valid_date(d: Date) -> Date {
    Date::new(d.year, d.month, d.day).unwrap_or_else(|| Date::from_days(d.to_days()))
}

/// A date and time that exist.
pub(crate) fn valid_time(t: DateTime) -> DateTime {
    DateTime::new(valid_date(t.date), t.hour, t.minute, t.second).unwrap_or_else(|| DateTime::from_civil_seconds(t.civil_seconds()))
}

/// `20261012`.
pub(crate) fn format_date(d: Date) -> String {
    let d = valid_date(d);
    format!("{:04}{:02}{:02}", d.year, d.month, d.day)
}

/// `20261012T090000` (floating).
pub(crate) fn format_local(t: DateTime) -> String {
    let t = valid_time(t);
    format!("{}T{:02}{:02}{:02}", format_date(t.date), t.hour, t.minute, t.second)
}

/// `20261012T130000Z`.
pub(crate) fn format_utc(ts: Timestamp) -> String {
    format!("{}Z", format_local(DateTime::from_timestamp(ts, 0)))
}

/// A TEXT value ready to write: `\`, `;`, `,` and line breaks escaped (`\r\n` and `\r` count as
/// one line break), other control characters except tab dropped.
pub(crate) fn escape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(8));
    let mut after_cr = false;
    for c in s.chars() {
        let cr = c == '\r';
        match c {
            '\n' if after_cr => {}
            '\r' | '\n' => out.push_str("\\n"),
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
        after_cr = cr;
    }
    out
}

/// Undo [`escape_text`]: `\n` and `\N` are line breaks, `\\`, `\;`, `\,` (and the `\:` and `\"`
/// some writers add) their second character; any other backslash is kept. Control characters
/// except tab are dropped.
pub(crate) fn unescape_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(x @ ('\\' | ';' | ',' | ':' | '"')) => out.push(x),
                Some(x) => {
                    out.push('\\');
                    if !x.is_control() || x == '\t' {
                        out.push(x);
                    }
                }
                None => out.push('\\'),
            },
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// The first `max` items of a list value, split on its unescaped commas; the items are still
/// escaped.
pub(crate) fn split_list(s: &str, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut escaped) = (0usize, false);
    for (i, c) in s.char_indices() {
        if out.len() >= max {
            return out;
        }
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            ',' => {
                out.push(s.get(start..i).unwrap_or(""));
                start = i.saturating_add(1);
            }
            _ => {}
        }
    }
    if out.len() < max {
        out.push(s.get(start..).unwrap_or(""));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    fn at(y: i32, m: u8, day: u8, h: u8, min: u8, s: u8) -> DateTime {
        DateTime::new(d(y, m, day), h, min, s).unwrap()
    }

    #[test]
    fn dates_and_times() {
        assert_eq!(When::parse("20261012"), Some(When::Date(d(2026, 10, 12))));
        assert_eq!(When::parse("20261012T090000"), Some(When::Local(at(2026, 10, 12, 9, 0, 0))));
        assert_eq!(When::parse(" 20261012T133015Z "), Some(When::Utc(at(2026, 10, 12, 13, 30, 15))));
        assert_eq!(When::parse("20261012t0900z"), Some(When::Utc(at(2026, 10, 12, 9, 0, 0))));
        assert_eq!(When::parse("2026-10-12T09:00:00Z"), Some(When::Utc(at(2026, 10, 12, 9, 0, 0))));
        // A leap second.
        assert_eq!(When::parse("20161231T235960Z"), Some(When::Utc(at(2016, 12, 31, 23, 59, 59))));
    }

    #[test]
    fn junk_dates_are_none() {
        for bad in [
            "",
            "tomorrow",
            "2026101",
            "202610123",
            "20261312",
            "20260230",
            "00000101",
            "20261012T",
            "20261012T25",
            "20261012T240000",
            "20261012T0960",
            "20261012T090000+0200",
            "２０２６１０１２",
            "20261012T09000000000000000000000000000000000000",
        ] {
            assert_eq!(When::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn utc_times_become_local_and_local_times_become_instants() {
        let utc = When::parse("20261012T130000Z").unwrap();
        assert_eq!(utc.local(&|_| -240), at(2026, 10, 12, 9, 0, 0));
        assert_eq!(utc.local(&|_| 330), at(2026, 10, 12, 18, 30, 0));
        assert_eq!(utc.wall(), at(2026, 10, 12, 13, 0, 0));
        // Across midnight.
        assert_eq!(When::parse("20261012T020000Z").unwrap().local(&|_| -300).date, d(2026, 10, 11));
        let local = When::parse("20261012T090000").unwrap();
        assert_eq!(local.local(&|_| -240), at(2026, 10, 12, 9, 0, 0));
        assert_eq!(local.timestamp(&|_| -240), utc.timestamp(&|_| 600));
        assert_eq!(When::Date(d(1970, 1, 2)).timestamp(&|_| 60), DAY - 3600);
        // A zone that changes its offset at an instant: each side gets its own.
        let change = at(2026, 11, 1, 6, 0, 0).to_timestamp(0);
        let zone = |ts: Timestamp| if ts < change { -240 } else { -300 };
        assert_eq!(When::parse("20261101T000000").unwrap().timestamp(&zone), at(2026, 11, 1, 4, 0, 0).to_timestamp(0));
        assert_eq!(When::parse("20261101T120000").unwrap().timestamp(&zone), at(2026, 11, 1, 17, 0, 0).to_timestamp(0));
        // A hostile offset is survived.
        let _ = utc.local(&|_| i32::MIN);
        let _ = local.timestamp(&|_| i32::MAX);
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("PT15M"), Some(900));
        assert_eq!(parse_duration("-PT15M"), Some(-900));
        assert_eq!(parse_duration("+PT1H30M"), Some(5400));
        assert_eq!(parse_duration("P1D"), Some(DAY));
        assert_eq!(parse_duration("P1DT12H"), Some(DAY + 12 * 3600));
        assert_eq!(parse_duration("P2W"), Some(14 * DAY));
        assert_eq!(parse_duration("pt0s"), Some(0));
        assert_eq!(parse_duration("-PT0M"), Some(0));
        assert_eq!(parse_duration("PT1H2M3S"), Some(3723));
        for bad in ["", "P", "PT", "15M", "PT15", "P15M", "PT1D", "PTM", "P1X", "P-1D", "P99999999999999999999D", "P9999999999W", "PT1H T"] {
            assert_eq!(parse_duration(bad), None, "{bad}");
        }
    }

    #[test]
    fn formats_and_repairs_hostile_dates() {
        assert_eq!(format_date(d(2026, 1, 2)), "20260102");
        assert_eq!(format_date(d(987, 6, 5)), "09870605");
        assert_eq!(format_local(at(2026, 10, 12, 9, 5, 0)), "20261012T090500");
        assert_eq!(format_utc(0), "19700101T000000Z");
        assert_eq!(format_utc(1_760_082_785), "20251010T075305Z");
        assert_eq!(format_utc(i64::MAX), "99991231T235959Z");
        assert_eq!(format_utc(i64::MIN), "00010101T000000Z");
        for hostile in [
            Date { year: 0, month: 0, day: 0 },
            Date { year: i32::MAX, month: 255, day: 255 },
            Date { year: i32::MIN, month: 13, day: 40 },
            Date { year: 2026, month: 2, day: 30 },
        ] {
            assert!(When::parse(&format_date(hostile)).is_some(), "{hostile:?}");
            let t = DateTime { date: hostile, hour: 99, minute: 99, second: 99 };
            assert!(When::parse(&format_local(t)).is_some(), "{t:?}");
        }
        assert_eq!(format_date(Date { year: 2026, month: 2, day: 30 }), "20260302");
    }

    #[test]
    fn text_escapes() {
        assert_eq!(escape_text("Budget, phase two; \\ done\nNext"), "Budget\\, phase two\\; \\\\ done\\nNext");
        assert_eq!(escape_text("a\r\nb\rc\n\nd\u{0}\u{7f}\te"), "a\\nb\\nc\\n\\nd\te");
        assert_eq!(unescape_text("Budget\\, phase two\\; \\\\ done\\nNext\\NLast"), "Budget, phase two; \\ done\nNext\nLast");
        // What other writers escape without being asked to, and what isn't an escape at all.
        assert_eq!(unescape_text("10\\:30 \\\"quoted\\\" C:\\Users\\x tail\\"), "10:30 \"quoted\" C:\\Users\\x tail\\");
        assert_eq!(unescape_text("a\u{0}b\u{1b}c\td"), "abc\td");
    }

    #[test]
    fn lists_split_on_unescaped_commas() {
        assert_eq!(split_list("Work,Travel\\, abroad,,Home\\\\,x", 10), ["Work", "Travel\\, abroad", "", "Home\\\\", "x"]);
        assert_eq!(split_list("", 10), [""]);
        assert_eq!(split_list("a,b,c,d", 2), ["a", "b"]);
        assert_eq!(split_list("a,b", 2), ["a", "b"]);
        assert!(split_list("a,b", 0).is_empty());
    }

    proptest::proptest! {
        #[test]
        fn clean_text_survives_escaping(s in "(\\PC|[\n\t;,\\\\:\"]){0,80}") {
            proptest::prop_assert_eq!(unescape_text(&escape_text(&s)), s);
        }

        #[test]
        fn any_text_escapes_to_one_clean_line(s in proptest::prelude::any::<String>()) {
            let escaped = escape_text(&s);
            proptest::prop_assert!(!escaped.contains(['\r', '\n']));
            // Escaping what was read gives the same line again.
            proptest::prop_assert_eq!(escape_text(&unescape_text(&escaped)), escaped);
        }

        #[test]
        fn values_never_panic(s in proptest::prelude::any::<String>(), max in 0usize..5) {
            let _ = When::parse(&s);
            let _ = parse_duration(&s);
            let _ = unescape_text(&s);
            let _ = split_list(&s, max);
        }
    }
}

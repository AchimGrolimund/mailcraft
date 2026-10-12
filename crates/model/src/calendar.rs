//! Calendars, events and recurrence.
//!
//! Event times are wall-clock times in the user's zone ("floating"): a 9:00 meeting stays at 9:00
//! across a daylight-saving change, which is what a recurring series needs. Readers convert UTC
//! and zoned times to local wall-clock times on the way in.

use mailcraft_time::{Date, DateTime, Weekday, month_name};
use serde::{Deserialize, Serialize};

use crate::{Address, Id, Origin, Rgb};

/// Most periods a recurrence walks, and most occurrences one query returns: a hostile rule (or a
/// daily series since 1970) costs bounded work.
const MAX_STEPS: i64 = 20_000;
const MAX_RESULTS: usize = 2_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calendar {
    pub id: Id,
    pub name: String,
    pub color: Rgb,
    pub visible: bool,
    /// Set for a calendar that an account's server keeps (Microsoft 365, say): its events are
    /// copies of the server's, brought by a download.
    #[serde(skip_serializing_if = "Origin::is_local")]
    pub origin: Origin,
    /// For a calendar that a server keeps: the account may read it and not change it (a calendar
    /// of holidays or birthdays, somebody else's that is shared for reading).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// The group the server lists it under (`My Calendars`, `Other Calendars`); empty for none.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub group: String,
    /// The colour the owner chose for it here (`calendar.color`), which it is then shown in
    /// whatever `color` is: a server's calendar keeps the server's colour there, and shows in it
    /// again when the owner's choice is taken back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub own_color: Option<Rgb>,
}

impl Calendar {
    /// The colour it is shown in: the one chosen here when there is one, else its own.
    pub fn shown_color(&self) -> Rgb {
        self.own_color.unwrap_or(self.color)
    }
}

/// How an event shows in free/busy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ShowAs {
    Free,
    Tentative,
    #[default]
    Busy,
    OutOfOffice,
    WorkingElsewhere,
}

impl ShowAs {
    pub const ALL: [ShowAs; 5] = [ShowAs::Free, ShowAs::WorkingElsewhere, ShowAs::Tentative, ShowAs::Busy, ShowAs::OutOfOffice];

    pub fn label(self) -> &'static str {
        match self {
            ShowAs::Free => "Free",
            ShowAs::Tentative => "Tentative",
            ShowAs::Busy => "Busy",
            ShowAs::OutOfOffice => "Out of Office",
            ShowAs::WorkingElsewhere => "Working Elsewhere",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ShowAs::Free => "free",
            ShowAs::Tentative => "tentative",
            ShowAs::Busy => "busy",
            ShowAs::OutOfOffice => "outOfOffice",
            ShowAs::WorkingElsewhere => "workingElsewhere",
        }
    }

    pub fn parse(s: &str) -> Option<ShowAs> {
        let s = s.trim().to_ascii_lowercase().replace([' ', '-', '_'], "");
        Self::ALL.into_iter().find(|x| x.code().to_ascii_lowercase() == s || (s == "oof" && *x == ShowAs::OutOfOffice))
    }
}

/// An attendee's answer to an invitation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Response {
    #[default]
    None,
    Accepted,
    Tentative,
    Declined,
}

impl Response {
    pub fn label(self) -> &'static str {
        match self {
            Response::None => "No response",
            Response::Accepted => "Accepted",
            Response::Tentative => "Tentative",
            Response::Declined => "Declined",
        }
    }

    pub fn parse(s: &str) -> Option<Response> {
        match s.trim().to_ascii_lowercase().as_str() {
            "none" | "needs-action" | "" => Some(Response::None),
            "accepted" | "accept" => Some(Response::Accepted),
            "tentative" => Some(Response::Tentative),
            "declined" | "decline" => Some(Response::Declined),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Attendee {
    pub address: Address,
    pub optional: bool,
    pub response: Response,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Freq {
    Daily,
    #[default]
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    pub fn parse(s: &str) -> Option<Freq> {
        match s.trim().to_ascii_lowercase().as_str() {
            "daily" => Some(Freq::Daily),
            "weekly" => Some(Freq::Weekly),
            "monthly" => Some(Freq::Monthly),
            "yearly" => Some(Freq::Yearly),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Freq::Daily => "daily",
            Freq::Weekly => "weekly",
            Freq::Monthly => "monthly",
            Freq::Yearly => "yearly",
        }
    }

    fn unit(self) -> &'static str {
        match self {
            Freq::Daily => "day",
            Freq::Weekly => "week",
            Freq::Monthly => "month",
            Freq::Yearly => "year",
        }
    }
}

/// How an event repeats (the common subset of iCalendar's RRULE).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Recurrence {
    pub freq: Freq,
    /// Every `interval` days/weeks/months/years; 0 reads as 1.
    pub interval: u32,
    /// Weekly: the days of the week (none: the start's own). Daily: only on these days. Monthly and
    /// yearly with `nth`: the weekday(s) counted.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub by_day: Vec<Weekday>,
    /// Monthly and yearly: the `nth` `by_day` weekday of the month (1–5, or -1 for the last)
    /// instead of the start's day of the month.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nth: Option<i8>,
    /// Ends after this many occurrences.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Ends on this day (inclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub until: Option<Date>,
}

/// The `nth` (1–5, or negative for the last) `weekday` of a month; `None` when the month has no
/// such day (a fifth Friday).
fn nth_weekday(first_of_month: Date, weekday: Weekday, nth: i8) -> Option<Date> {
    if nth > 0 {
        let offset = (i64::from(weekday.index()) - i64::from(first_of_month.weekday().index())).rem_euclid(7);
        let day = 1 + offset + 7 * (i64::from(nth) - 1);
        Date::new(first_of_month.year, first_of_month.month, u8::try_from(day).ok()?)
    } else {
        let last = first_of_month.end_of_month();
        let back = (i64::from(last.weekday().index()) - i64::from(weekday.index())).rem_euclid(7);
        Some(last.add_days(-back))
    }
}

impl Recurrence {
    pub fn every(freq: Freq) -> Recurrence {
        Recurrence { freq, interval: 1, ..Default::default() }
    }

    fn step(&self) -> i64 {
        i64::from(self.interval.clamp(1, 1000))
    }

    /// The candidate dates of period `p` (0 is the period `start` is in), in order.
    fn period(&self, start: Date, p: i64) -> Vec<Date> {
        let n = p.saturating_mul(self.step());
        match self.freq {
            Freq::Daily => {
                let d = start.add_days(n);
                if self.by_day.is_empty() || self.by_day.contains(&d.weekday()) { vec![d] } else { Vec::new() }
            }
            Freq::Weekly => {
                let week = start.start_of_week(Weekday::Mon).add_days(n.saturating_mul(7));
                let mut days: Vec<Weekday> = if self.by_day.is_empty() { vec![start.weekday()] } else { self.by_day.clone() };
                days.sort();
                days.dedup();
                days.into_iter().map(|d| week.add_days(i64::from(d.index()))).collect()
            }
            Freq::Monthly | Freq::Yearly => {
                let months = if self.freq == Freq::Yearly { n.saturating_mul(12) } else { n };
                let first = start.start_of_month().add_months(i32::try_from(months).unwrap_or(i32::MAX));
                match self.nth {
                    Some(nth) if nth != 0 => {
                        let mut days: Vec<Weekday> = if self.by_day.is_empty() { vec![start.weekday()] } else { self.by_day.clone() };
                        days.sort();
                        days.dedup();
                        let mut out: Vec<Date> = days.into_iter().filter_map(|d| nth_weekday(first, d, nth)).collect();
                        out.sort();
                        out
                    }
                    // A day the month lacks (the 31st, February 29th) falls on its last day.
                    _ => vec![Date { day: start.day.min(first.end_of_month().day), ..first }],
                }
            }
        }
    }

    /// The days the series falls on between `from` and `to` (inclusive), for a series that starts
    /// on `start`, without the `exdates`. Excluded days still count towards `count`.
    pub fn dates(&self, start: Date, from: Date, to: Date, exdates: &[Date]) -> Vec<Date> {
        let mut out = Vec::new();
        let last = self.until.map_or(to, |u| u.min(to));
        if last < start || to < from {
            return out;
        }
        // Without a count, the periods before the window don't need walking.
        let first_period = if self.count.is_some() {
            0
        } else {
            let behind = match self.freq {
                Freq::Daily => start.days_until(from) / self.step(),
                Freq::Weekly => start.days_until(from) / (7 * self.step()),
                Freq::Monthly => (i64::from(from.year - start.year) * 12 + i64::from(from.month) - i64::from(start.month)) / self.step(),
                Freq::Yearly => i64::from(from.year - start.year) / self.step(),
            };
            (behind - 1).max(0)
        };
        let mut produced = 0u32;
        let mut previous: Option<Date> = None;
        for p in first_period..first_period.saturating_add(MAX_STEPS) {
            let period = self.period(start, p);
            // The calendar ends at year 9999: stop rather than repeat the clamped last day.
            if let (Some(first), Some(prev)) = (period.first(), previous)
                && *first <= prev
            {
                return out;
            }
            previous = period.last().copied().or(previous);
            for day in period {
                if day < start {
                    continue;
                }
                if day > last {
                    return out;
                }
                produced = produced.saturating_add(1);
                if self.count.is_some_and(|c| produced > c) {
                    return out;
                }
                if day >= from && !exdates.contains(&day) {
                    out.push(day);
                    if out.len() >= MAX_RESULTS {
                        return out;
                    }
                }
            }
        }
        out
    }

    /// The rule in words: `Every 2 weeks on Monday, Wednesday, until Dec 31, 2026`.
    pub fn describe(&self, start: Date) -> String {
        let mut s = match self.step() {
            1 => format!("Every {}", self.freq.unit()),
            n => format!("Every {n} {}s", self.freq.unit()),
        };
        let days = |list: &[Weekday]| {
            let mut d = list.to_vec();
            d.sort();
            d.dedup();
            d.iter().map(|d| d.name()).collect::<Vec<_>>().join(", ")
        };
        let own_day = [start.weekday()];
        let by_day = if self.by_day.is_empty() { &own_day[..] } else { &self.by_day[..] };
        match (self.freq, self.nth) {
            (Freq::Daily, _) if !self.by_day.is_empty() => s.push_str(&format!(" on {}", days(by_day))),
            (Freq::Daily, _) => {}
            (Freq::Weekly, _) => s.push_str(&format!(" on {}", days(by_day))),
            (Freq::Monthly, Some(n)) if n != 0 => s.push_str(&format!(" on the {} {}", ordinal(n), days(by_day))),
            (Freq::Monthly, _) => s.push_str(&format!(" on day {}", start.day)),
            (Freq::Yearly, Some(n)) if n != 0 => s.push_str(&format!(" on the {} {} of {}", ordinal(n), days(by_day), month_name(start.month))),
            (Freq::Yearly, _) => s.push_str(&format!(" on {} {}", month_name(start.month), start.day)),
        }
        if let Some(c) = self.count {
            s.push_str(&format!(", {c} time{}", if c == 1 { "" } else { "s" }));
        } else if let Some(u) = self.until {
            s.push_str(&format!(", until {}", u.medium()));
        }
        s
    }
}

fn ordinal(n: i8) -> &'static str {
    match n {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        5 => "fifth",
        _ => "last",
    }
}

/// One time an event happens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Occurrence {
    pub event: Id,
    pub start: DateTime,
    pub end: DateTime,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Event {
    pub id: Id,
    pub calendar: Id,
    /// iCalendar `UID`, kept so an exported event updates the same event elsewhere.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub uid: String,
    pub title: String,
    pub location: String,
    pub description: String,
    pub start: DateTime,
    /// Exclusive: an all-day event on one day ends at the next midnight.
    pub end: DateTime,
    pub all_day: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organizer: Option<Address>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<Attendee>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Recurrence>,
    /// Days of a recurring series that don't happen.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<Date>,
    /// Remind this many minutes before the start.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_minutes: Option<u32>,
    pub show_as: ShowAs,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub private: bool,
    pub cancelled: bool,
    /// Set for an event that an account's server keeps: it is a copy of the server's, and what
    /// the server calls it is how the next download finds it again.
    #[serde(skip_serializing_if = "Origin::is_local")]
    pub origin: Origin,
}

impl Event {
    /// Minutes from start to end; never negative.
    pub fn duration_minutes(&self) -> i64 {
        self.start.minutes_until(self.end).max(0)
    }

    /// A meeting has other people in it; an appointment doesn't.
    pub fn is_meeting(&self) -> bool {
        !self.attendees.is_empty()
    }

    /// Every time the event happens on the days `from`–`to` (inclusive), in order.
    pub fn occurrences(&self, from: Date, to: Date) -> Vec<Occurrence> {
        let minutes = self.duration_minutes();
        // An occurrence that started before `from` may still be running then.
        let reach = minutes / (24 * 60) + 1;
        let window_start = DateTime::at_midnight(from);
        let window_end = DateTime::at_midnight(to).add_days(1);
        let days = match &self.recurrence {
            Some(r) => r.dates(self.start.date, from.add_days(-reach), to, &self.exceptions),
            None => vec![self.start.date],
        };
        days.into_iter()
            .map(|day| {
                let start = DateTime { date: day, ..self.start };
                Occurrence { event: self.id, start, end: start.add_minutes(minutes) }
            })
            // Overlap; an event without a length still shows at its start.
            .filter(|o| o.start < window_end && (o.end > window_start || o.start >= window_start))
            .collect()
    }

    /// `9:00 AM – 10:00 AM`, or `All day`.
    pub fn time_label(&self, twelve_hour: bool) -> String {
        if self.all_day { "All day".into() } else { format!("{} – {}", self.start.time_short(twelve_hour), self.end.time_short(twelve_hour)) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(y: i32, m: u8, day: u8) -> Date {
        Date::new(y, m, day).unwrap()
    }

    fn all(r: &Recurrence, start: Date, from: Date, to: Date) -> Vec<String> {
        r.dates(start, from, to, &[]).into_iter().map(Date::iso).collect()
    }

    #[test]
    fn daily_and_weekdays() {
        let start = d(2026, 10, 5); // Monday
        let r = Recurrence { interval: 2, ..Recurrence::every(Freq::Daily) };
        assert_eq!(all(&r, start, start, d(2026, 10, 10)), ["2026-10-05", "2026-10-07", "2026-10-09"]);
        let weekdays =
            Recurrence { by_day: vec![Weekday::Mon, Weekday::Tue, Weekday::Wed, Weekday::Thu, Weekday::Fri], ..Recurrence::every(Freq::Daily) };
        assert_eq!(all(&weekdays, start, d(2026, 10, 9), d(2026, 10, 13)), ["2026-10-09", "2026-10-12", "2026-10-13"]);
    }

    #[test]
    fn weekly_on_several_days_with_count_and_until() {
        let start = d(2026, 10, 7); // Wednesday
        let r = Recurrence { by_day: vec![Weekday::Fri, Weekday::Mon, Weekday::Wed], count: Some(5), ..Recurrence::every(Freq::Weekly) };
        // The Monday of the first week is before the start.
        assert_eq!(all(&r, start, d(2026, 1, 1), d(2027, 1, 1)), ["2026-10-07", "2026-10-09", "2026-10-12", "2026-10-14", "2026-10-16"]);
        // The count runs from the start even when the window opens later.
        assert_eq!(all(&r, start, d(2026, 10, 13), d(2027, 1, 1)), ["2026-10-14", "2026-10-16"]);
        let r = Recurrence { interval: 2, until: Some(d(2026, 11, 4)), ..Recurrence::every(Freq::Weekly) };
        assert_eq!(all(&r, start, d(2026, 1, 1), d(2027, 1, 1)), ["2026-10-07", "2026-10-21", "2026-11-04"]);
    }

    #[test]
    fn monthly_by_day_and_by_nth_weekday() {
        let r = Recurrence::every(Freq::Monthly);
        assert_eq!(all(&r, d(2026, 1, 31), d(2026, 1, 1), d(2026, 4, 30)), ["2026-01-31", "2026-02-28", "2026-03-31", "2026-04-30"]);
        let second_tue = Recurrence { by_day: vec![Weekday::Tue], nth: Some(2), ..Recurrence::every(Freq::Monthly) };
        assert_eq!(all(&second_tue, d(2026, 10, 13), d(2026, 10, 1), d(2026, 12, 31)), ["2026-10-13", "2026-11-10", "2026-12-08"]);
        let last_fri = Recurrence { by_day: vec![Weekday::Fri], nth: Some(-1), ..Recurrence::every(Freq::Monthly) };
        assert_eq!(all(&last_fri, d(2026, 10, 30), d(2026, 10, 1), d(2026, 12, 31)), ["2026-10-30", "2026-11-27", "2026-12-25"]);
        // Only some months have a fifth Friday.
        let fifth_fri = Recurrence { by_day: vec![Weekday::Fri], nth: Some(5), ..Recurrence::every(Freq::Monthly) };
        assert_eq!(all(&fifth_fri, d(2026, 10, 30), d(2026, 10, 1), d(2027, 1, 31)), ["2026-10-30", "2027-01-29"]);
    }

    #[test]
    fn yearly_and_leap_days() {
        let r = Recurrence::every(Freq::Yearly);
        assert_eq!(all(&r, d(2024, 2, 29), d(2024, 1, 1), d(2028, 12, 31)), ["2024-02-29", "2025-02-28", "2026-02-28", "2027-02-28", "2028-02-29"]);
        // US Thanksgiving: the fourth Thursday of November.
        let r = Recurrence { by_day: vec![Weekday::Thu], nth: Some(4), ..Recurrence::every(Freq::Yearly) };
        assert_eq!(all(&r, d(2025, 11, 27), d(2025, 1, 1), d(2027, 12, 31)), ["2025-11-27", "2026-11-26", "2027-11-25"]);
    }

    #[test]
    fn far_windows_and_hostile_rules_are_bounded() {
        let r = Recurrence::every(Freq::Daily);
        assert_eq!(all(&r, d(1970, 1, 1), d(2026, 10, 11), d(2026, 10, 12)), ["2026-10-11", "2026-10-12"]);
        assert_eq!(r.dates(d(1970, 1, 1), d(1970, 1, 1), d(9999, 12, 31), &[]).len(), MAX_RESULTS);
        let hostile = Recurrence {
            interval: u32::MAX,
            count: Some(u32::MAX),
            by_day: vec![Weekday::Mon; 100],
            nth: Some(i8::MIN),
            ..Recurrence::every(Freq::Yearly)
        };
        let _ = hostile.dates(d(1, 1, 1), d(1, 1, 1), d(9999, 12, 31), &[]);
        let _ = Recurrence { interval: 1000, ..Recurrence::every(Freq::Monthly) }.dates(d(9990, 1, 1), d(1, 1, 1), d(9999, 12, 31), &[]);
        // A window before the start, or the wrong way round.
        assert!(r.dates(d(2026, 1, 1), d(2025, 1, 1), d(2025, 12, 31), &[]).is_empty());
        assert!(r.dates(d(2026, 1, 1), d(2026, 2, 1), d(2026, 1, 1), &[]).is_empty());
    }

    #[test]
    fn exceptions_are_skipped_but_counted() {
        let r = Recurrence { count: Some(3), ..Recurrence::every(Freq::Daily) };
        let got = r.dates(d(2026, 10, 5), d(2026, 10, 1), d(2026, 10, 31), &[d(2026, 10, 6)]);
        assert_eq!(got, [d(2026, 10, 5), d(2026, 10, 7)]);
    }

    #[test]
    fn rules_in_words() {
        let start = d(2026, 10, 13);
        assert_eq!(Recurrence::every(Freq::Daily).describe(start), "Every day");
        assert_eq!(
            Recurrence { interval: 2, by_day: vec![Weekday::Wed, Weekday::Mon], ..Recurrence::every(Freq::Weekly) }.describe(start),
            "Every 2 weeks on Monday, Wednesday"
        );
        assert_eq!(Recurrence { count: Some(10), ..Recurrence::every(Freq::Weekly) }.describe(start), "Every week on Tuesday, 10 times");
        assert_eq!(Recurrence { nth: Some(2), ..Recurrence::every(Freq::Monthly) }.describe(start), "Every month on the second Tuesday");
        assert_eq!(
            Recurrence { until: Some(d(2030, 1, 1)), ..Recurrence::every(Freq::Yearly) }.describe(start),
            "Every year on October 13, until Jan 1, 2030"
        );
    }

    #[test]
    fn event_occurrences_overlap_the_window() {
        let at = |day: Date, h: u8| DateTime::new(day, h, 0, 0).unwrap();
        let once = Event { id: 7, start: at(d(2026, 10, 12), 9), end: at(d(2026, 10, 12), 10), ..Default::default() };
        assert_eq!(once.occurrences(d(2026, 10, 12), d(2026, 10, 12)).len(), 1);
        assert!(once.occurrences(d(2026, 10, 13), d(2026, 10, 20)).is_empty());
        assert_eq!(once.duration_minutes(), 60);
        // A three-day conference shows on each of its days.
        let conf = Event { all_day: true, start: at(d(2026, 10, 12), 0), end: at(d(2026, 10, 15), 0), ..Default::default() };
        assert_eq!(conf.occurrences(d(2026, 10, 14), d(2026, 10, 14)).len(), 1);
        assert!(conf.occurrences(d(2026, 10, 15), d(2026, 10, 15)).is_empty());
        // A weekly series, minus one cancelled day.
        let weekly = Event {
            start: at(d(2026, 10, 5), 14),
            end: at(d(2026, 10, 5), 15),
            recurrence: Some(Recurrence::every(Freq::Weekly)),
            exceptions: vec![d(2026, 10, 19)],
            ..Default::default()
        };
        let got: Vec<String> = weekly.occurrences(d(2026, 10, 1), d(2026, 10, 31)).into_iter().map(|o| o.start.iso()).collect();
        assert_eq!(got, ["2026-10-05T14:00:00", "2026-10-12T14:00:00", "2026-10-26T14:00:00"]);
        // An event that ends before it starts has no length rather than a negative one.
        let odd = Event { start: at(d(2026, 10, 12), 9), end: at(d(2026, 10, 11), 9), ..Default::default() };
        assert_eq!((odd.duration_minutes(), odd.occurrences(d(2026, 10, 12), d(2026, 10, 12)).len()), (0, 1));
        assert_eq!(once.time_label(true), "9:00 AM – 10:00 AM");
    }
}

//! **Mail to keep offline** (#143): how far back each account's mail is kept in full on this
//! computer. Older mail keeps its headers and preview here, stays in the message list, its
//! conversations and search, and is fetched from the server again when it is opened.

use mailcraft_time::{DateTime, Timestamp};
use serde::{Deserialize, Serialize};

/// Outlook's steps, shortest first. Mail received within it is kept in full on this computer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum KeepOffline {
    #[serde(rename = "3d")]
    Days3,
    #[serde(rename = "1w")]
    Week1,
    #[serde(rename = "2w")]
    Weeks2,
    #[serde(rename = "1m")]
    Month1,
    #[serde(rename = "3m")]
    Months3,
    #[serde(rename = "6m")]
    Months6,
    #[serde(rename = "12m")]
    Months12,
    #[serde(rename = "2y")]
    Years2,
    #[serde(rename = "5y")]
    Years5,
    #[serde(rename = "10y")]
    Years10,
    /// Everything: what a mailbox file from before there was a window says.
    #[default]
    #[serde(rename = "all")]
    All,
}

impl KeepOffline {
    /// Every step, shortest first (the slider's).
    pub const STEPS: [KeepOffline; 11] = [
        KeepOffline::Days3,
        KeepOffline::Week1,
        KeepOffline::Weeks2,
        KeepOffline::Month1,
        KeepOffline::Months3,
        KeepOffline::Months6,
        KeepOffline::Months12,
        KeepOffline::Years2,
        KeepOffline::Years5,
        KeepOffline::Years10,
        KeepOffline::All,
    ];

    /// What a new account starts with.
    pub const NEW_ACCOUNT: KeepOffline = KeepOffline::Months12;

    pub fn is_default(&self) -> bool {
        *self == KeepOffline::default()
    }

    /// Its code, as `account.edit` takes it and the mailbox file writes it.
    pub fn code(self) -> &'static str {
        match self {
            KeepOffline::Days3 => "3d",
            KeepOffline::Week1 => "1w",
            KeepOffline::Weeks2 => "2w",
            KeepOffline::Month1 => "1m",
            KeepOffline::Months3 => "3m",
            KeepOffline::Months6 => "6m",
            KeepOffline::Months12 => "12m",
            KeepOffline::Years2 => "2y",
            KeepOffline::Years5 => "5y",
            KeepOffline::Years10 => "10y",
            KeepOffline::All => "all",
        }
    }

    /// A code ([`KeepOffline::code`]), whatever its case, or `None`.
    pub fn parse(text: &str) -> Option<KeepOffline> {
        let text = text.trim();
        KeepOffline::STEPS.into_iter().find(|k| k.code().eq_ignore_ascii_case(text))
    }

    /// In words: `3 days`, `12 months`, `All`.
    pub fn label(self) -> &'static str {
        match self {
            KeepOffline::Days3 => "3 days",
            KeepOffline::Week1 => "1 week",
            KeepOffline::Weeks2 => "2 weeks",
            KeepOffline::Month1 => "1 month",
            KeepOffline::Months3 => "3 months",
            KeepOffline::Months6 => "6 months",
            KeepOffline::Months12 => "12 months",
            KeepOffline::Years2 => "2 years",
            KeepOffline::Years5 => "5 years",
            KeepOffline::Years10 => "10 years",
            KeepOffline::All => "All",
        }
    }

    /// The earliest received time kept in full at `now`; `None` for All. Months and years are
    /// calendar ones, in UTC (a month back from 31 March is the last day of February).
    pub fn since(self, now: Timestamp) -> Option<Timestamp> {
        let at = DateTime::from_timestamp(now, 0);
        let days = |n: i64| Some(now.saturating_sub(n.saturating_mul(86_400)));
        let months = |n: i32| Some(DateTime { date: at.date.add_months(-n), ..at }.to_timestamp(0));
        match self {
            KeepOffline::Days3 => days(3),
            KeepOffline::Week1 => days(7),
            KeepOffline::Weeks2 => days(14),
            KeepOffline::Month1 => months(1),
            KeepOffline::Months3 => months(3),
            KeepOffline::Months6 => months(6),
            KeepOffline::Months12 => months(12),
            KeepOffline::Years2 => months(24),
            KeepOffline::Years5 => months(60),
            KeepOffline::Years10 => months(120),
            KeepOffline::All => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: Timestamp = 1_792_000_000; // 2026-10-15

    #[test]
    fn codes_round_trip_and_order_runs_shortest_first() {
        for k in KeepOffline::STEPS {
            assert_eq!(KeepOffline::parse(k.code()), Some(k));
            assert_eq!(serde_json::to_value(k).unwrap(), serde_json::json!(k.code()));
        }
        assert_eq!(KeepOffline::parse(" 12M "), Some(KeepOffline::Months12));
        assert_eq!(KeepOffline::parse("forever"), None);
        assert!(KeepOffline::Days3 < KeepOffline::Month1 && KeepOffline::Years10 < KeepOffline::All);
        assert_eq!(KeepOffline::default(), KeepOffline::All);
    }

    #[test]
    fn since_counts_calendar_months_back() {
        assert_eq!(KeepOffline::Days3.since(NOW), Some(NOW - 3 * 86_400));
        assert_eq!(KeepOffline::All.since(NOW), None);
        // 31 March 2026 12:00 UTC, a month back: 28 February.
        let march = DateTime { date: mailcraft_time::Date::new(2026, 3, 31).unwrap(), hour: 12, minute: 0, second: 0 }.to_timestamp(0);
        let feb = DateTime { date: mailcraft_time::Date::new(2026, 2, 28).unwrap(), hour: 12, minute: 0, second: 0 }.to_timestamp(0);
        assert_eq!(KeepOffline::Month1.since(march), Some(feb));
        assert!(KeepOffline::Years10.since(NOW).unwrap() < KeepOffline::Years5.since(NOW).unwrap());
    }
}

//! MailCraft iCalendar: events and to-dos as RFC 5545 text (`.ics` files, meeting invitations).
//!
//! [`parse`] reads `VEVENT`s and `VTODO`s, [`write()`] writes them, and [`parse_rrule`] and
//! [`format_rrule`] turn a recurrence rule into the model's [`Recurrence`](mailcraft_model::Recurrence)
//! and back.
//!
//! The model keeps event times as local wall-clock times, so the reader converts on the way in: a
//! UTC time (`…Z`) becomes the local time through the caller's `offset_at`, and a date becomes an
//! all-day event. A time with a `TZID` is taken as local time as written, like a floating one:
//! `VTIMEZONE` blocks are skipped, not applied (only a `TZID` that names UTC is honoured). The
//! writer writes floating times.
//!
//! Input is hostile. Nothing here panics on any text: line lengths and counts are capped, and a
//! component that makes no sense is skipped.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod lines;
mod read;
mod rrule;
#[cfg(test)]
mod tests;
mod values;
mod write;

use mailcraft_model::{Event, Task};
use mailcraft_time::Timestamp;

pub use read::parse;
pub use rrule::{format_rrule, parse_rrule};
pub use write::write;

/// What an iCalendar text holds.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IcsFile {
    pub events: Vec<Event>,
    pub tasks: Vec<Task>,
    /// `METHOD` (`REQUEST`, `REPLY`, `CANCEL`, `PUBLISH`…), upper-cased; empty when absent.
    pub method: String,
}

/// What [`write()`] puts around the events and to-dos.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WriteOptions {
    /// `METHOD` to write; empty for none.
    pub method: String,
    /// `DTSTAMP` (UTC).
    pub now: Timestamp,
}

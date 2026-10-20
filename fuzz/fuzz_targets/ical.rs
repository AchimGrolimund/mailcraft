// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An iCalendar file and a recurrence rule.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let _ = mailcraft_ical::parse(&text, &|_| 0);
    let _ = mailcraft_ical::parse_rrule(&text);
});

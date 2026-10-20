// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! HTML mail to display blocks, HTML pasted into the composer, and the original under a reply or
//! forward, which must never carry what runs or embeds.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let _ = mailcraft_html::parse(&html);
    let _ = mailcraft_mime::rich::from_html(&html);
    let quoted = mailcraft_html::quotable(&html).to_ascii_lowercase();
    for tag in ["<script", "<style", "<iframe", "<object", "<embed"] {
        assert!(!quoted.contains(tag), "{tag} carried");
    }
});

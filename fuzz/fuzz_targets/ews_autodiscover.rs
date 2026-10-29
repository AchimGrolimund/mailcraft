// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Fuzzes Autodiscover (#252, E3): the state machine fed arbitrary answers, redirects and SRV
//! hosts, as a hostile server would.
//!
//! Invariants: nothing panics; the search ends; every request it makes is HTTPS; and with nobody
//! having agreed to anything, every request is to a host inside the owner's domain.

#![no_main]

use mailcraft_ews::http::{HttpResponse, Secret, TransportError};
use mailcraft_ews::{Autodiscover, Step};

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Ok(mut search) = Autodiscover::new("ada@example.org", Secret::new("Basic ZnV6ejpmdXp6")) else { return };
    // The input is cut into pieces: a status, a Location and a body for each answer.
    let mut pieces = data.split(|byte| *byte == 0xff);
    let text = String::from_utf8_lossy(data).into_owned();
    for _ in 0..64 {
        match search.step() {
            Step::Request(request) => {
                let url = request.url().to_ascii_lowercase();
                assert!(url.starts_with("https://"));
                let host = url.trim_start_matches("https://").split(['/', ':']).next().unwrap_or("");
                assert!(host == "example.org" || host.ends_with(".example.org"), "{host}");
                let piece = pieces.next().unwrap_or(&[]);
                let status = u16::from(piece.first().copied().unwrap_or(0)).saturating_add(200 + u16::from(piece.get(1).copied().unwrap_or(0)) % 3 * 100);
                let response = HttpResponse::new(status, piece.to_vec()).with_field("Location", &String::from_utf8_lossy(piece));
                search.response(if piece.first() == Some(&0) { Err(TransportError::Other) } else { Ok(response) });
            }
            Step::LookupSrv(_) => search.srv(Some(text.split_whitespace().map(str::to_owned).collect())),
            Step::NeedsYes(_) => search.decline(),
            Step::Done(_) => return,
        }
    }
    panic!("the search did not end");
});

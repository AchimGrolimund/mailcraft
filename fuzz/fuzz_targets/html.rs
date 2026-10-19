//! HTML mail to display blocks, and HTML pasted into the composer.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let html = String::from_utf8_lossy(data);
    let _ = mailcraft_html::parse(&html);
    let _ = mailcraft_mime::rich::from_html(&html);
});

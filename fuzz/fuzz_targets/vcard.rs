//! A vCard file, in any character set.
#![no_main]

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let _ = mailcraft_vcard::parse_bytes(data);
});

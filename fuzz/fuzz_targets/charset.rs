//! Bytes in a character set the message names: the first byte picks the label.
#![no_main]

const LABELS: [&str; 11] = ["", "utf-8", "utf-16", "utf-16be", "iso-2022-jp", "shift_jis", "gb18030", "big5", "euc-kr", "windows-1250", "x-unknown"];

libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    let Some((&pick, bytes)) = data.split_first() else { return };
    let label = LABELS.get(usize::from(pick) % LABELS.len()).copied().unwrap_or("");
    let _ = mailcraft_charset::decode(bytes, label);
    let _ = mailcraft_charset::guess(bytes);
});

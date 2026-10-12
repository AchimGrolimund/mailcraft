//! Bytes to text, for the character sets mail still arrives in.

/// Windows-1252 for 0x80–0x9F; the five holes keep their C1 control code, as browsers do.
const CP1252: [u32; 32] = [
    0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019,
    0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
];

/// Text from bytes in a named character set.
///
/// Knows UTF-8, US-ASCII, ISO-8859-1, ISO-8859-15 and Windows-1252. ISO-8859-1 is read as
/// Windows-1252, its superset, because mail that says Latin-1 very often holds curly quotes and
/// euro signs. A missing name, or US-ASCII with bytes above 0x7F, is guessed with [`guess`]. Any
/// other character set is read as UTF-8, with U+FFFD for what doesn't fit.
pub(crate) fn decode(bytes: &[u8], label: &str) -> String {
    if bytes.is_ascii() {
        return String::from_utf8_lossy(bytes).into_owned();
    }
    let label = label.trim().trim_matches(['"', '\'']).to_ascii_lowercase();
    match label.as_str() {
        "" | "us-ascii" | "ascii" | "ansi_x3.4-1968" | "iso646-us" => guess(bytes),
        "iso-8859-1" | "iso8859-1" | "iso_8859-1" | "latin1" | "latin-1" | "l1" | "cp819" | "ibm819" | "windows-1252" | "cp1252" | "cp-1252"
        | "x-cp1252" => bytes.iter().map(|b| cp1252(*b)).collect(),
        "iso-8859-15" | "iso8859-15" | "iso_8859-15" | "latin9" | "latin-9" | "l9" => bytes.iter().map(|b| latin9(*b)).collect(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Text from bytes that don't say what they are: UTF-8 when they are valid UTF-8, else Windows-1252.
pub(crate) fn guess(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|b| cp1252(*b)).collect(),
    }
}

fn cp1252(b: u8) -> char {
    let code = match b {
        0x80..=0x9F => CP1252.get(usize::from(b - 0x80)).copied().unwrap_or(u32::from(b)),
        _ => u32::from(b),
    };
    char::from_u32(code).unwrap_or(char::REPLACEMENT_CHARACTER)
}

/// ISO-8859-15: Latin-1 with eight characters replaced (the euro sign among them).
fn latin9(b: u8) -> char {
    match b {
        0xA4 => '\u{20AC}',
        0xA6 => '\u{0160}',
        0xA8 => '\u{0161}',
        0xB4 => '\u{017D}',
        0xB8 => '\u{017E}',
        0xBC => '\u{0152}',
        0xBD => '\u{0153}',
        0xBE => '\u{0178}',
        b => char::from(b),
    }
}

/// Line endings as `\n`: CRLF and a lone CR both become LF.
pub(crate) fn unix_lines(text: &str) -> String {
    if !text.contains('\r') {
        return text.to_string();
    }
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utf8_and_ascii() {
        assert_eq!(decode("héllo €".as_bytes(), "UTF-8"), "héllo €");
        assert_eq!(decode("héllo".as_bytes(), "utf8"), "héllo");
        assert_eq!(decode(b"plain", "x-unknown"), "plain");
        assert_eq!(decode(b"a\xFFb", "utf-8"), "a\u{fffd}b");
        assert_eq!(decode(b"", "utf-8"), "");
    }

    #[test]
    fn latin1_is_read_as_windows_1252() {
        assert_eq!(decode(b"caf\xE9 \xA4", "ISO-8859-1"), "café ¤");
        assert_eq!(decode(b"\x93hi\x94 \x80 \x85 \x99", "iso-8859-1"), "“hi” € … ™");
        assert_eq!(decode(b"\x93hi\x94 \x80", "\"Windows-1252\""), "“hi” €");
        assert_eq!(decode(b"\x81\x8D\x8F\x90\x9D", "cp1252"), "\u{81}\u{8d}\u{8f}\u{90}\u{9d}");
        // Every byte gives a character.
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(decode(&all, "windows-1252").chars().count(), 256);
    }

    #[test]
    fn latin9_has_the_euro() {
        assert_eq!(decode(b"\xA4 \xBC\xBD \xE9 \xA6\xA8\xB4\xB8\xBE", "ISO-8859-15"), "€ Œœ é ŠšŽžŸ");
        assert_eq!(decode(b"\x80", "latin9"), "\u{80}");
    }

    #[test]
    fn unlabelled_bytes_are_guessed() {
        assert_eq!(decode("naïve".as_bytes(), ""), "naïve");
        assert_eq!(decode(b"na\xEFve", ""), "naïve");
        assert_eq!(decode(b"na\xEFve", "us-ascii"), "naïve");
        assert_eq!(guess(b"\xE2\x82\xAC"), "€");
        assert_eq!(guess(b"\xE2\x82"), "\u{e2}\u{201a}");
    }

    #[test]
    fn unknown_charsets_fall_back_to_lossy_utf8() {
        assert_eq!(decode("grüße".as_bytes(), "koi8-r"), "grüße");
        assert_eq!(decode(b"\xC0\xC1", "shift_jis"), "\u{fffd}\u{fffd}");
    }

    #[test]
    fn line_endings() {
        assert_eq!(unix_lines("a\r\nb\rc\nd\r\n"), "a\nb\nc\nd\n");
        assert_eq!(unix_lines("\r\r\n\r"), "\n\n\n");
        assert_eq!(unix_lines("none"), "none");
    }
}

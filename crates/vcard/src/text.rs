//! Text values: cleaning, escaping, splitting, and the 2.1 encodings (quoted-printable, base64,
//! charsets).

use base64::Engine as _;

/// A one-line field: line breaks and other control characters become spaces, and the ends are
/// trimmed.
pub(crate) fn single(s: &str) -> String {
    let spaced: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    spaced.trim().to_string()
}

/// Text of several lines: `\r\n` and `\r` become `\n`; other control characters except tab are
/// dropped.
pub(crate) fn multi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut after_cr = false;
    for c in s.chars() {
        let cr = c == '\r';
        match c {
            '\n' if after_cr => {}
            '\r' | '\n' => out.push('\n'),
            '\t' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
        after_cr = cr;
    }
    out
}

/// A value ready to write: `\`, `;`, `,` and line breaks escaped.
pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len().saturating_add(8));
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            ';' => out.push_str("\\;"),
            ',' => out.push_str("\\,"),
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out
}

/// Undo [`escape`]: `\n` and `\N` are line breaks, `\\`, `\;`, `\,` (and the `\:` some writers
/// add) their second character; any other backslash is kept.
pub(crate) fn unescape(s: &str) -> String {
    if !s.contains('\\') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => out.push('\n'),
            Some(x @ ('\\' | ';' | ',' | ':')) => out.push(x),
            Some(x) => {
                out.push('\\');
                out.push(x);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// The first `max` pieces of `s`, split on its unescaped `separator`s; the pieces are still
/// escaped.
pub(crate) fn split(s: &str, separator: char, max: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let (mut start, mut escaped) = (0usize, false);
    for (i, c) in s.char_indices() {
        if out.len() >= max {
            return out;
        }
        if escaped {
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else if c == separator {
            out.push(s.get(start..i).unwrap_or(""));
            start = i.saturating_add(c.len_utf8());
        }
    }
    if out.len() < max {
        out.push(s.get(start..).unwrap_or(""));
    }
    out
}

/// One component of a structured value (a name part, a street) as one line of text. vCard 3.0 and
/// 4.0 let a component list several values with commas, which are joined with `joiner`; in 2.1
/// (`v21`) a comma is just a comma.
pub(crate) fn component(piece: &str, joiner: &str, v21: bool) -> String {
    let values = if v21 { vec![piece] } else { split(piece, ',', 32) };
    match values.as_slice() {
        [one] => single(&unescape(one)),
        many => many.iter().map(|v| single(&unescape(v))).filter(|v| !v.is_empty()).collect::<Vec<_>>().join(joiner),
    }
}

/// The bytes of a quoted-printable value (`=C3=A9`); soft line breaks are already gone, and a `=`
/// that isn't followed by two hex digits is kept.
pub(crate) fn quoted_printable(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let hex = |at: usize| bytes.get(at).and_then(|b| char::from(*b).to_digit(16)).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'='
            && let (Some(high), Some(low)) = (hex(i.saturating_add(1)), hex(i.saturating_add(2)))
        {
            out.push(high.saturating_mul(16).saturating_add(low));
            i = i.saturating_add(3);
        } else {
            out.push(b);
            i = i.saturating_add(1);
        }
    }
    out
}

/// The bytes of a base64 value, white space and padding or not; empty for junk.
pub(crate) fn base64_bytes(s: &str) -> Vec<u8> {
    let compact: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    base64::engine::general_purpose::STANDARD_NO_PAD.decode(compact.trim_end_matches('=')).unwrap_or_default()
}

/// Bytes as text in the named charset: UTF-8, or one of the Western single-byte sets. Without a
/// name it is UTF-8 when the bytes are valid UTF-8, and Windows-1252 otherwise.
pub(crate) fn decode(bytes: Vec<u8>, charset: &str) -> String {
    let western = |bytes: &[u8]| bytes.iter().map(|b| windows_1252(*b)).collect::<String>();
    match charset.trim().to_ascii_uppercase().as_str() {
        "ISO-8859-1" | "ISO8859-1" | "ISO_8859-1" | "LATIN1" | "LATIN-1" | "L1" | "WINDOWS-1252" | "CP1252" | "CP-1252" => western(&bytes),
        "UTF-8" | "UTF8" => String::from_utf8_lossy(&bytes).into_owned(),
        _ => String::from_utf8(bytes).unwrap_or_else(|e| western(e.as_bytes())),
    }
}

/// One Windows-1252 byte (a superset of what ISO 8859-1 prints) as a character.
fn windows_1252(b: u8) -> char {
    const HIGH: [char; 32] = [
        '\u{20ac}', '\u{81}', '\u{201a}', '\u{192}', '\u{201e}', '\u{2026}', '\u{2020}', '\u{2021}', '\u{2c6}', '\u{2030}', '\u{160}', '\u{2039}',
        '\u{152}', '\u{8d}', '\u{17d}', '\u{8f}', '\u{90}', '\u{2018}', '\u{2019}', '\u{201c}', '\u{201d}', '\u{2022}', '\u{2013}', '\u{2014}',
        '\u{2dc}', '\u{2122}', '\u{161}', '\u{203a}', '\u{153}', '\u{9d}', '\u{17e}', '\u{178}',
    ];
    HIGH.get(usize::from(b).wrapping_sub(0x80)).copied().unwrap_or(char::from(b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_one_line_and_many_line_text() {
        assert_eq!(single("  Ada\r\nLovelace\t\u{0}x  "), "Ada  Lovelace  x");
        assert_eq!(single("\n\u{85}\u{7f} "), "");
        assert_eq!(multi("a\r\nb\rc\n\nd\u{0}\u{7f}\te "), "a\nb\nc\n\nd\te ");
        assert_eq!(multi(""), "");
    }

    #[test]
    fn escapes_and_unescapes() {
        assert_eq!(escape("Research, Inc.; Lab \\ 7\nSecond\r\n"), "Research\\, Inc.\\; Lab \\\\ 7\\nSecond\\n");
        assert_eq!(unescape("Research\\, Inc.\\; Lab \\\\ 7\\nSecond\\NThird"), "Research, Inc.; Lab \\ 7\nSecond\nThird");
        assert_eq!(unescape("http\\://example.org C:\\Users\\x tail\\"), "http://example.org C:\\Users\\x tail\\");
        assert_eq!(unescape("plain"), "plain");
    }

    #[test]
    fn splits_on_unescaped_separators() {
        assert_eq!(split("Okafor;Nadia;;Dr.;", ';', 9), ["Okafor", "Nadia", "", "Dr.", ""]);
        assert_eq!(split("a\\;b;c\\\\;d", ';', 9), ["a\\;b", "c\\\\", "d"]);
        assert_eq!(split("a,b,c", ',', 2), ["a", "b"]);
        assert_eq!(split("", ';', 9), [""]);
        assert!(split("a;b", ';', 0).is_empty());
        assert_eq!(split("é;東;🎉", ';', 9), ["é", "東", "🎉"]);
    }

    #[test]
    fn components_join_their_lists() {
        assert_eq!(component("Quinlan,Paul", " ", false), "Quinlan Paul");
        assert_eq!(component("Quinlan\\,Paul", " ", false), "Quinlan,Paul");
        assert_eq!(component(" 41 Harbor Road , Suite 300,, ", ", ", false), "41 Harbor Road, Suite 300");
        assert_eq!(component("Smith, Jr.", " ", true), "Smith, Jr.");
        assert_eq!(component("  one\\nline  ", " ", false), "one line");
        assert_eq!(component("", ", ", false), "");
    }

    #[test]
    fn decodes_quoted_printable() {
        assert_eq!(quoted_printable("J=C3=BCrgen M=c3=bcller"), "Jürgen Müller".as_bytes());
        assert_eq!(quoted_printable("a=3Db=0D=0Ac"), b"a=b\r\nc");
        // What isn't an escape stays.
        assert_eq!(quoted_printable("1+1=2 =ZZ =4"), b"1+1=2 =ZZ =4");
        assert_eq!(quoted_printable("="), b"=");
        assert_eq!(quoted_printable("é=41"), "éA".as_bytes());
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_bytes("SGVsbG8sIHdvcmxk"), b"Hello, world");
        assert_eq!(base64_bytes(" SGVs\r\n bG8=\n"), b"Hello");
        assert_eq!(base64_bytes("SGVsbG8"), b"Hello");
        assert!(base64_bytes("not base64!").is_empty());
        assert!(base64_bytes("").is_empty());
    }

    #[test]
    fn decodes_charsets() {
        assert_eq!(decode("Jürgen".as_bytes().to_vec(), "utf-8"), "Jürgen");
        assert_eq!(decode("Jürgen".as_bytes().to_vec(), ""), "Jürgen");
        assert_eq!(decode(vec![b'J', 0xfc, b'r', b'g', b'e', b'n'], "ISO-8859-1"), "Jürgen");
        assert_eq!(decode(vec![0x80, b'5', 0x97, 0x93, b'x', 0x94], "windows-1252"), "€5—“x”");
        // No charset named and not UTF-8: Western.
        assert_eq!(decode(vec![b'J', 0xfc, b'r', b'g', b'e', b'n'], ""), "Jürgen");
        assert_eq!(decode(vec![b'J', 0xfc], "UTF-8"), "J\u{fffd}");
        assert_eq!(decode(vec![b'J', 0xfc], "x-unknown"), "Jü");
    }

    proptest::proptest! {
        #[test]
        fn clean_text_survives_escaping(s in "(\\PC|[\n\t;,\\\\:]){0,80}") {
            proptest::prop_assert_eq!(unescape(&escape(&s)), s.clone());
            // And as one piece of a structured value or a list.
            proptest::prop_assert_eq!(split(&format!("{};x", escape(&s)), ';', 9).first().map(|p| unescape(p)), Some(s.clone()));
            proptest::prop_assert_eq!(split(&format!("{},x", escape(&s)), ',', 9).first().map(|p| unescape(p)), Some(s));
        }

        #[test]
        fn text_helpers_never_panic(s in proptest::prelude::any::<String>(), bytes in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..64), max in 0usize..5) {
            let _ = (single(&s), multi(&s), unescape(&s), split(&s, ';', max), component(&s, ", ", false), quoted_printable(&s), base64_bytes(&s));
            let _ = (decode(bytes.clone(), ""), decode(bytes.clone(), "latin1"), decode(bytes, &s));
            proptest::prop_assert!(!escape(&multi(&s)).contains(['\r', '\n']));
            proptest::prop_assert!(!single(&s).contains(['\r', '\n']));
        }
    }
}

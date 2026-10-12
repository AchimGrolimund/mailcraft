//! HTML character references (`&amp;`, `&#233;`, `&#xE9;`) and escaping.

/// The Latin-1 names in code point order, from U+00A0 (`nbsp`) to U+00FF (`yuml`).
const LATIN1: [&str; 96] = [
    "nbsp", "iexcl", "cent", "pound", "curren", "yen", "brvbar", "sect", "uml", "copy", "ordf", "laquo", "not", "shy", "reg", "macr", "deg",
    "plusmn", "sup2", "sup3", "acute", "micro", "para", "middot", "cedil", "sup1", "ordm", "raquo", "frac14", "frac12", "frac34", "iquest", "Agrave",
    "Aacute", "Acirc", "Atilde", "Auml", "Aring", "AElig", "Ccedil", "Egrave", "Eacute", "Ecirc", "Euml", "Igrave", "Iacute", "Icirc", "Iuml", "ETH",
    "Ntilde", "Ograve", "Oacute", "Ocirc", "Otilde", "Ouml", "times", "Oslash", "Ugrave", "Uacute", "Ucirc", "Uuml", "Yacute", "THORN", "szlig",
    "agrave", "aacute", "acirc", "atilde", "auml", "aring", "aelig", "ccedil", "egrave", "eacute", "ecirc", "euml", "igrave", "iacute", "icirc",
    "iuml", "eth", "ntilde", "ograve", "oacute", "ocirc", "otilde", "ouml", "divide", "oslash", "ugrave", "uacute", "ucirc", "uuml", "yacute",
    "thorn", "yuml",
];

/// Greek capitals from U+0391; U+03A2 is unassigned.
const GREEK_UPPER: [&str; 25] = [
    "Alpha", "Beta", "Gamma", "Delta", "Epsilon", "Zeta", "Eta", "Theta", "Iota", "Kappa", "Lambda", "Mu", "Nu", "Xi", "Omicron", "Pi", "Rho", "",
    "Sigma", "Tau", "Upsilon", "Phi", "Chi", "Psi", "Omega",
];

/// Greek small letters from U+03B1.
const GREEK_LOWER: [&str; 25] = [
    "alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta", "iota", "kappa", "lambda", "mu", "nu", "xi", "omicron", "pi", "rho",
    "sigmaf", "sigma", "tau", "upsilon", "phi", "chi", "psi", "omega",
];

/// Windows-1252 for 0x80–0x9F: numeric references in that range mean these by long tradition.
const CP1252: [u32; 32] = [
    0x20AC, 0x81, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0x8D, 0x017D, 0x8F, 0x90, 0x2018, 0x2019,
    0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0x9D, 0x017E, 0x0178,
];

/// Names that also work without the closing `;` (old pages leave it off).
const LEGACY: [&str; 5] = ["amp", "lt", "gt", "quot", "nbsp"];

/// Longest entity name looked at.
const MAX_NAME: usize = 32;

/// Text as HTML: `&`, `<`, `>`, `"` and `'` become character references.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len().saturating_add(text.len() / 8));
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Decode character references: named (`&eacute;`, the HTML 4 set and a few more), decimal
/// (`&#233;`) and hex (`&#xE9;`). Anything that isn't a reference is left as it is.
pub fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find('&') {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i + 1..).unwrap_or("");
        match reference(after) {
            Some((c, used)) => {
                out.push(c);
                rest = after.get(used..).unwrap_or("");
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The character a reference names and the bytes it takes, given the text after its `&`.
pub(crate) fn reference(s: &str) -> Option<(char, usize)> {
    if let Some(num) = s.strip_prefix('#') {
        let (digits, radix, skip) = match num.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16, 2),
            None => (num, 10, 1),
        };
        let len = digits.bytes().take_while(|b| char::from(*b).is_digit(radix)).count();
        if len == 0 {
            return None;
        }
        // Too many digits is not a character; neither is 0, a surrogate, or anything past U+10FFFF.
        let mut code: u32 = 0;
        for d in digits.bytes().take(len) {
            let d = char::from(d).to_digit(radix).unwrap_or(0);
            code = code.checked_mul(radix).and_then(|c| c.checked_add(d)).unwrap_or(u32::MAX);
        }
        let semicolon = usize::from(digits.as_bytes().get(len) == Some(&b';'));
        return Some((numeric(code), skip + len + semicolon));
    }
    let len = s.bytes().take(MAX_NAME).take_while(u8::is_ascii_alphanumeric).count();
    let name = s.get(..len)?;
    match s.as_bytes().get(len) {
        Some(b';') => named(name).map(|c| (c, len + 1)),
        // `&amp=` inside a URL is a query parameter, not a reference.
        Some(b'=') => None,
        _ if LEGACY.contains(&name) => named(name).map(|c| (c, len)),
        _ => None,
    }
}

fn numeric(code: u32) -> char {
    let code = match code {
        0x80..=0x9F => CP1252.get((code - 0x80) as usize).copied().unwrap_or(code),
        c => c,
    };
    match char::from_u32(code) {
        Some('\0') | None => char::REPLACEMENT_CHARACTER,
        Some(c) => c,
    }
}

fn named(name: &str) -> Option<char> {
    if name.is_empty() {
        return None;
    }
    let at = |table: &[&str], base: u32| table.iter().position(|n| *n == name).and_then(|i| char::from_u32(base + i as u32));
    if let Some(c) = at(&LATIN1, 0xA0).or_else(|| at(&GREEK_UPPER, 0x391)).or_else(|| at(&GREEK_LOWER, 0x3B1)) {
        return Some(c);
    }
    let code: u32 = match name {
        "quot" | "QUOT" => 0x22,
        "amp" | "AMP" => 0x26,
        "apos" => 0x27,
        "lt" | "LT" => 0x3C,
        "gt" | "GT" => 0x3E,
        "Tab" => 0x09,
        "NewLine" => 0x0A,
        "excl" => 0x21,
        "num" => 0x23,
        "dollar" => 0x24,
        "percnt" => 0x25,
        "lpar" => 0x28,
        "rpar" => 0x29,
        "ast" => 0x2A,
        "plus" => 0x2B,
        "comma" => 0x2C,
        "period" => 0x2E,
        "sol" => 0x2F,
        "colon" => 0x3A,
        "semi" => 0x3B,
        "equals" => 0x3D,
        "quest" => 0x3F,
        "commat" => 0x40,
        "lbrack" | "lsqb" => 0x5B,
        "bsol" => 0x5C,
        "rbrack" | "rsqb" => 0x5D,
        "lowbar" => 0x5F,
        "grave" => 0x60,
        "lbrace" | "lcub" => 0x7B,
        "verbar" | "vert" => 0x7C,
        "rbrace" | "rcub" => 0x7D,
        "COPY" => 0xA9,
        "REG" => 0xAE,
        "OElig" => 0x152,
        "oelig" => 0x153,
        "Scaron" => 0x160,
        "scaron" => 0x161,
        "Yuml" => 0x178,
        "fnof" => 0x192,
        "circ" => 0x2C6,
        "tilde" => 0x2DC,
        "thetasym" => 0x3D1,
        "upsih" => 0x3D2,
        "piv" => 0x3D6,
        "ensp" => 0x2002,
        "emsp" => 0x2003,
        "emsp13" => 0x2004,
        "emsp14" => 0x2005,
        "numsp" => 0x2007,
        "puncsp" => 0x2008,
        "thinsp" => 0x2009,
        "hairsp" => 0x200A,
        "ZeroWidthSpace" => 0x200B,
        "zwnj" => 0x200C,
        "zwj" => 0x200D,
        "lrm" => 0x200E,
        "rlm" => 0x200F,
        "hyphen" | "dash" => 0x2010,
        "ndash" => 0x2013,
        "mdash" => 0x2014,
        "horbar" => 0x2015,
        "lsquo" => 0x2018,
        "rsquo" | "rsquor" => 0x2019,
        "sbquo" | "lsquor" => 0x201A,
        "ldquo" => 0x201C,
        "rdquo" | "rdquor" => 0x201D,
        "bdquo" | "ldquor" => 0x201E,
        "dagger" => 0x2020,
        "Dagger" => 0x2021,
        "bull" | "bullet" => 0x2022,
        "hellip" | "mldr" => 0x2026,
        "permil" => 0x2030,
        "prime" => 0x2032,
        "Prime" => 0x2033,
        "lsaquo" => 0x2039,
        "rsaquo" => 0x203A,
        "oline" => 0x203E,
        "frasl" => 0x2044,
        "NoBreak" => 0x2060,
        "euro" => 0x20AC,
        "image" => 0x2111,
        "weierp" => 0x2118,
        "real" => 0x211C,
        "trade" | "TRADE" => 0x2122,
        "alefsym" => 0x2135,
        "larr" => 0x2190,
        "uarr" => 0x2191,
        "rarr" => 0x2192,
        "darr" => 0x2193,
        "harr" => 0x2194,
        "crarr" => 0x21B5,
        "lArr" => 0x21D0,
        "uArr" => 0x21D1,
        "rArr" => 0x21D2,
        "dArr" => 0x21D3,
        "hArr" => 0x21D4,
        "forall" => 0x2200,
        "part" => 0x2202,
        "exist" => 0x2203,
        "empty" => 0x2205,
        "nabla" => 0x2207,
        "isin" => 0x2208,
        "notin" => 0x2209,
        "ni" => 0x220B,
        "prod" => 0x220F,
        "sum" => 0x2211,
        "minus" => 0x2212,
        "lowast" => 0x2217,
        "radic" => 0x221A,
        "prop" => 0x221D,
        "infin" => 0x221E,
        "ang" => 0x2220,
        "and" => 0x2227,
        "or" => 0x2228,
        "cap" => 0x2229,
        "cup" => 0x222A,
        "int" => 0x222B,
        "there4" => 0x2234,
        "sim" => 0x223C,
        "cong" => 0x2245,
        "asymp" => 0x2248,
        "ne" => 0x2260,
        "equiv" => 0x2261,
        "le" => 0x2264,
        "ge" => 0x2265,
        "sub" => 0x2282,
        "sup" => 0x2283,
        "nsub" => 0x2284,
        "sube" => 0x2286,
        "supe" => 0x2287,
        "oplus" => 0x2295,
        "otimes" => 0x2297,
        "perp" => 0x22A5,
        "sdot" => 0x22C5,
        "lceil" => 0x2308,
        "rceil" => 0x2309,
        "lfloor" => 0x230A,
        "rfloor" => 0x230B,
        "lang" => 0x2329,
        "rang" => 0x232A,
        "loz" => 0x25CA,
        "phone" => 0x260E,
        "starf" => 0x2605,
        "star" => 0x2606,
        "spades" => 0x2660,
        "clubs" => 0x2663,
        "hearts" => 0x2665,
        "diams" => 0x2666,
        "check" => 0x2713,
        "cross" => 0x2717,
        _ => return None,
    };
    char::from_u32(code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_the_five() {
        assert_eq!(escape(r#"<a href="x">Tom & 'Jerry'</a>"#), "&lt;a href=&quot;x&quot;&gt;Tom &amp; &#39;Jerry&#39;&lt;/a&gt;");
        assert_eq!(escape("plain ünïcode"), "plain ünïcode");
        assert_eq!(escape(""), "");
    }

    #[test]
    fn named_references() {
        assert_eq!(unescape("&lt;b&gt; &amp; &quot;x&quot; &apos;y&apos;"), "<b> & \"x\" 'y'");
        assert_eq!(unescape("a&nbsp;b"), "a\u{a0}b");
        assert_eq!(unescape("&copy; &reg; &trade; &hellip; &mdash; &ndash;"), "© ® ™ … — –");
        assert_eq!(unescape("&lsquo;a&rsquo; &ldquo;b&rdquo; &euro;5"), "‘a’ “b” €5");
        assert_eq!(unescape("caf&eacute; na&iuml;ve &Uuml;ber &szlig; &yuml;"), "café naïve Über ß ÿ");
        assert_eq!(unescape("&alpha;&beta;&Omega;&sigmaf; &rarr; &le; &ne;"), "αβΩς → ≤ ≠");
        assert_eq!(unescape("&zwnj;"), "\u{200c}");
    }

    #[test]
    fn names_are_case_sensitive() {
        assert_eq!(unescape("&Eacute;&eacute;"), "Éé");
        assert_eq!(unescape("&EACUTE;"), "&EACUTE;");
        assert_eq!(unescape("&AMP;&LT;"), "&<");
    }

    #[test]
    fn numeric_references() {
        assert_eq!(unescape("&#233;&#xE9;&#Xe9;"), "ééé");
        assert_eq!(unescape("&#128512;"), "😀");
        assert_eq!(unescape("&#x1F600;"), "😀");
        // Without the semicolon.
        assert_eq!(unescape("&#65 &#x42."), "A B.");
        // Windows-1252 for the C1 range.
        assert_eq!(unescape("&#147;hi&#148; &#128; &#x99;"), "“hi” € ™");
    }

    #[test]
    fn bad_numbers_become_the_replacement_character() {
        assert_eq!(unescape("&#0;"), "\u{fffd}");
        assert_eq!(unescape("&#xD800;"), "\u{fffd}");
        assert_eq!(unescape("&#x110000;"), "\u{fffd}");
        assert_eq!(unescape("&#99999999999999999999999999;"), "\u{fffd}");
        assert_eq!(unescape("&#xFFFFFFFFFFFFFFFFFFFF;"), "\u{fffd}");
    }

    #[test]
    fn invalid_references_are_left_alone() {
        for s in ["&", "&&", "& ", "&;", "&#;", "&#x;", "&#xZZ;", "&bogus;", "AT&T", "a & b", "&amp", "x&y=z", "&#", "&#x", "&eacute", "ünï&"] {
            let want = if s == "&amp" { "&" } else { s };
            assert_eq!(unescape(s), want, "{s}");
        }
    }

    #[test]
    fn legacy_names_work_without_a_semicolon_except_in_queries() {
        assert_eq!(unescape("a &lt b &gt c"), "a < b > c");
        assert_eq!(unescape("x&nbsp"), "x\u{a0}");
        assert_eq!(unescape("?a=1&amp=2&lt=3"), "?a=1&amp=2&lt=3");
        assert_eq!(unescape("?a=1&amp;b=2"), "?a=1&b=2");
        // A longer word that starts like a legacy name is not one.
        assert_eq!(unescape("&amplify"), "&amplify");
    }

    #[test]
    fn a_long_run_of_ampersands_is_quick() {
        let s = "&".repeat(200_000);
        assert_eq!(unescape(&s), s);
        let s = "&#".repeat(100_000);
        assert_eq!(unescape(&s), s);
    }

    proptest::proptest! {
        #[test]
        fn unescape_undoes_escape(s in proptest::prelude::any::<String>()) {
            proptest::prop_assert_eq!(unescape(&escape(&s)), s);
        }

        #[test]
        fn unescape_never_panics(s in "[&#xX0-9a-zA-Z;= é]{0,60}") {
            let _ = unescape(&s);
        }
    }
}

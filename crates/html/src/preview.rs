//! The first words of a message on one line, for a message list.
//!
//! A list shows what a message says, not how it was written: no markup, no character references,
//! no web addresses spelled out, and none of the invisible padding newsletters put after their
//! opening line to keep the rest of the message out of the preview. Reading stops as soon as there
//! are words enough, so a long message costs no more than a short one.

use crate::entities::reference;
use crate::tokenizer::{Token, Tokenizer};
use crate::{MAX_INPUT, truncate};

/// Most characters a preview is given; a request for more gets this many.
pub const MAX_PREVIEW_CHARS: usize = 2000;

/// Longest word kept, in characters: the rest of a longer one is dropped (no list shows it).
const MAX_WORD: usize = 400;

/// How far a `]` is looked for after `[image:` in a text body, in bytes.
const MAX_PLACEHOLDER: usize = 300;

/// Elements that sit inside a line of text: their tags do not end a word.
const INLINE: [&str; 33] = [
    "a", "abbr", "acronym", "b", "bdi", "bdo", "big", "cite", "code", "data", "del", "dfn", "em", "font", "i", "ins", "kbd", "label", "mark", "nobr",
    "q", "s", "samp", "small", "span", "strike", "strong", "sub", "sup", "time", "tt", "u", "var",
];

/// The first words an HTML message says, on one line, at most `max_chars` characters (and `…`
/// when there was more).
///
/// Text is read in the order it is written, the opening line a sender hid for message lists (the
/// preheader) included, with character references decoded and white space collapsed. Left out:
/// markup, comments, style sheets and scripts; characters that show nothing (zero-width spaces
/// and joiners, soft hyphens, the combining grapheme joiner, byte-order marks, direction marks)
/// and the no-break spaces between them; rows of dashes; web addresses, unless there is nothing
/// else; and what is quoted (`<blockquote>`), unless the message says nothing of its own. A
/// picture's alternative text is used only when there are no words at all.
pub fn preview(html: &str, max_chars: usize) -> String {
    let html = truncate(html, MAX_INPUT);
    let max = max_chars.min(MAX_PREVIEW_CHARS);
    let (mut said, mut quoted, mut pictures) = (Line::new(max), Line::new(max), Line::new(max));
    let mut quote_depth = 0usize;
    for token in Tokenizer::new(html) {
        let line = if quote_depth > 0 { &mut quoted } else { &mut said };
        match token {
            Token::Text(raw) => line.push_html(raw),
            Token::Open(tag) => {
                match tag.name.as_str() {
                    "blockquote" => quote_depth = quote_depth.saturating_add(1),
                    "img" if !pictures.full() => {
                        if let Some(alt) = tag.attr("alt") {
                            pictures.push_str(&alt);
                            pictures.end_word();
                        }
                    }
                    _ => {}
                }
                if !INLINE.contains(&tag.name.as_str()) {
                    line.end_word();
                }
            }
            Token::Close(name) => {
                if name == "blockquote" {
                    quote_depth = quote_depth.saturating_sub(1);
                }
                if !INLINE.contains(&name.as_str()) {
                    line.end_word();
                }
            }
        }
        if said.full() {
            break;
        }
    }
    first_said([said, quoted, pictures])
}

/// The first words a plain-text body says, on one line, at most `max_chars` characters (and `…`
/// when there was more).
///
/// Cleaned up as [`preview`] does it. Quoted lines (`>`) are left out unless there is nothing
/// else, and so are the `[image: …]` and `[cid:…]` marks mail programs write where a picture was.
/// A character reference written out in the text is text, except one that names padding
/// (`&zwnj;`, `&nbsp;`, `&#847;`): senders whose plain text is their HTML with the tags taken out
/// leave those behind.
pub fn text_preview(text: &str, max_chars: usize) -> String {
    let text = truncate(text, MAX_INPUT);
    let max = max_chars.min(MAX_PREVIEW_CHARS);
    let (mut said, mut quoted, mut pictures) = (Line::new(max), Line::new(max), Line::new(max));
    for row in text.split('\n') {
        let unquoted = row.trim_start().trim_start_matches(['>', ' ', '\t']);
        let (line, row) = if row.trim_start().starts_with('>') { (&mut quoted, unquoted) } else { (&mut said, row) };
        line.push_text(row, &mut pictures);
        line.end_word();
        if said.full() {
            break;
        }
    }
    first_said([said, quoted, pictures])
}

/// The first of `lines` that says something in words; failing that, the first web address one
/// of them left out.
fn first_said(lines: [Line; 3]) -> String {
    let mut address = String::new();
    for line in lines {
        let (words, left_out) = line.finish();
        if !words.is_empty() {
            return words;
        }
        if address.is_empty() {
            address = left_out;
        }
    }
    address
}

/// Whether a character shows nothing wherever it stands: soft hyphen, combining grapheme joiner,
/// zero-width space, word joiner, byte-order mark, the fillers and blanks used as padding, and
/// the marks that turn the direction of text (which have no business in a list of mail).
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{ad}'
            | '\u{34f}'
            | '\u{61c}'
            | '\u{115f}'
            | '\u{1160}'
            | '\u{17b4}'
            | '\u{17b5}'
            | '\u{180b}'..='\u{180f}'
            | '\u{200b}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2060}'..='\u{206f}'
            | '\u{2800}'
            | '\u{3164}'
            | '\u{feff}'
            | '\u{ffa0}'
            | '\u{fff9}'..='\u{fffc}'
    )
}

/// The zero-width joiner and non-joiner: padding between words, but part of a word when they
/// stand inside one (Persian spelling, emoji made of several).
fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200c}' | '\u{200d}')
}

/// Whether a character parts words: white space of every kind (no-break and figure spaces too)
/// and control characters.
fn is_space(c: char) -> bool {
    c.is_whitespace() || c.is_control()
}

/// A row of dashes, underscores, equals signs, asterisks or tildes: a rule drawn in text.
fn is_rule(word: &str) -> bool {
    word.len() >= 2 && word.chars().all(|c| matches!(c, '-' | '_' | '=' | '*' | '~'))
}

/// A web address written out, with or without brackets round it or a full stop after it.
fn is_address(word: &str) -> bool {
    let bare = word.trim_start_matches(['<', '(', '[', '"', '\'']);
    ["https://", "http://", "www."].iter().any(|scheme| {
        bare.len() > scheme.len() && bare.as_bytes().get(..scheme.len()).is_some_and(|start| start.eq_ignore_ascii_case(scheme.as_bytes()))
    })
}

/// One line being put together, word by word.
struct Line {
    /// The words so far, one space between them.
    out: String,
    /// Characters in `out`.
    len: usize,
    /// The most characters wanted.
    max: usize,
    /// The word being read, and its length in characters.
    word: String,
    word_len: usize,
    /// The word being read is longer than [`MAX_WORD`]: what is kept of it is not all of it.
    word_cut: bool,
    /// A joiner met inside the word: kept if a letter follows it.
    joiner: Option<char>,
    /// The first web address left out: shown when no line has words.
    address: String,
    /// There was more than fits.
    cut: bool,
}

impl Line {
    fn new(max: usize) -> Line {
        Line { out: String::new(), len: 0, max, word: String::new(), word_len: 0, word_cut: false, joiner: None, address: String::new(), cut: false }
    }

    /// Whether the line has all it can hold: nothing more needs to be read.
    fn full(&self) -> bool {
        self.cut
    }

    fn push(&mut self, c: char) {
        if is_space(c) {
            self.end_word();
        } else if is_joiner(c) {
            if self.word_len > 0 {
                self.joiner = Some(c);
            }
        } else if !is_invisible(c) {
            self.letter(c);
        }
    }

    /// A character that shows joins the word being read, behind the joiner that was waiting for it.
    fn letter(&mut self, c: char) {
        if self.word_len + 1 + usize::from(self.joiner.is_some()) > MAX_WORD {
            // Only a web address is this long without a space, and that is read past. Anything
            // else ends the line here: there is no need to read the rest of it.
            let address = self.word_cut || is_address(&self.word);
            self.word_cut = true;
            if !address {
                self.end_word();
            }
            return;
        }
        if let Some(joiner) = self.joiner.take() {
            self.word.push(joiner);
            self.word_len += 1;
        }
        self.word.push(c);
        self.word_len += 1;
    }

    fn push_str(&mut self, text: &str) {
        for c in text.chars() {
            if self.cut {
                break;
            }
            self.push(c);
        }
    }

    /// Character data as it stands in HTML: references are decoded on the way.
    fn push_html(&mut self, raw: &str) {
        let mut rest = raw;
        while !self.cut {
            let mut chars = rest.chars();
            let Some(c) = chars.next() else { break };
            let after = chars.as_str();
            match (c == '&').then(|| reference(after)).flatten() {
                Some((named, used)) => {
                    self.push(named);
                    rest = after.get(used..).unwrap_or("");
                }
                None => {
                    self.push(c);
                    rest = after;
                }
            }
        }
    }

    /// One line of a text body. What stands where a picture was goes to `pictures`.
    fn push_text(&mut self, row: &str, pictures: &mut Line) {
        let mut rest = row;
        while !self.cut {
            let mut chars = rest.chars();
            let Some(c) = chars.next() else { break };
            let after = chars.as_str();
            rest = after;
            match c {
                '[' => match placeholder(after) {
                    Some((alt, used)) => {
                        pictures.push_str(alt);
                        pictures.end_word();
                        self.end_word();
                        rest = after.get(used..).unwrap_or("");
                    }
                    None => self.push(c),
                },
                // Only padding: `AT&amp;T` in a text body is what its writer typed.
                '&' => match reference(after).filter(|(named, _)| is_space(*named) || is_joiner(*named) || is_invisible(*named)) {
                    Some((named, used)) => {
                        self.push(named);
                        rest = after.get(used..).unwrap_or("");
                    }
                    None => self.push(c),
                },
                c => self.push(c),
            }
        }
    }

    /// The word being read is complete: it joins the line, unless it is a rule or an address, or
    /// the line is full.
    fn end_word(&mut self) {
        self.joiner = None;
        let count = std::mem::take(&mut self.word_len);
        let word_cut = std::mem::take(&mut self.word_cut);
        if count == 0 {
            return;
        }
        let word = std::mem::take(&mut self.word);
        if self.cut || is_rule(&word) {
            return;
        }
        if is_address(&word) {
            if self.address.is_empty() {
                self.address = word;
            }
            return;
        }
        let gap = usize::from(self.len > 0);
        let room = self.max.saturating_sub(self.len);
        if count + gap <= room {
            if gap > 0 {
                self.out.push(' ');
            }
            self.out.push_str(&word);
            self.len += count + gap;
            self.cut = word_cut;
            return;
        }
        // The line ends at the word before, or inside this one when that would waste most of it.
        if self.len.saturating_mul(2) < self.max && room > gap {
            if gap > 0 {
                self.out.push(' ');
            }
            self.out.extend(word.chars().take(room - gap));
            while self.out.ends_with(is_joiner) {
                self.out.pop();
            }
            self.len = self.max;
        }
        self.cut = true;
    }

    /// The line, with `…` when there was more, and the address shown when no line has words (cut
    /// like a line).
    fn finish(mut self) -> (String, String) {
        self.end_word();
        if self.cut {
            self.out.push('…');
        }
        let mut address: String = self.address.chars().take(self.max).collect();
        while address.ends_with(is_joiner) {
            address.pop();
        }
        if address.len() < self.address.len() {
            address.push('…');
        }
        (self.out, address)
    }
}

/// What stands for a picture in a text body, given the text after a `[`: the words in
/// `[image: words]` (empty for `[cid:…]`) and how many bytes the mark takes.
fn placeholder(after: &str) -> Option<(&str, usize)> {
    let starts = |prefix: &str| after.as_bytes().get(..prefix.len()).is_some_and(|start| start.eq_ignore_ascii_case(prefix.as_bytes()));
    let (words_from, shown) = if starts("image:") {
        (6, true)
    } else if starts("cid:") {
        (4, false)
    } else {
        return None;
    };
    let end = truncate(after, MAX_PLACEHOLDER).find([']', '\n'])?;
    if after.as_bytes().get(end) != Some(&b']') {
        return None;
    }
    let words = if shown { after.get(words_from..end).unwrap_or("") } else { "" };
    Some((words, end + 1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_goes_and_references_are_decoded() {
        assert_eq!(preview("<p>Hello <b>wor</b>ld</p><p>Second&nbsp;line<br>third</p>", 200), "Hello world Second line third");
        assert_eq!(preview("<div>Caf&eacute; &amp; bar &#8212; &#x263A; &lt;now&gt;</div>", 200), "Café & bar — ☺ <now>");
        assert_eq!(preview("<table><tr><td>One</td><td>Two</td></tr><tr><td>Three</td></tr></table>", 200), "One Two Three");
        assert_eq!(preview("<ul><li>a</li><li>b</li></ul><h1>T</h1>x<hr>y", 200), "a b T x y");
        assert_eq!(preview("", 200), "");
        assert_eq!(preview("   \n\t ", 200), "");
        assert_eq!(preview("no markup at all", 200), "no markup at all");
    }

    #[test]
    fn what_never_shows_is_left_out() {
        let html = "<html><head><title>Title</title><style>p { color: red }</style><script>alert(1)</script></head>\
                    <body><!-- a comment --><!--[if mso]><p>For one program only</p><![endif]--><p>Shown</p></body></html>";
        assert_eq!(preview(html, 200), "Shown");
        // A tag that never ends and a reference that is none are read as far as they go.
        assert_eq!(preview("Before <b unfinished", 200), "Before");
        assert_eq!(preview("Fish &chips; &amp chips &#0; &#xFFFFFFFF;", 200), "Fish &chips; & chips \u{fffd} \u{fffd}");
    }

    #[test]
    fn the_padding_after_a_preheader_is_dropped() {
        // The opening line a sender hides for message lists is exactly what a list shows.
        let padding = "&zwnj;&nbsp;&#847; &#8199;&shy;&#65279;&ZeroWidthSpace;&#8288;\u{200c}\u{a0}\u{34f}\u{200b}\u{2060}\u{feff}\u{ad}".repeat(60);
        let html = format!(
            "<div style=\"display:none;max-height:0;overflow:hidden\">Autumn arrivals are here</div>\
             <div style=\"display:none\">{padding}</div><table><tr><td>View in browser</td></tr></table>"
        );
        assert_eq!(preview(&html, 200), "Autumn arrivals are here View in browser");
        // Written out in a text body, the same padding goes the same way.
        let text = format!("Autumn arrivals are here\n{}\nView in browser", "&zwnj; &nbsp; &#847; &zwnj;".repeat(40));
        assert_eq!(text_preview(&text, 200), "Autumn arrivals are here View in browser");
        assert_eq!(text_preview("&zwnj; &zwnj; &zwnj; &zwnj;", 200), "");
        assert_eq!(preview("&zwnj; &zwnj;<br>&nbsp;", 200), "");
        // Any other reference in a text body is what its writer typed.
        assert_eq!(text_preview("AT&amp;T said &lt;no&gt; &eacute;", 200), "AT&amp;T said &lt;no&gt; &eacute;");
    }

    #[test]
    fn joiners_stay_inside_words() {
        // Persian spells with the non-joiner; an emoji family is made with the joiner.
        assert_eq!(preview("می\u{200c}خواهم 👨\u{200d}👩\u{200d}👧", 200), "می\u{200c}خواهم 👨\u{200d}👩\u{200d}👧");
        assert_eq!(preview("a\u{200c} \u{200c}b\u{200d}", 200), "a b");
        assert_eq!(preview("\u{200d}\u{200c}", 200), "");
        // Marks that turn the direction of text are dropped: they could dress one name as another.
        assert_eq!(preview("invoice\u{202e}fdp.exe", 200), "invoicefdp.exe");
    }

    #[test]
    fn addresses_rules_and_pictures_only_when_nothing_else() {
        let link = r#"<a href="https://example.org/track?id=1">Read the story</a>"#;
        assert_eq!(preview(link, 200), "Read the story");
        assert_eq!(
            preview(r#"<a href="https://example.org/a">https://example.org/a</a> Offer ends <https://example.org/b> soon"#, 200),
            "Offer ends soon"
        );
        assert_eq!(preview("<p>https://example.org/only</p>", 200), "https://example.org/only");
        assert_eq!(preview("<p>-----</p><p>After the rule</p><p>=====</p>", 200), "After the rule");
        assert_eq!(preview("a - b", 200), "a - b");
        assert_eq!(preview(r#"<img src="cid:a" alt="Logo of Quillon"><p>Hello</p>"#, 200), "Hello");
        assert_eq!(preview(r#"<img src="cid:a" alt="Logo of Quillon"><img alt="&amp; more">"#, 200), "Logo of Quillon & more");
        assert_eq!(preview(r#"<img src="cid:a">"#, 200), "");
        assert_eq!(text_preview("[image: Logo] Hello [cid:image001.png@01DC] there www.example.org/x.", 200), "Hello there");
        assert_eq!(text_preview("[image: Logo of Quillon]", 200), "Logo of Quillon");
        assert_eq!(text_preview("[External] is kept, [image: never closed", 200), "[External] is kept, [image: never closed");
        assert_eq!(text_preview("See <https://example.org/x>.\nhttp://example.org", 200), "See");
        assert_eq!(text_preview("HTTPS://EXAMPLE.ORG/A", 200), "HTTPS://EXAMPLE.ORG/A");
    }

    #[test]
    fn quotes_are_left_out_unless_there_is_nothing_else() {
        let reply = "<div>Thanks, that works.</div><blockquote><p>Can you make Tuesday?</p><blockquote>Deeper</blockquote></blockquote><p>Ada</p>";
        assert_eq!(preview(reply, 200), "Thanks, that works. Ada");
        assert_eq!(preview("<blockquote><p>Can you make Tuesday?</p></blockquote>", 200), "Can you make Tuesday?");
        assert_eq!(preview("</blockquote></blockquote>Said after stray end tags", 200), "Said after stray end tags");
        assert_eq!(text_preview("Hello  there,\n\n> old text\n-----\nSee you “soon” — Ada", 200), "Hello there, See you “soon” — Ada");
        assert_eq!(text_preview("> old text\r\n>> older", 200), "old text older");
        assert_eq!(text_preview("One\r\nTwo\rThree", 200), "One Two Three");
    }

    #[test]
    fn a_line_is_cut_at_a_word_where_it_can_be() {
        assert_eq!(text_preview("Hello there, world", 12), "Hello there,…");
        assert_eq!(text_preview("Hello there, world", 11), "Hello there…");
        assert_eq!(text_preview("Hello there, world", 18), "Hello there, world");
        // A word too long for the line is cut inside, on a character boundary.
        assert_eq!(text_preview("Donaudampfschifffahrt", 5), "Donau…");
        assert_eq!(text_preview("né époustouflante", 6), "né épo…");
        assert_eq!(text_preview("Hello", 0), "…");
        assert_eq!(text_preview("", 0), "");
        assert_eq!(preview("<p>Hello there, world</p>", 11), "Hello there…");
        // With most of the line used, it ends at a word.
        assert_eq!(text_preview("Hello there, world", 16), "Hello there,…");
        // An address shown for want of anything else is cut the same way.
        assert_eq!(text_preview("https://example.org/a/long/path", 19), "https://example.org…");
        // More than a list could show is never handed over.
        assert_eq!(text_preview(&"word ".repeat(10_000), usize::MAX).chars().count(), MAX_PREVIEW_CHARS);
        let word = text_preview(&"x".repeat(10_000), 5000);
        assert!(word.chars().count() == MAX_WORD + 1 && word.ends_with('…'), "{word}");
        // An address longer than any word is still an address, and still left out.
        assert_eq!(text_preview(&format!("Read it https://example.org/{} here", "a".repeat(5000)), 200), "Read it here");
    }

    #[test]
    fn reading_stops_when_the_line_is_full() {
        // A message of megabytes costs what its first words cost.
        let long = format!("<p>First words of a long message</p>{}", "<p>and more</p>".repeat(140_000));
        let t0 = std::time::Instant::now();
        for _ in 0..2_000 {
            assert_eq!(preview(&long, 20), "First words of a…");
        }
        // Neither is a word without end read to its end.
        let endless = "x".repeat(2_000_000);
        for _ in 0..300 {
            assert_eq!(text_preview(&endless, 50), format!("{}…", "x".repeat(50)));
            assert_eq!(preview(&endless, 50), format!("{}…", "x".repeat(50)));
        }
        assert!(t0.elapsed().as_secs() < 20, "{:?}", t0.elapsed());
        let padded = format!("{}Said at last", "&zwnj;&nbsp;".repeat(150_000));
        assert_eq!(preview(&padded, 50), "Said at last");
        assert_eq!(text_preview(&padded, 50), "Said at last");
    }

    #[test]
    fn the_corpus_reads_as_a_person_would_tell_it() {
        use crate::corpus;
        for (name, html) in [
            ("outlook reply", corpus::OUTLOOK_REPLY),
            ("gmail reply", corpus::GMAIL_REPLY),
            ("newsletter", corpus::NEWSLETTER),
            ("receipt", corpus::RECEIPT),
            ("screenshots", corpus::SCREENSHOTS),
            ("calendar", corpus::CALENDAR),
            ("hostile remote", corpus::HOSTILE_REMOTE),
            ("hostile colours", corpus::HOSTILE_COLOURS),
        ] {
            let line = preview(html, 120);
            assert!(!line.is_empty(), "{name}");
            assert_clean(&line, 120);
            assert!(!line.contains("http") && !line.contains('<') && !line.contains("&nbsp") && !line.contains("&zwnj"), "{name}: {line}");
        }
        let line = text_preview(corpus::PLAIN_THREAD, 120);
        assert!(!line.is_empty() && !line.contains('>'), "{line}");
        assert_clean(&line, 120);
    }

    /// What every preview is, whatever it was made from.
    fn assert_clean(line: &str, max: usize) {
        let count = line.chars().count();
        assert!(count <= max.min(MAX_PREVIEW_CHARS) + 1, "{count} characters for {max}: {line:?}");
        assert!(count <= max.min(MAX_PREVIEW_CHARS) || line.ends_with('…'), "{line:?}");
        assert!(!line.chars().any(|c| c.is_control() || is_invisible(c) || (c.is_whitespace() && c != ' ')), "{line:?}");
        assert!(!line.starts_with(' ') && !line.ends_with(' ') && !line.contains("  "), "{line:?}");
        assert!(!line.starts_with(is_joiner) && !line.ends_with(is_joiner), "{line:?}");
    }

    proptest::proptest! {
        #[test]
        fn any_input_gives_a_clean_line(s in proptest::prelude::any::<String>(), max in 0usize..400) {
            assert_clean(&preview(&s, max), max);
            assert_clean(&text_preview(&s, max), max);
        }

        /// Markup, references, padding, addresses and quotes in any order, well formed or not.
        #[test]
        fn hostile_mixtures_give_a_clean_line(
            parts in proptest::collection::vec(
                proptest::prop_oneof![
                    "[a-zé]{1,12}",
                    " {1,3}",
                    "<[a-z/!]{0,3}[a-z]{0,10}( [a-z]{1,5}=[\"']?[a-z:/.]{0,12}[\"']?)?>?",
                    "&(zwnj|nbsp|shy|amp|lt|#847|#x200c|#0|#99999999|bogus);?",
                    "[\u{200b}-\u{200f}\u{ad}\u{34f}\u{2060}\u{feff}\u{a0}\u{202e}]{1,6}",
                    "(<|\\[|\\()?(https?://|www\\.)[a-z./?=&]{0,20}(>|\\]|\\))?",
                    "\\[(image|cid):? ?[a-z ]{0,10}\\]?",
                    "(\r\n|\n|\r)>{0,3} ?",
                    "[-_=*~]{1,6}",
                    "\\PC{0,4}",
                ],
                0..40,
            ),
            max in 0usize..200,
        ) {
            let s = parts.concat();
            let html = preview(&s, max);
            assert_clean(&html, max);
            let text = text_preview(&s, max);
            assert_clean(&text, max);
            // A line that fits is the same line however much room there is.
            if !html.ends_with('…') {
                proptest::prop_assert_eq!(&preview(&s, max + 50), &html);
            }
        }

        /// Words between padding come out as the words, in order.
        #[test]
        fn words_survive_any_padding(
            words in proptest::collection::vec("[a-z]{1,8}", 1..8),
            pads in proptest::collection::vec("([ \u{a0}\u{200b}\u{200c}\u{34f}\u{feff}]|&zwnj;|&nbsp;|&#847;){0,5}[ \u{a0}]", 8),
        ) {
            let mut s = String::new();
            for (word, pad) in words.iter().zip(pads.iter().cycle()) {
                s.push_str(pad);
                s.push_str(word);
            }
            let want = words.join(" ");
            proptest::prop_assert_eq!(preview(&s, 200), want.clone());
            proptest::prop_assert_eq!(text_preview(&s, 200), want.clone());
            proptest::prop_assert_eq!(preview(&format!("<p>{}</p>", s.replace(' ', "</p><p>")), 200), want);
        }
    }
}

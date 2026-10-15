use mailcraft_model::rich::{Format, ListKind, MAX_SIZE, MIN_SIZE, Paragraph, RichText, Run, Style, safe_link};
use proptest::prelude::*;

use super::*;

/// Every tag in `html` is one the set writes, with only the attributes it writes, and no `<`
/// stands alone in text. The whole check of "nothing outside the set".
fn assert_in_set(html: &str) {
    let mut rest = html;
    while let Some(at) = rest.find('<') {
        let after = &rest[at..];
        let end = after.find('>').unwrap_or_else(|| panic!("an unclosed tag in {html:?}"));
        let tag = &after[..=end];
        let ok = matches!(
            tag,
            "<div>"
                | "</div>"
                | "<br>"
                | "<li>"
                | "</li>"
                | "</ul>"
                | "</ol>"
                | "<b>"
                | "</b>"
                | "<i>"
                | "</i>"
                | "<u>"
                | "</u>"
                | "</span>"
                | "</a>"
        ) || tag == "<ul style=\"margin-top:0;margin-bottom:0\">"
            || tag == "<ol style=\"margin-top:0;margin-bottom:0\">"
            || tag
                .strip_prefix("<span style=\"font-size:")
                .and_then(|t| t.strip_suffix("pt\">"))
                .and_then(|n| n.parse::<u8>().ok())
                .is_some_and(|n| (MIN_SIZE..=MAX_SIZE).contains(&n))
            || tag
                .strip_prefix("<a href=\"")
                .and_then(|t| t.strip_suffix("\">"))
                .is_some_and(|url| !url.contains(['"', '<', '>']) && safe_link(&mailcraft_html::unescape(url)).is_some());
        assert!(ok, "{tag:?} is outside the set in {html:?}");
        rest = &after[end + 1..];
    }
    assert!(!html.contains(['\0']));
}

/// What any document read from HTML holds is within the set.
fn assert_clean(t: &RichText) {
    for p in t.paragraphs() {
        for r in &p.runs {
            assert!(!r.text.is_empty() && r.text.chars().all(|c| c == '\t' || !c.is_control()), "{r:?}");
            assert!(r.style.size.is_none_or(|n| (MIN_SIZE..=MAX_SIZE).contains(&n)), "{r:?}");
            assert!(r.style.link.as_deref().is_none_or(|l| safe_link(l).as_deref() == Some(l)), "{r:?}");
        }
    }
}

fn styled(text: &str, style: Style) -> Run {
    Run::new(text, style)
}

#[test]
fn the_set_is_written_as_html() {
    let t = RichText::from_paragraphs(vec![
        Paragraph::new(vec![Run::plain("Hi "), styled("Ada", Style { bold: true, ..Style::default() }), Run::plain(",")], ListKind::None),
        Paragraph::default(),
        Paragraph::new(vec![styled("one", Style { italic: true, underline: true, ..Style::default() })], ListKind::Bullet),
        Paragraph::new(vec![Run::plain("two")], ListKind::Bullet),
        Paragraph::new(vec![styled("big", Style { size: Some(18), ..Style::default() })], ListKind::Numbered),
        Paragraph::new(
            vec![styled("the <site> & \"more\"", Style { link: Some("https://example.org/?a=1&b=\"2\"".into()), ..Style::default() })],
            ListKind::None,
        ),
    ]);
    let html = to_html(&t);
    assert_eq!(
        html,
        "<div>Hi <b>Ada</b>,</div>\r\n<div><br></div>\r\n\
         <ul style=\"margin-top:0;margin-bottom:0\"><li><i><u>one</u></i></li>\r\n<li>two</li>\r\n</ul>\
         <ol style=\"margin-top:0;margin-bottom:0\"><li><span style=\"font-size:18pt\">big</span></li>\r\n</ol>\
         <div><a href=\"https://example.org/?a=1&amp;b=&quot;2&quot;\">the &lt;site&gt; &amp; &quot;more&quot;</a></div>\r\n"
    );
    assert_in_set(&html);
    assert_eq!(from_html(&html), t);
}

#[test]
fn white_space_is_kept_as_typed() {
    let t = RichText::from_plain("  two  spaces \tand a tab ");
    let html = to_html(&t);
    assert_eq!(html, "<div>&nbsp;&nbsp;two &nbsp;spaces &nbsp;&nbsp;&nbsp;&nbsp;and a tab&nbsp;</div>\r\n");
    // A tab comes back as spaces.
    assert_eq!(from_html(&html).text(), "  two  spaces     and a tab ");
}

#[test]
fn a_document_has_the_text_and_what_goes_below() {
    let t = RichText::from_plain("\n\nThanks!\n\n");
    let doc = to_html_document(&t, Some("<div id=\"x\">quoted</div>"));
    assert!(doc.starts_with("<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head><body>\r\n<div><br></div>"));
    assert!(doc.ends_with("<div id=\"x\">quoted</div>\r\n</body></html>\r\n"));
    // Blank lines at the edges come back too.
    let own = to_html_document(&t, None);
    assert_eq!(from_html(&own), t);
    assert_eq!(from_html(&to_html_document(&RichText::from_plain("\n\n"), None)).text(), "\n\n");
    assert_eq!(from_html(""), RichText::new());
}

#[test]
fn pasted_html_is_reduced_to_the_set() {
    let pasted = r#"<html><head><style>p{color:red}</style><script>alert(1)</script></head><body>
        <h1 style="font-family:Comic Sans">Title</h1>
        <p style="color:#f00;text-align:center;font-size:40px">Big <s>struck</s> <code>code</code> <strong>strong</strong> <em>em</em></p>
        <img src="https://tracker.example/x.png" alt="pixel"><hr>
        <ol><li>first</li><li><a href="javascript:alert(1)">bad link</a> <a href="cid:logo">cid</a> <a href="mailto:ada@example.org">ok</a></li></ol>
        <table><tr><td>cell 1</td><td><ul><li>in a cell</li></ul></td></tr></table>
        <p>line<br>break</p>
        </body></html>"#;
    let t = from_html(pasted);
    assert_clean(&t);
    assert_eq!(t.text(), "Title\n\nBig struck code strong em\n\nfirst\nbad link cid ok\n\ncell 1\nin a cell\n\nline\nbreak");
    let p = t.paragraphs();
    assert!(p[2].runs.iter().all(|r| r.style.size == Some(24)), "40px is shown at the most, 32px: 24pt");
    assert!(p[2].runs.iter().any(|r| r.style.bold && r.text == "strong"));
    assert_eq!(p[4].list, ListKind::Numbered);
    assert_eq!(p[5].runs.iter().filter_map(|r| r.style.link.as_deref()).collect::<Vec<_>>(), ["mailto:ada@example.org"]);
    assert_eq!(p[8].list, ListKind::Bullet);
    assert_eq!(p[10].list, ListKind::None);
    assert_in_set(&to_html(&t));
}

#[test]
fn sizes_survive_the_trip_in_points() {
    for pt in MIN_SIZE..=24 {
        let mut t = RichText::from_plain("x");
        t.format(0, 1, &Format::Size(Some(pt))).unwrap();
        assert_eq!(from_html(&to_html(&t)), t, "{pt}");
    }
    assert_eq!(px_to_points(10), MIN_SIZE);
    assert_eq!(px_to_points(255), MAX_SIZE);
}

fn arb_style() -> impl Strategy<Value = Style> {
    (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        proptest::option::of(any::<u8>()),
        proptest::option::of(prop_oneof![
            Just("https://example.org/a?b=c&d=\"e\"".to_string()),
            Just("mailto:ada@example.org".to_string()),
            Just("javascript:alert(1)".to_string()),
            "\\PC{0,20}",
        ]),
    )
        .prop_map(|(bold, italic, underline, size, link)| Style { bold, italic, underline, size, link })
}

fn arb_text() -> impl Strategy<Value = RichText> {
    proptest::collection::vec((proptest::collection::vec(("\\PC{0,12}", arb_style()), 0..4), 0u8..3), 1..8).prop_map(|ps| {
        RichText::from_paragraphs(
            ps.into_iter()
                .map(|(runs, list)| {
                    Paragraph::new(
                        runs.into_iter().map(|(t, s)| Run::new(t, s)).collect(),
                        [ListKind::None, ListKind::Bullet, ListKind::Numbered][list as usize],
                    )
                })
                .collect(),
        )
    })
}

/// Documents the HTML form can carry exactly: words separated by one space, sizes up to 24pt,
/// list items with text.
fn arb_exact() -> impl Strategy<Value = RichText> {
    let style = (
        any::<bool>(),
        any::<bool>(),
        any::<bool>(),
        proptest::option::of(MIN_SIZE..=24u8),
        proptest::option::of(Just("https://example.org/x".to_string())),
    )
        .prop_map(|(bold, italic, underline, size, link)| Style { bold, italic, underline, size, link });
    let paragraph = (proptest::collection::vec(("[a-zA-Z0-9&<>\"'é]{1,6}", style), 0..4), 0u8..3).prop_map(|(runs, list)| {
        let runs: Vec<Run> = runs.into_iter().enumerate().map(|(i, (t, s))| Run::new(if i == 0 { t } else { format!(" {t}") }, s)).collect();
        let list = if runs.is_empty() { ListKind::None } else { [ListKind::None, ListKind::Bullet, ListKind::Numbered][list as usize] };
        Paragraph::new(runs, list)
    });
    proptest::collection::vec(paragraph, 1..8).prop_map(RichText::from_paragraphs)
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(300))]

    /// Whatever is pasted, what comes of it is within the set, and so is its HTML.
    #[test]
    fn pasted_html_never_escapes_the_set(html in "(<(/?)(b|i|u|s|a|p|div|ul|ol|li|table|tr|td|span|font|img|script|style|br|h1|pre|blockquote)( href=\"(javascript:x|https://example.org|cid:y|mailto:a@b.example|data:,x)\"| style=\"(font-size:[0-9]{1,3}(px|pt)|color:red|margin:0)\"| src=\"https://example.org/p.png\")?>|[a-z &;<>\"\u{a0}\\x00\\r\\n\\t]{0,8}|&nbsp;|&#0;|&lt;script&gt;){0,40}") {
        let t = from_html(&html);
        assert_clean(&t);
        assert_in_set(&to_html(&t));
        assert_in_set(&to_html_document(&t, None).replace("<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head><body>", "").replace("</body></html>", ""));
    }

    /// Bytes that aren't HTML at all, too.
    #[test]
    fn any_text_pasted_as_html_stays_in_the_set(html in "\\PC{0,200}") {
        let t = from_html(&html);
        assert_clean(&t);
        assert_in_set(&to_html(&t));
    }

    /// Whatever the owner wrote, the HTML that goes out holds only the set.
    #[test]
    fn any_text_is_written_within_the_set(t in arb_text()) {
        assert_in_set(&to_html(&t));
    }

    /// What is written comes back as it was.
    #[test]
    fn the_set_survives_the_trip(t in arb_exact()) {
        prop_assert_eq!(from_html(&to_html_document(&t, None)), t);
    }

    /// However hostile the text and the subject, a message written from them has no header
    /// line that isn't a header, and no NUL or bare CR or LF anywhere in its header.
    #[test]
    fn a_message_from_any_text_keeps_its_headers(t in arb_text(), subject in "\\PC{0,40}|[\\r\\n\\x00a-z:]{0,20}") {
        let m = mailcraft_model::Message {
            subject,
            from: mailcraft_model::Address::new("Ada", "ada@example.org"),
            to: vec![mailcraft_model::Address::new("Bob\r\nBcc: x@example.org", "bob@example.org")],
            body: mailcraft_model::Body { text: t.to_plain(), html: Some(to_html_document(&t, None)) },
            ..Default::default()
        };
        let bytes = crate::write(&m, &crate::WriteOptions::default());
        let end = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(bytes.len());
        let head = &bytes[..end];
        prop_assert!(!head.contains(&0));
        for (i, line) in head.split(|b| *b == b'\n').enumerate() {
            prop_assert!(line.ends_with(b"\r") || i == head.split(|b| *b == b'\n').count() - 1, "a bare LF");
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            prop_assert!(!line.contains(&b'\r'), "a bare CR");
            let folded = line.first().is_some_and(|b| *b == b' ' || *b == b'\t');
            let field = line.iter().position(|b| *b == b':').is_some_and(|c| c > 0 && line[..c].iter().all(|b| b.is_ascii_graphic()));
            prop_assert!(folded || field, "not a header line: {:?}", String::from_utf8_lossy(line));
            let name = String::from_utf8_lossy(line).to_ascii_lowercase();
            prop_assert!(!name.starts_with("bcc:"), "an injected field");
        }
        // And the body comes back as it was sent.
        let back = crate::parse(&bytes);
        prop_assert_eq!(back.body.html.as_deref().map(from_html), Some(from_html(&to_html_document(&t, None))));
    }
}

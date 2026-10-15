//! Rich text (`mailcraft_model::rich`) to HTML and back.
//!
//! [`to_html`] writes the owner's text as the HTML part of a message: `<div>` paragraphs, `<ul>`
//! and `<ol>` lists, `<b>`, `<i>`, `<u>`, `<span style="font-size:…pt">` and `<a href>`, and
//! nothing else. Every character of text and of an address is escaped, and a link is written
//! only when its address is one the model accepts (`http`, `https`, `mailto`, `tel`).
//!
//! [`from_html`] is the way back, and the way in for anything pasted: the HTML is read by
//! `mailcraft_html` (which never runs or fetches anything) and what it gives is reduced to the
//! set. Tables give their cells' paragraphs in reading order; pictures, rules, colours, fonts,
//! alignment and everything else are dropped.

use mailcraft_html::{Block, BlockKind, Marker, Span};
use mailcraft_model::rich::{ListKind, MAX_SIZE, MIN_SIZE, Paragraph, RichText, Run, Style, safe_link};

/// The text as an HTML fragment (what goes inside `<body>`).
pub fn to_html(text: &RichText) -> String {
    let mut out = String::new();
    let mut open: Option<ListKind> = None;
    for p in text.paragraphs() {
        if open != Some(p.list) {
            close_list(&mut out, open);
            open = match p.list {
                ListKind::None => None,
                ListKind::Bullet => {
                    out.push_str("<ul style=\"margin-top:0;margin-bottom:0\">");
                    Some(ListKind::Bullet)
                }
                ListKind::Numbered => {
                    out.push_str("<ol style=\"margin-top:0;margin-bottom:0\">");
                    Some(ListKind::Numbered)
                }
            };
        }
        let tag = if p.list == ListKind::None { "div" } else { "li" };
        out.push('<');
        out.push_str(tag);
        out.push('>');
        if p.is_empty() {
            out.push_str("<br>");
        } else {
            runs(&mut out, p);
        }
        out.push_str("</");
        out.push_str(tag);
        out.push('>');
        out.push_str("\r\n");
    }
    close_list(&mut out, open);
    out
}

/// A whole HTML document holding the text, and under it `below` as it is (a quoted original).
pub fn to_html_document(text: &RichText, below: Option<&str>) -> String {
    let mut out = String::from("<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head><body>\r\n");
    out.push_str(&to_html(text));
    if let Some(below) = below {
        out.push_str(below);
        out.push_str("\r\n");
    }
    out.push_str("</body></html>\r\n");
    out
}

fn close_list(out: &mut String, open: Option<ListKind>) {
    match open {
        Some(ListKind::Bullet) => out.push_str("</ul>"),
        Some(ListKind::Numbered) => out.push_str("</ol>"),
        _ => {}
    }
}

/// The runs of a paragraph. White space is kept as typed: a space at the start or the end of the
/// paragraph, or after another, is a no-break space, and a tab is four.
fn runs(out: &mut String, p: &Paragraph) {
    let total: usize = p.runs.iter().map(|r| r.text.chars().count()).sum();
    let mut index = 0usize;
    let mut after_space = true;
    for run in &p.runs {
        let style = run.style.clone().clean();
        let mut close: Vec<&str> = Vec::new();
        if let Some(url) = &style.link {
            out.push_str("<a href=\"");
            out.push_str(&mailcraft_html::escape(url));
            out.push_str("\">");
            close.push("</a>");
        }
        for (on, open, end) in [(style.bold, "<b>", "</b>"), (style.italic, "<i>", "</i>"), (style.underline, "<u>", "</u>")] {
            if on {
                out.push_str(open);
                close.push(end);
            }
        }
        if let Some(size) = style.size {
            out.push_str(&format!("<span style=\"font-size:{}pt\">", size.clamp(MIN_SIZE, MAX_SIZE)));
            close.push("</span>");
        }
        for c in run.text.chars() {
            index += 1;
            match c {
                ' ' if after_space || index == total => out.push_str("&nbsp;"),
                ' ' => out.push(' '),
                '\t' => out.push_str("&nbsp;&nbsp;&nbsp;&nbsp;"),
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&#39;"),
                c if c.is_control() => {}
                c => out.push(c),
            }
            after_space = c == ' ' || c == '\t';
        }
        for end in close.iter().rev() {
            out.push_str(end);
        }
    }
}

/// Any HTML (a message, or what was pasted) reduced to rich text.
///
/// Blank lines at the very start and end of the text are kept when they are written the way
/// [`to_html`] writes them (`<div><br></div>`): a reader shows nothing there, but the owner put
/// them there.
pub fn from_html(html: &str) -> RichText {
    let blocks = mailcraft_html::parse(html);
    let mut reader = Reader { out: Vec::new() };
    reader.blocks(&blocks);
    let (lead, trail) = edge_blank_lines(html);
    let found = !reader.out.is_empty();
    let mut out: Vec<Paragraph> = std::iter::repeat_with(Paragraph::default).take(lead).collect();
    out.append(&mut reader.out);
    if found {
        out.extend(std::iter::repeat_with(Paragraph::default).take(trail));
    }
    RichText::from_paragraphs(out)
}

/// The empty paragraphs [`to_html`] wrote before the first paragraph with text and after the last.
fn edge_blank_lines(html: &str) -> (usize, usize) {
    const BLANK: &str = "<div><br></div>";
    let start = html.find("<body>").map_or(0, |at| at + "<body>".len());
    let end = html.rfind("</body>").unwrap_or(html.len());
    let Some(inner) = html.get(start..end.max(start)) else { return (0, 0) };
    let mut lead = 0;
    let mut rest = inner.trim_start();
    while let Some(r) = rest.strip_prefix(BLANK) {
        lead += 1;
        rest = r.trim_start();
    }
    let mut trail = 0;
    let mut rest = inner.trim_end();
    while let Some(r) = rest.strip_suffix(BLANK) {
        trail += 1;
        rest = r.trim_end();
    }
    (lead, trail)
}

struct Reader {
    out: Vec<Paragraph>,
}

impl Reader {
    fn blocks(&mut self, blocks: &[Block]) {
        for b in blocks {
            if self.out.len() >= mailcraft_model::rich::MAX_PARAGRAPHS {
                return;
            }
            match &b.kind {
                BlockKind::Table(table) => {
                    self.gap(b.space);
                    for cell in table.rows.iter().flat_map(|r| &r.cells) {
                        self.blocks(&cell.blocks);
                    }
                }
                BlockKind::Rule | BlockKind::Image { .. } => {}
                kind => {
                    let list = match kind {
                        BlockKind::ListItem { marker: Marker::Bullet, .. } => ListKind::Bullet,
                        BlockKind::ListItem { marker: Marker::Number(_) | Marker::Label(_), .. } => ListKind::Numbered,
                        _ => ListKind::None,
                    };
                    self.gap(b.space);
                    self.paragraph(&b.spans, list);
                }
            }
        }
    }

    /// The blank lines above a block, as empty paragraphs (none above the first).
    fn gap(&mut self, space: u8) {
        if self.out.is_empty() {
            return;
        }
        for _ in 0..space {
            self.out.push(Paragraph::default());
        }
    }

    fn paragraph(&mut self, spans: &[Span], list: ListKind) {
        let mut cur: Vec<Run> = Vec::new();
        for span in spans {
            let style = Style {
                bold: span.bold,
                italic: span.italic,
                underline: span.underline,
                size: span.size.map(px_to_points),
                link: span.link.as_deref().and_then(safe_link),
            };
            let text = span.text.replace('\u{a0}', " ");
            for (i, line) in text.split('\n').enumerate() {
                if i > 0 {
                    self.out.push(Paragraph::new(std::mem::take(&mut cur), list));
                }
                cur.push(Run::new(line, style.clone()));
            }
        }
        self.out.push(Paragraph::new(cur, list));
    }
}

/// CSS pixels to points, within the model's sizes.
fn px_to_points(px: u8) -> u8 {
    let points = (u16::from(px) * 3 + 2) / 4;
    u8::try_from(points).unwrap_or(MAX_SIZE).clamp(MIN_SIZE, MAX_SIZE)
}

#[cfg(test)]
mod tests;

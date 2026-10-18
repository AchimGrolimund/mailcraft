//! The search box draws each keyword the query language reads, with its value, as a pill
//! (`[From: stuart] [Subject: iqa]`), so the owner sees which part of a message is searched
//! (#196). The text is the query as typed and stays the only truth: a pill is only how the box
//! lays it out ([`mailcraft_model::search::keyword_spans`] says where the keywords are). The pill
//! the cursor is in or at the end of is drawn as the plain text it is, so Backspace into one
//! edits it as text; while the box isn't focused every one is drawn.

use std::ops::Range;
use std::sync::Arc;

use egui::text::{CCursor, LayoutJob, TextFormat};
use egui::{FontId, Galley, Pos2, Rect, Shape, pos2};

use crate::theme::{Tokens, semibold};

/// Room before a pill's keyword and after its value.
const PAD: f32 = 6.0;
/// How far a pill's edge is from its text.
const INSET: f32 = 4.0;

/// A keyword and its value drawn as a pill: where they stand in the text, in characters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pill {
    pub range: Range<usize>,
    pub key: Range<usize>,
}

/// The pills of `text`, in order, with the cursor at character `cursor` (`None`: the box isn't
/// being edited).
pub(crate) fn pills(text: &str, cursor: Option<usize>) -> Vec<Pill> {
    let chars = |byte: usize| text.get(..byte).map_or(0, |s| s.chars().count());
    mailcraft_model::search::keyword_spans(text)
        .into_iter()
        .map(|k| Pill { range: chars(k.range.start)..chars(k.range.end), key: chars(k.key.start)..chars(k.key.end) })
        .filter(|p| cursor.is_none_or(|c| c < p.range.start || c > p.range.end))
        .collect()
}

/// `text` laid out with `pills`: a pill's keyword in semibold and the accent's colour with its
/// first letter as a capital (the same characters otherwise, so the cursor stands where it
/// would), its value as the rest of the text, and room around it for the pill's edge.
pub(crate) fn layout(text: &str, pills: &[Pill], font: FontId, t: &Tokens) -> LayoutJob {
    let plain = TextFormat { font_id: font.clone(), color: t.text, ..Default::default() };
    let key = TextFormat { font_id: semibold(font.size), color: t.accent_text, ..Default::default() };
    let chars: Vec<char> = text.chars().collect();
    let part = |r: Range<usize>| -> String { chars.get(r).map(|c| c.iter().collect()).unwrap_or_default() };
    let mut job = LayoutJob { keep_trailing_whitespace: true, ..Default::default() };
    let mut add = |piece: String, lead: f32, format: &TextFormat| {
        if !piece.is_empty() {
            job.append(&piece, lead, format.clone());
        }
    };
    let mut at = 0;
    let mut lead = 0.0;
    for p in pills {
        if p.range.start < at || p.range.end > chars.len() {
            continue;
        }
        add(part(at..p.range.start), lead, &plain);
        // A `-` before the keyword, in the pill.
        add(part(p.range.start..p.key.start), PAD, &plain);
        let mut keyword = part(p.key.clone());
        if let Some(first) = keyword.get(..1).filter(|f| f.is_ascii()) {
            keyword = format!("{}{}", first.to_ascii_uppercase(), keyword.get(1..).unwrap_or_default());
        }
        add(keyword, if p.key.start == p.range.start { PAD } else { 0.0 }, &key);
        add(part(p.key.end..p.range.end), 0.0, &plain);
        at = p.range.end;
        lead = PAD;
    }
    add(part(at..chars.len()), lead, &plain);
    job
}

/// The pills' shapes for `galley`, drawn at `at`: rounded, in a tint of the selection's colour,
/// under the text.
pub(crate) fn shapes(galley: &Arc<Galley>, at: Pos2, pills: &[Pill], t: &Tokens) -> Vec<Shape> {
    let fill = if t.dark { t.selection } else { t.selection.gamma_multiply(0.7) };
    let total = galley.text().chars().count();
    pills
        .iter()
        .map(|p| {
            let start = galley.pos_from_cursor(CCursor::new(p.range.start));
            let end = galley.pos_from_cursor(CCursor::new(p.range.end));
            // The room after a pill belongs to what comes next: the pill's own edge is before it.
            let right = end.min.x - if p.range.end < total { PAD } else { 0.0 };
            let rect = Rect::from_min_max(pos2(start.min.x - INSET, start.min.y - 1.0), pos2(right + INSET, start.max.y + 1.0));
            Shape::rect_filled(rect.translate(at.to_vec2()), rect.height() / 2.0, fill)
        })
        .collect()
}

/// A colour for tests to compare.
#[cfg(test)]
pub(crate) fn key_color(job: &LayoutJob, char_index: usize) -> Option<egui::Color32> {
    let byte = job.text.char_indices().nth(char_index)?.0;
    job.sections.iter().find(|s| s.byte_range.start.0 <= byte && byte < s.byte_range.end.0).map(|s| s.format.color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_and_their_values_are_pills_unless_the_cursor_is_in_one() {
        let q = "from: stuart subject: iqa budget";
        assert_eq!(pills(q, None), [Pill { range: 0..12, key: 0..5 }, Pill { range: 13..25, key: 13..21 }]);
        // Being typed (the cursor in it or at its end): text.
        assert_eq!(pills(q, Some(25)), [Pill { range: 0..12, key: 0..5 }]);
        assert_eq!(pills(q, Some(3)), [Pill { range: 13..25, key: 13..21 }]);
        assert_eq!(pills(q, Some(32)).len(), 2);
        // Characters, not bytes; a `-` before the keyword is in its pill.
        assert_eq!(pills("Grüße -from:Đorđe", None), [Pill { range: 6..17, key: 7..12 }]);
        for q in ["subject:", "re: x", "is:nothing", ""] {
            assert!(pills(q, None).is_empty(), "{q}");
        }
    }

    #[test]
    fn a_pill_is_laid_out_with_the_same_characters_and_its_keyword_marked() {
        let t = Tokens::light();
        let q = "budget -from: stuart x";
        let p = pills(q, None);
        let job = layout(q, &p, egui::FontId::proportional(13.0), &t);
        assert_eq!(job.text, "budget -From: stuart x", "only the keyword's first letter is a capital");
        assert_eq!(job.text.chars().count(), q.chars().count(), "the cursor stands where it would");
        assert_eq!(key_color(&job, 8), Some(t.accent_text));
        assert_eq!(key_color(&job, 15), Some(t.text));
        assert_eq!(key_color(&job, 0), Some(t.text));
        assert!(job.keep_trailing_whitespace);
        // Nothing to mark: the text as it is.
        assert_eq!(layout("plain words", &[], egui::FontId::proportional(13.0), &t).text, "plain words");
        // Pills that don't fit the text are passed over, never a panic.
        let _ = layout("ab", &[Pill { range: 1..9, key: 1..3 }, Pill { range: 0..1, key: 0..1 }], egui::FontId::proportional(13.0), &t);
    }
}

//! The search box's query language: words, `"exact phrases"`, and `keyword:value` filters.
//!
//! `budget from:ada has:attachment -draft after:2026-10-01`. Every term must match; `-` in front
//! of a term excludes what it matches. Keywords: `from`, `to`, `cc`, `subject`, `body`,
//! `category`, `attachment` (file name), `has:attachment`, `is:unread|read|flagged|important`,
//! `importance:high|normal|low`, `before:`, `after:`, `on:` (dates, `YYYY-MM-DD` or `M/D/YYYY`).
//!
//! A word matches the start of a word, ignoring case and accents (`meet` finds "Meeting",
//! `gvozdar` finds "Gvožđar"); a phrase, its words one after another ([`crate::words`]). A
//! category matches anywhere in its name. Text with no letters or digits in it (`-`, `@`) asks
//! for nothing.

use mailcraft_time::{Date, DateTime};

use crate::words::{self, ATTACHMENT, BODY, CC, FROM, SUBJECT, TO};
use crate::{FlagStatus, Importance, Message, contains_ci};

/// Longest query read and most terms kept (the rest is ignored).
const MAX_QUERY: usize = 2_000;
const MAX_TERMS: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Kind {
    /// Words, one after another, in the parts of a message `fields` names.
    Words {
        fields: u8,
        words: Vec<String>,
    },
    Category(String),
    HasAttachment,
    Unread,
    Read,
    Flagged,
    Importance(Importance),
    Before(Date),
    After(Date),
    On(Date),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
    negate: bool,
    kind: Kind,
}

/// A parsed search.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query {
    terms: Vec<Term>,
}

/// Split on spaces outside double quotes; the quotes are dropped.
fn tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for c in s.chars().take(MAX_QUERY) {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any || !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => cur.push(c),
        }
    }
    if any || !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn parse_date(s: &str) -> Option<Date> {
    if let Some(d) = Date::parse_iso(s) {
        return Some(d);
    }
    let mut parts = s.split('/');
    let (m, d, y) = (parts.next()?.parse().ok()?, parts.next()?.parse().ok()?, parts.next()?.parse().ok()?);
    Date::new(y, m, d)
}

impl Query {
    pub fn parse(s: &str) -> Query {
        let mut terms = Vec::new();
        for token in tokens(s).into_iter().take(MAX_TERMS) {
            let (negate, body) = match token.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest),
                _ => (false, token.as_str()),
            };
            let text = || Kind::Words { fields: words::ALL, words: words::words(body) };
            let words_in = |fields: u8, v: &str| Kind::Words { fields, words: words::words(v) };
            let kind = match body.split_once(':') {
                Some((key, value)) if !value.is_empty() => {
                    let v = value.to_string();
                    match key.to_ascii_lowercase().as_str() {
                        "from" => words_in(FROM, &v),
                        "to" => words_in(TO, &v),
                        "cc" => words_in(CC, &v),
                        "subject" => words_in(SUBJECT, &v),
                        "body" => words_in(BODY, &v),
                        "category" | "categories" => Kind::Category(v),
                        "attachment" | "attachments" => words_in(ATTACHMENT, &v),
                        "has" | "hasattachment" | "hasattachments" => match v.to_ascii_lowercase().as_str() {
                            "attachment" | "attachments" | "yes" | "true" => Kind::HasAttachment,
                            _ => text(),
                        },
                        "is" => match v.to_ascii_lowercase().as_str() {
                            "unread" => Kind::Unread,
                            "read" => Kind::Read,
                            "flagged" => Kind::Flagged,
                            "important" => Kind::Importance(Importance::High),
                            _ => text(),
                        },
                        "importance" => Importance::parse(&v).map_or_else(text, Kind::Importance),
                        "before" => parse_date(&v).map_or_else(text, Kind::Before),
                        "after" => parse_date(&v).map_or_else(text, Kind::After),
                        "on" | "received" | "sent" => parse_date(&v).map_or_else(text, Kind::On),
                        // `10:30`, `re: budget`, a URL: plain text.
                        _ => text(),
                    }
                }
                _ => text(),
            };
            // Text with no words in it asks for nothing.
            if !matches!(&kind, Kind::Words { words, .. } if words.is_empty()) {
                terms.push(Term { negate, kind });
            }
        }
        Query { terms }
    }

    pub fn is_empty(&self) -> bool {
        self.terms.is_empty()
    }

    /// Whether any term reads a message's text (a bare word, or `body:`): such a search wants the
    /// bodies of the messages it looks at in memory.
    pub fn reads_bodies(&self) -> bool {
        (0..self.terms.len()).any(|i| self.term_reads_body(i))
    }

    /// How many terms it has.
    pub fn len(&self) -> usize {
        self.terms.len()
    }

    /// Term `i`, when it is words: the parts of a message it looks in ([`words::ALL`] for a bare
    /// word) and its words, folded, one after another. Other terms (`is:`, dates, `category:`)
    /// say `None`.
    pub fn words_of(&self, i: usize) -> Option<(u8, &[String])> {
        match &self.terms.get(i)?.kind {
            Kind::Words { fields, words } => Some((*fields, words)),
            _ => None,
        }
    }

    /// Whether term `i` reads a message's text.
    pub fn term_reads_body(&self, i: usize) -> bool {
        self.words_of(i).is_some_and(|(fields, _)| fields & BODY != 0)
    }

    /// Whether `m` matches every term. Dates compare in a zone `offset_minutes` east of UTC.
    pub fn matches(&self, m: &Message, offset_minutes: i32) -> bool {
        self.matches_with(m, offset_minutes, |_| None)
    }

    /// [`Query::matches`], with what `known` says of a term (whether `m` has what term `i` asks
    /// for, before its `-`; a word index's answer) in place of looking at `m`; `None`: look.
    pub fn matches_with(&self, m: &Message, offset_minutes: i32, known: impl Fn(usize) -> Option<bool>) -> bool {
        let day = || DateTime::from_timestamp(m.date, offset_minutes).date;
        self.terms.iter().enumerate().all(|(i, t)| {
            let hit = known(i).unwrap_or_else(|| match &t.kind {
                Kind::Words { fields, words } => words::texts(m, &m.body.text, |field, text| field & fields != 0 && words::starts_in(text, words)),
                Kind::Category(x) => m.categories.iter().any(|c| contains_ci(c, x)),
                Kind::HasAttachment => m.has_attachments(),
                Kind::Unread => !m.read,
                Kind::Read => m.read,
                Kind::Flagged => m.flag == FlagStatus::Flagged,
                Kind::Importance(i) => m.importance == *i,
                Kind::Before(d) => day() < *d,
                Kind::After(d) => day() > *d,
                Kind::On(d) => day() == *d,
            });
            hit != t.negate
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Address, Attachment};

    fn msg() -> Message {
        Message {
            from: Address::new("Ada Lovelace", "ada@example.org"),
            to: vec![Address::new("Charles", "charles@example.org")],
            cc: vec![Address::new("", "team@example.org")],
            subject: "Budget review: Q4".into(),
            body: crate::Body { text: "Numbers attached. See the engine estimate.".into(), html: None },
            attachments: vec![Attachment::new("estimate.xlsx", "application/octet-stream", vec![1, 2, 3])],
            categories: vec!["Blue category".into()],
            // 2026-10-11T23:30:00Z
            date: 1_791_761_400,
            ..Default::default()
        }
    }

    fn hit(q: &str) -> bool {
        Query::parse(q).matches(&msg(), 0)
    }

    #[test]
    fn words_and_phrases_must_all_match() {
        assert!(hit("budget"));
        assert!(hit("BUDGET engine"));
        assert!(hit("\"budget review\""));
        assert!(!hit("\"review budget\""));
        assert!(!hit("budget invoice"));
        assert!(hit("estimate.xlsx") && hit("charles") && hit("team@"));
        assert!(hit(""));
        assert!(Query::parse("  \"\"  ").is_empty());
    }

    #[test]
    fn keywords_filter_fields() {
        assert!(hit("from:ada") && !hit("from:charles"));
        assert!(hit("to:charles") && hit("cc:team") && !hit("to:team"));
        assert!(hit("subject:q4") && !hit("subject:engine") && hit("body:engine"));
        assert!(hit("from:\"Ada Lovelace\""));
        assert!(hit("has:attachment") && hit("hasattachment:yes") && hit("attachment:estimate"));
        assert!(hit("is:unread") && !hit("is:read") && !hit("is:flagged"));
        assert!(hit("category:blue") && !hit("category:red"));
        assert!(hit("importance:normal") && !hit("is:important"));
    }

    #[test]
    fn negation_and_dates() {
        assert!(hit("budget -invoice") && !hit("budget -engine") && !hit("-from:ada") && hit("-is:read"));
        assert!(hit("on:2026-10-11") && hit("after:2026-10-10") && hit("before:10/12/2026"));
        assert!(!hit("after:2026-10-11") && !hit("before:2026-10-11"));
        // In a zone an hour east it is already the 12th.
        assert!(Query::parse("on:2026-10-12").matches(&msg(), 60));
    }

    /// R6.2: a word matches the start of a word, whatever its case and accents; inside a word it
    /// doesn't (it used to: substring matching).
    #[test]
    fn words_match_word_starts_ignoring_case_and_accents() {
        assert!(hit("budg") && hit("rev") && hit("ESTIM") && hit("lovelace"));
        assert!(!hit("udget") && !hit("view") && !hit("ngine") && !hit("ovelace"));
        assert!(hit("\"budget rev\"") && hit("from:\"ada love\"") && !hit("from:\"ada charles\""));
        let m = Message { subject: "Sjednica u četvrtak".into(), from: Address::new("Đorđe Gvožđar", "dg@example.org"), ..Default::default() };
        for q in ["gvozdar", "from:GVOŽĐAR", "dorde", "cetvrtak", "četvrtak", "subject:sjed"] {
            assert!(Query::parse(q).matches(&m, 0), "{q}");
        }
        // Punctuation alone asks for nothing; with words, the words.
        assert!(Query::parse("- @ ... \"\"").is_empty());
        assert!(hit("ada@") && hit("@example") && !hit("@nowhere"));
    }

    #[test]
    fn what_is_known_of_a_term_stands_in_for_looking() {
        let q = Query::parse("budget -invoice is:unread");
        assert_eq!(q.len(), 3);
        assert_eq!(q.words_of(0), Some((words::ALL, ["budget".to_string()].as_slice())));
        assert!(q.words_of(2).is_none() && q.term_reads_body(1) && !q.term_reads_body(2));
        let m = msg();
        assert!(q.matches(&m, 0));
        // An index that says the message has "invoice": excluded; that it lacks "budget": too.
        assert!(!q.matches_with(&m, 0, |i| (i == 1).then_some(true)));
        assert!(!q.matches_with(&m, 0, |i| (i == 0).then_some(false)));
        assert!(!Query::parse("subject:x").reads_bodies() && Query::parse("body:x").reads_bodies());
    }

    #[test]
    fn unknown_keywords_and_junk_are_text() {
        assert!(hit("review: q4"));
        assert!(!hit("nosuch:thing"));
        assert!(!hit("before:someday"));
        assert!(Query::parse("-").matches(&Message { subject: "a - b".into(), ..Default::default() }, 0));
        let long = "x ".repeat(10_000);
        assert!(Query::parse(&long).terms.len() <= MAX_TERMS);
        let _ = Query::parse("\"unterminated from:").matches(&msg(), i32::MAX);
    }
}

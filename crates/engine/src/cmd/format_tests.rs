//! Writing in rich text through commands: the selection, typing, formatting, links, lists and
//! undo inside the message, and messages written before rich text.

use mailcraft_model::rich::{ListKind, Selection};
use mailcraft_time::{Date, DateTime};
use serde_json::{Value, json};

use crate::{Compose, Session, sample};

fn session() -> Session {
    let now = DateTime::new(Date::new(2026, 10, 14).unwrap(), 16, 0, 0).unwrap().to_timestamp(0);
    Session::at(sample::mailbox(now, 0), now, 0)
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn compose(s: &Session) -> &Compose {
    s.compose.as_ref().unwrap()
}

/// What was written above the signature the sample account adds.
fn mine(s: &Session) -> String {
    let text = compose(s).body.plain();
    text.split("\n\nAvery Quinn").next().unwrap_or("").to_string()
}

#[test]
fn typing_and_formatting_have_their_own_undo() {
    let mut s = session();
    run(&mut s, "mail.new", json!({"to": "maya@lumen.example"}));
    let mailbox_undo = s.can_undo();
    for c in ["Hel", "lo"] {
        run(&mut s, "compose.type", json!({"text": c}));
    }
    run(&mut s, "compose.type", json!({"text": "\nworld"}));
    assert!(compose(&s).body.plain().starts_with("Hello\nworld"));
    assert!(compose(&s).dirty);
    let r = run(&mut s, "format.bold", json!({"from": 0, "to": 5}));
    assert_eq!(r["bold"], true);
    assert!(compose(&s).body.text().all(0, 5, |st| st.bold).unwrap());
    run(&mut s, "format.size", json!({"size": 14}));
    run(&mut s, "format.bullets", json!({"from": 6}));
    assert_eq!(compose(&s).body.text().list_at(6), ListKind::Bullet);
    assert_eq!(run(&mut s, "format.numbering", json!({}))["list"], "numbered");
    run(&mut s, "compose.link", json!({"url": "https://example.org", "from": 6, "to": 11}));
    assert_eq!(compose(&s).body.text().link_span(8), Some((6, 11)));
    // Undo walks back through the message, and the mailbox's own history is untouched.
    for _ in 0..5 {
        run(&mut s, "compose.undo", json!({}));
    }
    assert!(!compose(&s).body.text().has_formatting());
    run(&mut s, "compose.undo", json!({}));
    assert_eq!(mine(&s), "Hello");
    run(&mut s, "compose.redo", json!({}));
    assert_eq!(mine(&s), "Hello\nworld");
    assert_eq!(s.can_undo(), mailbox_undo);
    run(&mut s, "compose.undo", json!({}));
    run(&mut s, "compose.undo", json!({}));
    assert!(s.run("compose.undo", &json!({})).is_err());
    assert!(!s.enabled("compose.undo") && s.enabled("compose.redo"));
}

#[test]
fn deleting_and_selecting() {
    let mut s = session();
    run(&mut s, "mail.new", json!({"body": "abc def"}));
    let r = run(&mut s, "compose.select", json!({"from": 7}));
    assert_eq!((r["from"].as_u64(), r["to"].as_u64()), (Some(7), Some(7)));
    run(&mut s, "compose.delete", json!({}));
    assert_eq!(mine(&s), "abc de");
    run(&mut s, "compose.select", json!({"from": 0}));
    run(&mut s, "compose.delete", json!({"forward": true}));
    run(&mut s, "compose.delete", json!({"from": 0, "to": 2}));
    assert_eq!(mine(&s), " de");
    assert_eq!(compose(&s).body.selection(), Selection::caret(0));
}

#[test]
fn hostile_positions_and_values_are_refused_and_change_nothing() {
    let mut s = session();
    run(&mut s, "mail.new", json!({"body": "short"}));
    let before = compose(&s).clone();
    for (id, params) in [
        ("compose.select", json!({"from": 99})),
        ("compose.select", json!({"from": -1})),
        ("compose.select", json!({})),
        ("compose.select", json!({"from": "0"})),
        ("format.bold", json!({"from": 0, "to": 18446744073709551615u64})),
        ("format.size", json!({"size": 300})),
        ("format.size", json!({"size": "big"})),
        ("format.size", json!({})),
        ("compose.link", json!({"url": "javascript:alert(1)", "from": 0, "to": 5})),
        ("compose.link", json!({"url": 5})),
        ("compose.type", json!({})),
        ("compose.delete", json!({"from": 3, "to": 1e300})),
    ] {
        assert!(s.run(id, &params).is_err(), "{id} {params}");
        assert_eq!(compose(&s), &before, "{id} {params}");
    }
    // Control characters typed are dropped; a typed line break is a paragraph.
    run(&mut s, "compose.select", json!({"from": 0}));
    run(&mut s, "compose.type", json!({"text": "\u{0}a\u{7}\r\n"}));
    assert_eq!(mine(&s), "a\nshort");
}

#[test]
fn format_commands_need_a_message_being_written() {
    let mut s = session();
    for id in
        ["format.bold", "format.italic", "format.underline", "format.bullets", "format.numbering", "format.clear", "compose.type", "compose.undo"]
    {
        assert!(!s.enabled(id), "{id}");
        assert!(s.run(id, &json!({"text": "x"})).is_err(), "{id}");
    }
}

#[test]
fn a_message_being_written_before_rich_text_still_opens() {
    let old = json!({"mode": "reply", "to": "maya@lumen.example", "subject": "RE: x", "body": "Thanks!\n\n-----Original Message-----\nFrom: Maya"});
    let c: Compose = serde_json::from_value(old).unwrap();
    assert_eq!(c.body.plain(), "Thanks!\n\n-----Original Message-----\nFrom: Maya");
    let again: Compose = serde_json::from_value(serde_json::to_value(&c).unwrap()).unwrap();
    assert_eq!(again.body.text(), c.body.text());
}

#[test]
fn a_plain_edit_keeps_the_formatting_around_it() {
    let mut s = session();
    run(&mut s, "mail.new", json!({"body": "Hello world"}));
    run(&mut s, "format.italic", json!({"from": 6, "to": 11}));
    let edited = compose(&s).body.plain().replacen("Hello world", "Hello, world", 1);
    run(&mut s, "compose.set", json!({"body": edited}));
    assert!(compose(&s).body.text().all(7, 12, |st| st.italic).unwrap());
    run(&mut s, "compose.undo", json!({}));
    assert_eq!(mine(&s), "Hello world");
}

//! Signatures through commands (R4.5.1): rich text, pictures, the two defaults, and mailboxes
//! from before there were any of these.

use mailcraft_time::{Date, DateTime};
use serde_json::{Value, json};

use crate::{Session, sample};

fn session() -> Session {
    let now = DateTime::new(Date::new(2026, 10, 14).unwrap(), 16, 0, 0).unwrap().to_timestamp(0);
    Session::at(sample::mailbox(now, 0), now, 0)
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn png() -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nIHDR....")
}

fn account(s: &Session) -> u64 {
    s.mailbox.default_account().unwrap().id
}

#[test]
fn an_account_has_one_signature_for_new_mail_and_one_for_replies() {
    let mut s = session();
    let a = account(&s);
    // The sample's account is from before there were two: it has "Standard" for both.
    let acct = s.mailbox.account(a).unwrap();
    assert_eq!((acct.signature.as_str(), acct.reply_signature()), ("Standard", "Standard"));
    run(&mut s, "signature.set", json!({"name": "Short", "text": "Avery"}));
    let r = run(&mut s, "signature.defaults", json!({"account": a, "reply": "short"}));
    assert_eq!((r["new"].as_str(), r["reply"].as_str()), (Some("Standard"), Some("Short")));
    // New gets the long one, a reply the short one.
    run(&mut s, "mail.new", json!({"account": a}));
    assert!(s.compose.as_ref().unwrap().body.plain().ends_with("Avery Quinn\nLumen Studio"));
    run(&mut s, "compose.discard", json!({}));
    let id = s.mailbox.messages.values().find(|m| m.account == a).unwrap().id;
    run(&mut s, "mail.select", json!({"id": id}));
    run(&mut s, "mail.reply", json!({}));
    let body = s.compose.as_ref().unwrap().body.plain();
    assert!(body.starts_with("\n\nAvery") && !body.contains("Lumen Studio\n\n"), "{body:?}");
    run(&mut s, "compose.discard", json!({}));
    // None for replies; changing the new one leaves replies alone.
    run(&mut s, "signature.defaults", json!({"account": a, "reply": ""}));
    run(&mut s, "signature.defaults", json!({"account": a, "new": "Short"}));
    let acct = s.mailbox.account(a).unwrap();
    assert_eq!((acct.signature.as_str(), acct.reply_signature()), ("Short", ""));
    assert!(s.run("signature.defaults", &json!({"account": a, "new": "Nope"})).is_err());
    // Renaming follows into the accounts; removing leaves them none.
    run(&mut s, "signature.rename", json!({"name": "short", "to": "Brief"}));
    assert_eq!(s.mailbox.account(a).unwrap().signature, "Brief");
    assert!(s.run("signature.rename", &json!({"name": "Brief", "to": "standard"})).is_err());
    run(&mut s, "signature.remove", json!({"name": "Brief"}));
    let acct = s.mailbox.account(a).unwrap();
    assert_eq!((acct.signature.as_str(), acct.reply_signature()), ("", ""));
}

#[test]
fn a_signature_takes_rich_text_pasted_html_and_pictures_within_the_caps() {
    let mut s = session();
    run(
        &mut s,
        "signature.set",
        json!({"name": "Work", "html": "<p><b style=\"color:#1F6FEB\">Avery Quinn</b><br><img src=\"https://example.org/logo.png\">Lumen</p>"}),
    );
    let sig = s.mailbox.signature("work").unwrap();
    // The web picture in the paste is not taken; the colour and the bold are.
    assert_eq!((sig.plain().as_str(), sig.pictures().len()), ("Avery Quinn\nLumen", 0));
    assert_eq!(sig.body().style_at(2).color.as_deref(), Some("#1f6feb"));
    let r = run(&mut s, "signature.picture.add", json!({"name": "Work", "data": png(), "file": "lumen-logo.png"}));
    let id = r["id"].as_u64().unwrap();
    assert_eq!(r["heavy"], false);
    run(&mut s, "signature.picture.set", json!({"name": "Work", "id": id, "width": 120, "alt": "Lumen Studio"}));
    run(&mut s, "signature.layout", json!({"name": "Work", "value": "pictureLeft"}));
    let sig = s.mailbox.signature("Work").unwrap();
    assert_eq!(sig.plain(), "[Lumen Studio]Avery Quinn\nLumen");
    assert_eq!(sig.picture(u32::try_from(id).unwrap()).unwrap().width, Some(120));
    // Not a picture, a web address, and a picture that isn't there.
    let junk = {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(b"<html>")
    };
    assert!(s.run("signature.picture.add", &json!({"name": "Work", "data": junk})).unwrap_err().to_string().contains("PNG, JPEG or GIF"));
    assert!(
        s.run("signature.picture.add", &json!({"name": "Work", "path": "https://example.org/logo.png"}))
            .unwrap_err()
            .to_string()
            .contains("not downloaded")
    );
    assert!(s.run("signature.picture.set", &json!({"name": "Work", "id": 99, "width": 10})).is_err());
    run(&mut s, "signature.picture.remove", json!({"name": "Work", "id": id}));
    assert!(s.mailbox.signature("Work").unwrap().pictures().is_empty());
    // A save and an open keep all of it.
    run(&mut s, "signature.picture.add", json!({"name": "Work", "data": png()}));
    let json = serde_json::to_string(&s.mailbox).unwrap();
    let back: mailcraft_model::Mailbox = serde_json::from_str(&json).unwrap();
    assert_eq!(back.signatures, s.mailbox.signatures);
}

#[test]
fn colour_and_font_are_formats_of_the_message() {
    let mut s = session();
    run(&mut s, "mail.new", json!({}));
    run(&mut s, "compose.set", json!({"format": "html"}));
    run(&mut s, "compose.type", json!({"text": "Hello"}));
    run(&mut s, "format.color", json!({"value": "#C00", "from": 0, "to": 5}));
    run(&mut s, "format.font", json!({"value": "georgia", "from": 0, "to": 5}));
    let body = &s.compose.as_ref().unwrap().body;
    let style = body.text().style_at(3);
    assert_eq!((style.color.as_deref(), style.font.map(|f| f.label())), (Some("#cc0000"), Some("Georgia")));
    assert!(s.run("format.color", &json!({"value": "red"})).is_err());
    assert!(s.run("format.font", &json!({"value": "Comic Sans"})).is_err());
    run(&mut s, "format.color", json!({"value": null, "from": 0, "to": 5}));
    assert_eq!(s.compose.as_ref().unwrap().body.text().style_at(3).color, None);
}

#[test]
fn a_mailbox_from_before_has_its_signatures_and_defaults_as_they_were() {
    let json = r#"{"accounts": [{"id": 1, "email": "avery@lumen.example", "signature": "Standard"}],
        "signatures": [{"name": "Standard", "text": "Avery Quinn\nLumen Studio"}]}"#;
    let mb: mailcraft_model::Mailbox = serde_json::from_str(json).unwrap();
    let a = mb.account(1).unwrap();
    assert_eq!((a.signature.as_str(), a.reply_signature()), ("Standard", "Standard"));
    assert_eq!(mb.signature("standard").unwrap().plain(), "Avery Quinn\nLumen Studio");
}

/// A signature with a logo beside the words, the default account's for new mail.
fn logo(s: &mut Session) {
    run(s, "signature.set", json!({"name": "Logo", "text": "Avery Quinn\nLumen Studio", "default": true}));
    let id =
        run(s, "signature.picture.add", json!({"name": "Logo", "data": png(), "file": "lumen-logo.png", "alt": "Lumen Studio logo"}))["id"].clone();
    run(s, "signature.picture.set", json!({"name": "Logo", "id": id, "width": 96}));
    run(s, "signature.layout", json!({"name": "Logo", "value": "pictureLeft"}));
}

#[test]
fn a_signature_with_a_logo_goes_out_with_the_picture_inside_the_message() {
    let mut s = session();
    logo(&mut s);
    run(&mut s, "mail.new", json!({"to": "maya@lumen.example", "subject": "Plan", "body": "Hello"}));
    run(&mut s, "compose.formatHtml", json!({}));
    let id = run(&mut s, "compose.send", json!({}))["id"].as_u64().unwrap();
    let m = s.mailbox.message(id).unwrap().clone();
    let pic = m.attachments.iter().find(|a| a.inline).expect("the logo goes with the message");
    assert!(pic.content_id.starts_with("sig-") && pic.content_id.ends_with("@mailcraft.invalid") && pic.mime == "image/png");
    let html = m.body.html.clone().unwrap();
    assert!(html.contains(&format!("cid:{}", pic.content_id)) && html.contains("Lumen Studio logo") && html.contains("Hello"), "{html}");
    // The plain-text part has the words, and the picture said in words.
    assert!(m.body.text.starts_with("Hello\n\n[Lumen Studio logo]Avery Quinn\nLumen Studio"), "{:?}", m.body.text);
    // On the wire the picture is part of the HTML, and it reads back.
    let bytes = mailcraft_mime::write(&m, &mailcraft_mime::WriteOptions::default());
    assert!(String::from_utf8_lossy(&bytes).contains("multipart/related"));
    let back = mailcraft_mime::parse(&bytes);
    let read = back.attachments.iter().find(|a| a.content_id == pic.content_id).expect("the picture reads back");
    assert_eq!(read.data, pic.data);
}

#[test]
fn a_plain_text_message_has_the_signature_in_words_and_no_pictures() {
    let mut s = session();
    logo(&mut s);
    run(&mut s, "mail.new", json!({"to": "maya@lumen.example", "body": "Hello"}));
    run(&mut s, "compose.formatPlain", json!({}));
    let id = run(&mut s, "compose.send", json!({}))["id"].as_u64().unwrap();
    let m = s.mailbox.message(id).unwrap();
    assert!(m.body.html.is_none() && m.attachments.is_empty());
    assert!(m.body.text.ends_with("[Lumen Studio logo]Avery Quinn\nLumen Studio"), "{:?}", m.body.text);
}

#[test]
fn changing_the_account_swaps_the_signature_unless_the_owner_edited_it() {
    let mut s = session();
    let a = account(&s);
    let b = run(&mut s, "account.add", json!({"email": "ada@example.org", "name": "Ada"}))["id"].as_u64().unwrap();
    let none = run(&mut s, "account.add", json!({"email": "bo@example.org", "name": "Bo"}))["id"].as_u64().unwrap();
    run(&mut s, "signature.set", json!({"name": "Ada", "text": "Ada Byron"}));
    run(&mut s, "signature.defaults", json!({"account": b, "new": "Ada"}));
    run(&mut s, "signature.defaults", json!({"account": none, "new": ""}));
    let body = |s: &Session| s.compose.as_ref().unwrap().body.plain();
    run(&mut s, "mail.new", json!({"account": a, "body": "Hi"}));
    run(&mut s, "compose.set", json!({"account": b}));
    assert_eq!(body(&s), "Hi\n\nAda Byron");
    run(&mut s, "compose.set", json!({"account": none}));
    assert!(!body(&s).contains("Ada"), "{:?}", body(&s));
    run(&mut s, "compose.discard", json!({}));
    // Edited, it stays as the owner made it.
    run(&mut s, "mail.new", json!({"account": b, "body": "Hi"}));
    let end = s.compose.as_ref().unwrap().body.text().len();
    run(&mut s, "compose.select", json!({"from": end}));
    run(&mut s, "compose.type", json!({"text": ", PhD"}));
    run(&mut s, "compose.set", json!({"account": a}));
    assert_eq!(body(&s), "Hi\n\nAda Byron, PhD");
}

#[test]
fn choosing_a_signature_replaces_the_one_in_the_message() {
    let mut s = session();
    run(&mut s, "signature.set", json!({"name": "Short", "text": "Avery"}));
    run(&mut s, "mail.new", json!({"body": "Hi"}));
    run(&mut s, "compose.signature", json!({"name": "Short"}));
    assert_eq!(s.compose.as_ref().unwrap().body.plain(), "Hi\n\nAvery");
    run(&mut s, "compose.signature", json!({}));
    assert_eq!(s.compose.as_ref().unwrap().body.plain(), "Hi\n\nAvery Quinn\nLumen Studio");
    // With none in it, it goes where the caret is.
    run(&mut s, "compose.discard", json!({}));
    run(&mut s, "signature.defaults", json!({"new": ""}));
    run(&mut s, "mail.new", json!({"body": "Hi"}));
    run(&mut s, "compose.select", json!({"from": 2}));
    run(&mut s, "compose.signature", json!({"name": "Short"}));
    assert_eq!(s.compose.as_ref().unwrap().body.plain(), "Hi\nAvery");
}

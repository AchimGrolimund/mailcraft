//! Content kept on disk as a session uses it: what Undo holds through a sweep, a search and a
//! rule that read bodies, and what bringing content in counts as (review of #87).

use std::sync::Mutex;

use mailcraft_model::{Action, Condition, ContentRef, Rule};

use super::*;
use crate::{CmdError, CommandSpec, Registry};

/// The message of `mb` with a long text and a file of its own, both kept outside the mailbox file.
fn with_text_and_file(mb: &Mailbox) -> Id {
    first_with(mb, |m| m.body.text.len() > INLINE_MAX && m.attachments.iter().any(|a| a.data.len() > INLINE_MAX && a.name.starts_with("file-")))
}

/// The index of `m`'s own file.
fn file_index(m: &Message) -> usize {
    m.attachments.iter().position(|a| a.name.starts_with("file-")).unwrap()
}

#[test]
fn undo_after_a_sweep_brings_back_the_files_of_a_message_read_before_it_was_deleted() {
    let tmp = Temp::new("undo-viewed");
    let path = tmp.file();
    let mb = mailbox(24);
    save(&path, &mb).unwrap();
    clear_cache();
    let mut s = session_without_bodies(&path);
    let victim = with_text_and_file(&mb);
    let i = file_index(mb.message(victim).unwrap());
    // Read in the reading pane: its text comes in; its file stays on disk.
    s.sel.messages = vec![victim];
    assert_eq!(s.load_for_display(victim), Ok(true));
    assert!(!s.mailbox.message(victim).unwrap().attachments[i].is_loaded());
    // Shift+Delete, the autosave, the first sweep, then Ctrl+Z.
    s.run("mail.deletePermanently", &json!({"ids": [victim]})).unwrap();
    save(&path, &s.mailbox).unwrap();
    sweep(&path, Duration::ZERO).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    clear_cache();
    let was = mb.message(victim).unwrap();
    assert_eq!(s.content(victim, crate::Part::Attachment(i)).unwrap().as_slice(), was.attachments[i].data.as_slice());
    save(&path, &s.mailbox).unwrap();
    assert!(whole(&path) == mb);
}

#[test]
fn undo_after_a_sweep_brings_back_a_body_that_was_let_go_of_before_the_delete() {
    let tmp = Temp::new("undo-evicted");
    let path = tmp.file();
    let mb = mailbox(24);
    save(&path, &mb).unwrap();
    clear_cache();
    let mut s = session_without_bodies(&path);
    let kept: Vec<Id> = newest_first(&mb).into_iter().filter(|id| s.mailbox.message(*id).unwrap().body_is_kept()).collect();
    let largest = kept.iter().map(|id| body_outside(mb.message(*id).unwrap())).max().unwrap();
    s.loaded_budget = largest;
    // The first is read, then the others, until it is let go of again.
    let victim = kept[0];
    assert_eq!(s.load_for_display(victim), Ok(true));
    for id in &kept[1..] {
        s.load_for_display(*id).unwrap();
    }
    assert!(s.mailbox.message(victim).unwrap().body_is_kept(), "its body went back to disk");
    s.run("mail.deletePermanently", &json!({"ids": [victim]})).unwrap();
    save(&path, &s.mailbox).unwrap();
    sweep(&path, Duration::ZERO).unwrap();
    s.run("edit.undo", &json!({})).unwrap();
    clear_cache();
    let was = mb.message(victim).unwrap();
    assert_eq!(s.content(victim, crate::Part::Text).unwrap().as_slice(), was.body.text.as_bytes());
    save(&path, &s.mailbox).unwrap();
    assert!(whole(&path) == mb);
}

#[test]
fn a_search_under_a_small_budget_finds_every_match_and_stays_within_it() {
    let tmp = Temp::new("search-budget");
    let path = tmp.file();
    let mut mb = mailbox(24);
    let ids: Vec<Id> = mb.messages.keys().copied().collect();
    let mut with_word = 0;
    for id in ids {
        let m = mb.message_mut(id).unwrap();
        if m.body.text.len() > INLINE_MAX {
            m.body.text.push_str("A word: zymurgy.\n");
            with_word += 1;
        }
    }
    assert_eq!(with_word, 16);
    save(&path, &mb).unwrap();
    clear_cache();
    let mut s = session_without_bodies(&path);
    let largest = mb.messages.values().map(|m| body_outside(m)).max().unwrap();
    // Room for about one body: every one brought in pushes an earlier one out.
    s.loaded_budget = largest;
    s.view.search = "zymurgy".into();
    let rows = s.rows();
    assert_eq!((rows.len(), s.unsearched), (16, 0));
    assert!(s.resident_bytes() <= largest * 2, "{} bytes kept", s.resident_bytes());
    assert!(s.mailbox.messages.values().filter(|m| m.body_is_kept()).count() > 8, "most went back to disk");
    // Searched again with them on disk, it finds them again; a word nobody wrote, nothing.
    s.view.search = "zymurgy ".into();
    assert_eq!(s.rows().len(), 16);
    s.view.search = "xylophone".into();
    assert_eq!((s.rows().len(), s.unsearched), (0, 0));
    // However long it is given (past what the clock can say), it never panics.
    s.search_time = Duration::MAX;
    s.view.search = "zymurgy  ".into();
    assert_eq!(s.rows().len(), 16);
}

#[test]
fn a_rule_that_reads_the_body_reads_one_kept_on_disk() {
    let tmp = Temp::new("rule-body");
    let path = tmp.file();
    let mut mb = mailbox(24);
    let old = *newest_first(&mb).iter().rev().find(|id| mb.message(**id).unwrap().body.text.len() > INLINE_MAX).unwrap();
    mb.message_mut(old).unwrap().body.text.push_str("Only here: quixotic.\n");
    save(&path, &mb).unwrap();
    clear_cache();
    let mut s = session_without_bodies(&path);
    assert!(s.mailbox.message(old).unwrap().body_is_kept());
    let inbox = s.mailbox.inbox().unwrap();
    let target = s.mailbox.folders.iter().find(|f| f.id != inbox).unwrap().id;
    let id = s.mailbox.alloc_id();
    s.mailbox.rules.push(Rule {
        id,
        name: "Quixotic".into(),
        enabled: true,
        conditions: vec![Condition::BodyContains("quixotic".into())],
        actions: vec![Action::MoveTo(target)],
        stop: false,
    });
    let said = s.run("rules.run", &json!({"folder": inbox})).unwrap();
    assert_eq!(said["matched"].as_u64(), Some(1));
    assert_eq!(s.mailbox.message(old).unwrap().folder, target);
}

/// A command that brings a whole message in, changes it, and fails.
fn load_then_fail(s: &mut Session, v: &Value) -> crate::CmdResult {
    let id = v["id"].as_u64().unwrap_or(0);
    s.load_message(id)?;
    if let Ok(m) = s.mailbox.message_mut(id) {
        m.read = true;
    }
    Err(CmdError::Failed("it fails".into()))
}

/// The bytes of `m`'s body and files in memory.
fn held(m: &Message) -> u64 {
    (m.body.text.len() + m.body.html.as_ref().map_or(0, String::len) + m.attachments.iter().map(|a| a.data.len()).sum::<usize>()) as u64
}

#[test]
fn bringing_content_in_is_no_change_and_a_failed_command_keeps_what_it_brought_counted() {
    let tmp = Temp::new("load-no-change");
    let path = tmp.file();
    let mb = mailbox(24);
    save(&path, &mb).unwrap();
    clear_cache();
    let mut s = session_without_bodies(&path);
    let kept: Vec<Id> = newest_first(&mb).into_iter().filter(|id| s.mailbox.message(*id).unwrap().body_is_kept()).collect();
    // A command that reads a message brings its text in: nothing to undo, save or redraw.
    let (rev, first) = (s.rev(), kept[0]);
    let before = held(s.mailbox.message(first).unwrap());
    s.run("mail.get", &json!({"id": first})).unwrap();
    assert!(!s.mailbox.message(first).unwrap().body_is_kept());
    assert!(s.rev() == rev && !s.dirty && !s.can_undo());
    assert_eq!(s.resident_bytes(), held(s.mailbox.message(first).unwrap()) - before);

    // A command that fails after bringing content in leaves the mailbox as it was, but for that
    // content, which stays, counted.
    let mut specs = crate::cmd::registry().all().to_vec();
    specs.push(CommandSpec::new("test.loadThenFail", "Load Then Fail", "Test", load_then_fail));
    s.registry = Arc::new(Registry::new(specs));
    let other = with_text_and_file(&mb);
    let (counted, before) = (s.resident_bytes(), held(s.mailbox.message(other).unwrap()));
    assert!(s.run("test.loadThenFail", &json!({"id": other})).is_err());
    let m = s.mailbox.message(other).unwrap();
    assert!(!m.read && m.is_loaded());
    assert_eq!(s.resident_bytes(), counted + held(m) - before);
    assert!(!s.dirty && !s.can_undo());
}

#[test]
fn a_body_brought_in_before_a_file_that_fails_is_counted() {
    let tmp = Temp::new("load-partial");
    let path = tmp.file();
    let mb = mailbox(24);
    save(&path, &mb).unwrap();
    clear_cache();
    let mut s = session_without_bodies(&path);
    let id = with_text_and_file(&mb);
    let was = mb.message(id).unwrap();
    fs::remove_file(blob_file(&path, &was.attachments[file_index(was)].data)).unwrap();
    let before = held(s.mailbox.message(id).unwrap());
    assert!(s.load_message(id).is_err());
    let m = s.mailbox.message(id).unwrap();
    assert!(!m.body_is_kept() && m.body.text == was.body.text);
    assert_eq!(s.resident_bytes(), held(m) - before);
    assert!(s.resident_bytes() > 0);
}

/// A source of content that notes the length of everything asked of it.
struct Noting {
    asked: Mutex<Vec<u64>>,
}

impl crate::io::ContentSource for Noting {
    fn fetch(&self, mailbox: &Path, r: &ContentRef) -> Result<Arc<Vec<u8>>, String> {
        self.asked.lock().unwrap().push(r.len);
        Content.fetch(mailbox, r)
    }
}

#[test]
fn a_reply_reads_the_text_and_its_pictures_and_a_calendar_part_too_large_is_not_read() {
    let tmp = Temp::new("reply-parts");
    let path = tmp.file();
    let mut mb = mailbox(4);
    let inbox = mb.inbox().unwrap();
    let mut m = Message { folder: inbox, subject: "With a picture".into(), from: Address::new("Ada", "ada@example.org"), ..Default::default() };
    m.body.text = long_text(3);
    m.body.html = Some(format!("<html><body><p>{}</p><img src=\"cid:logo@pic.example\"></body></html>", long_text(4)));
    let logo = Attachment { content_id: "logo@pic.example".into(), ..Attachment::new("logo.png", "image/png", noise(31, 3_001)) };
    m.attachments.push(logo);
    m.attachments.push(Attachment::new("report.bin", "application/octet-stream", noise(32, 9_003)));
    // An invitation too large to be read, and one that is not.
    m.attachments.push(Attachment::new("huge.ics", "text/calendar", noise(33, crate::session::MAX_CALENDAR_PART + 1)));
    m.attachments.push(Attachment::new("invite.ics", "text/calendar", b"BEGIN:VCALENDAR\r\nEND:VCALENDAR\r\n".repeat(40)));
    let id = mb.add_message(m).unwrap();
    save(&path, &mb).unwrap();
    clear_cache();
    let mut s = session_without_bodies(&path);
    let noting = Arc::new(Noting { asked: Mutex::new(Vec::new()) });
    s.content_source = Some(noting.clone());

    s.run("mail.reply", &json!({"id": id})).unwrap();
    let asked = std::mem::take(&mut *noting.asked.lock().unwrap());
    assert!(asked.contains(&3_001), "the picture is read: {asked:?}");
    assert!(!asked.contains(&9_003), "the file isn't: {asked:?}");
    let quote = s.compose.as_ref().unwrap().quote.as_ref().unwrap();
    assert_eq!(quote.pictures.len(), 1);
    assert_eq!(quote.pictures[0].data.len(), 3_001);
    s.compose = None;

    // Shown, the small invitation comes in; the large one stays where it is, and is not wanted.
    s.sel.messages = vec![id];
    s.load_for_display(id).unwrap();
    let m = s.mailbox.message(id).unwrap();
    assert!(m.attachments[3].is_loaded() && !m.attachments[2].is_loaded() && !m.attachments[1].is_loaded());
    assert!(!s.needs_load_for_display(id));
    noting.asked.lock().unwrap().clear();
    let _ = s.run("calendar.invitation", &json!({"message": id}));
    let large = (crate::session::MAX_CALENDAR_PART + 1) as u64;
    assert!(!noting.asked.lock().unwrap().contains(&large));
}

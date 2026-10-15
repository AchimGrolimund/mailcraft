//! The body of a message being written, driven with the keyboard and the pointer in a window
//! without a window: typing, the formatting keys (the body's only while it has the keyboard),
//! lists, moving about, the clipboard, and a long message.

use egui::{Event, Key, Modifiers};
use egui_kittest::Harness;
use mailcraft_engine::Session;
use mailcraft_model::rich::{ListKind, Selection};
use serde_json::json;

use super::editor;
use crate::{MailApp, Services};

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

type Window = Harness<'static, MailApp>;

fn app() -> MailApp {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app
}

fn window(app: MailApp) -> Window {
    let mut harness = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(
        |ui, app: &mut MailApp| {
            let ctx = ui.ctx().clone();
            app.logic(&ctx);
            app.ui(ui);
        },
        app,
    );
    harness.run_steps(3);
    harness
}

/// A new message, with the keyboard in its body and nothing in it.
fn writing() -> Window {
    let mut a = app();
    a.execute("mail.new", json!({"to": "maya@lumen.example"})).unwrap();
    a.execute("compose.set", json!({"body": ""})).unwrap();
    a.execute("compose.select", json!({"from": 0})).unwrap();
    a.body_focus_wanted = true;
    let mut h = window(a);
    h.run_steps(2);
    assert!(editor::has_focus(&h.ctx));
    h
}

fn key_with(h: &mut Window, modifiers: Modifiers, key: Key) {
    for pressed in [true, false] {
        h.event(Event::Key { key, physical_key: None, pressed, repeat: false, modifiers });
        h.step();
    }
    h.run_steps(1);
}

fn key(h: &mut Window, key: Key) {
    key_with(h, Modifiers::NONE, key);
}

fn type_in(h: &mut Window, text: &str) {
    h.event(Event::Text(text.into()));
    h.run_steps(2);
}

fn plain(h: &Window) -> String {
    h.state().session.compose.as_ref().unwrap().body.plain()
}

fn sel(h: &Window) -> Selection {
    h.state().session.compose.as_ref().unwrap().body.selection()
}

fn kinds(h: &Window) -> Vec<ListKind> {
    h.state().session.compose.as_ref().unwrap().body.text().paragraphs().iter().map(|p| p.list).collect()
}

#[test]
fn typing_and_the_formatting_keys() {
    let mut h = writing();
    type_in(&mut h, "Hello");
    key_with(&mut h, Modifiers::COMMAND, Key::B);
    // The ribbon says what the next character will be.
    assert!(crate::ribbon::look(h.state().session.compose.as_ref().unwrap()).bold);
    type_in(&mut h, " world");
    key_with(&mut h, Modifiers::COMMAND, Key::B);
    key_with(&mut h, Modifiers::COMMAND, Key::I);
    key_with(&mut h, Modifiers::COMMAND, Key::U);
    type_in(&mut h, "!");
    let body = h.state().session.compose.as_ref().unwrap().body.clone();
    let runs = &body.text().paragraphs()[0].runs;
    assert_eq!(runs.iter().map(|r| r.text.as_str()).collect::<Vec<_>>(), ["Hello", " world", "!"]);
    assert!(!runs[0].style.bold && runs[1].style.bold && !runs[1].style.italic);
    assert!(runs[2].style.italic && runs[2].style.underline && !runs[2].style.bold);
    // Enter starts a paragraph; Backspace takes back a character; the caret follows.
    key(&mut h, Key::Enter);
    type_in(&mut h, "Second");
    key(&mut h, Key::Backspace);
    assert_eq!(plain(&h), "Hello world!\nSecon");
    assert_eq!(sel(&h), Selection::caret(18));
    // A selection and a key: the selection takes it.
    key_with(&mut h, Modifiers::COMMAND, Key::A);
    assert_eq!(sel(&h), Selection { anchor: 0, head: 18 });
    key_with(&mut h, Modifiers::COMMAND, Key::B);
    assert!(h.state().session.compose.as_ref().unwrap().body.text().paragraphs().iter().all(|p| p.runs.iter().all(|r| r.style.bold)));
    // Undo inside the message, step by step; redo.
    key_with(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(!h.state().session.compose.as_ref().unwrap().body.text().paragraphs()[0].runs[0].style.bold);
    key_with(&mut h, Modifiers::COMMAND, Key::Y);
    assert!(h.state().session.compose.as_ref().unwrap().body.text().paragraphs()[0].runs[0].style.bold);
    key_with(&mut h, Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
    assert!(h.state().session.compose.as_ref().unwrap().body.text().paragraphs()[0].runs[0].style.bold, "nothing more to redo");
    // Tab is a tab character in the text, not a way out of it.
    key_with(&mut h, Modifiers::NONE, Key::End);
    key(&mut h, Key::Tab);
    assert!(plain(&h).ends_with("Secon\t") && editor::has_focus(&h.ctx));
}

#[test]
fn input_methods_type_what_they_commit() {
    let mut h = writing();
    h.event(Event::Ime(egui::ImeEvent::Preedit { text: "ni".into(), active_range_chars: None }));
    h.run_steps(2);
    assert_eq!(plain(&h), "", "what is being composed is not typed yet");
    h.event(Event::Ime(egui::ImeEvent::Commit("你".into())));
    h.run_steps(2);
    assert_eq!(plain(&h), "你");
    // The window tells the input method where the caret is.
    assert!(h.output().platform_output.ime.is_some());
}

#[test]
fn enter_goes_on_with_a_list_and_ends_it_on_an_empty_item() {
    let mut h = writing();
    type_in(&mut h, "Shopping:");
    key(&mut h, Key::Enter);
    h.state_mut().run("format.bullets", json!({})).unwrap();
    h.run_steps(2);
    // The ribbon button left the keyboard in the text.
    assert!(editor::has_focus(&h.ctx));
    type_in(&mut h, "bread");
    key(&mut h, Key::Enter);
    type_in(&mut h, "milk");
    key(&mut h, Key::Enter);
    assert_eq!(kinds(&h), [ListKind::None, ListKind::Bullet, ListKind::Bullet, ListKind::Bullet]);
    key(&mut h, Key::Enter);
    assert_eq!(kinds(&h), [ListKind::None, ListKind::Bullet, ListKind::Bullet, ListKind::None]);
    assert_eq!(plain(&h), "Shopping:\nbread\nmilk\n");
    // Shift+Tab takes an item out of its list; Backspace at an item's start does too.
    key(&mut h, Key::ArrowUp);
    key_with(&mut h, Modifiers::SHIFT, Key::Tab);
    assert_eq!(kinds(&h), [ListKind::None, ListKind::Bullet, ListKind::None, ListKind::None]);
    key(&mut h, Key::ArrowUp);
    key(&mut h, Key::Home);
    key(&mut h, Key::Backspace);
    assert_eq!(kinds(&h), [ListKind::None; 4]);
    assert_eq!(plain(&h), "Shopping:\nbread\nmilk\n");
}

#[test]
fn moving_about_and_selecting_with_the_keys() {
    let mut h = writing();
    type_in(&mut h, "one two three");
    key(&mut h, Key::Enter);
    type_in(&mut h, "four");
    key(&mut h, Key::Home);
    assert_eq!(sel(&h), Selection::caret(14));
    key(&mut h, Key::ArrowUp);
    assert_eq!(sel(&h), Selection::caret(0));
    key_with(&mut h, Modifiers::CTRL, Key::ArrowRight);
    assert_eq!(sel(&h), Selection::caret(3));
    key_with(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::ArrowRight);
    assert_eq!(sel(&h), Selection { anchor: 3, head: 7 });
    key_with(&mut h, Modifiers::SHIFT, Key::End);
    assert_eq!(sel(&h), Selection { anchor: 3, head: 13 });
    // An arrow without Shift lets go of the selection at its end.
    key(&mut h, Key::ArrowRight);
    assert_eq!(sel(&h), Selection::caret(13));
    key(&mut h, Key::ArrowRight);
    assert_eq!(sel(&h), Selection::caret(14), "over the paragraph's end");
    key(&mut h, Key::ArrowLeft);
    key_with(&mut h, Modifiers::CTRL, Key::ArrowLeft);
    assert_eq!(sel(&h), Selection::caret(8));
    key_with(&mut h, Modifiers::CTRL, Key::Backspace);
    assert_eq!(plain(&h), "one three\nfour");
    key_with(&mut h, Modifiers::CTRL, Key::End);
    assert_eq!(sel(&h), Selection::caret(14));
    key(&mut h, Key::PageUp);
    assert_eq!(sel(&h), Selection::caret(0));
    key(&mut h, Key::PageDown);
    assert_eq!(sel(&h), Selection::caret(14));
    key(&mut h, Key::Delete);
    key_with(&mut h, Modifiers::CTRL, Key::Home);
    key(&mut h, Key::Delete);
    assert_eq!(plain(&h), "ne three\nfour");
}

#[test]
fn the_clipboard_copies_cuts_and_pastes_through_commands() {
    let mut h = writing();
    type_in(&mut h, "alpha beta");
    key_with(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::ArrowLeft);
    h.event(Event::Copy);
    h.step();
    let copied = h.output().platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(t) if t == "beta"));
    assert!(copied, "{:?}", h.output().platform_output.commands);
    h.run_steps(1);
    h.event(Event::Cut);
    h.run_steps(2);
    assert_eq!(plain(&h), "alpha ");
    h.event(Event::Paste("gamma\ndelta".into()));
    h.run_steps(2);
    assert_eq!(plain(&h), "alpha gamma\ndelta");
    // The paste is one step back.
    key_with(&mut h, Modifiers::COMMAND, Key::Z);
    assert_eq!(plain(&h), "alpha ");
}

/// Ctrl+U and Ctrl+Z are the body's only while it has the keyboard: elsewhere they mark the
/// message unread and undo what was done to the mailbox.
#[test]
fn the_formatting_keys_are_the_bodys_only_while_it_has_the_keyboard() {
    let mut a = app();
    let id = a.session.rows().first().map(|r| r.id).unwrap();
    a.run("mail.select", json!({"id": id})).unwrap();
    a.run("mail.markRead", json!({"ids": [id]})).unwrap();
    a.run("mail.flag", json!({"ids": [id]})).unwrap();
    let flagged = |h: &Window| h.state().session.mailbox.message(id).unwrap().flag == mailcraft_model::FlagStatus::Flagged;
    let read = |h: &Window| h.state().session.mailbox.message(id).unwrap().read;
    a.run("mail.reply", json!({"id": id})).unwrap();
    a.body_focus_wanted = true;
    let mut h = window(a);
    h.run_steps(2);
    assert!(editor::has_focus(&h.ctx) && flagged(&h) && read(&h));
    let typed = plain(&h);
    type_in(&mut h, "x");
    key_with(&mut h, Modifiers::COMMAND, Key::U);
    key_with(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(read(&h) && flagged(&h), "the mailbox is not touched from the body");
    assert_eq!(plain(&h), typed, "the typing was undone");
    // Out of the body: the same keys are the mailbox's.
    h.ctx.memory_mut(|m| m.surrender_focus(editor::id()));
    h.run_steps(2);
    assert!(!editor::has_focus(&h.ctx));
    key_with(&mut h, Modifiers::COMMAND, Key::Z);
    assert!(!flagged(&h), "Ctrl+Z undid the flag");
    key_with(&mut h, Modifiers::COMMAND, Key::U);
    assert!(!read(&h), "Ctrl+U marked the message unread");
    assert_eq!(plain(&h), typed);
}

#[test]
fn the_pointer_puts_the_caret_and_selects_words() {
    let mut h = writing();
    type_in(&mut h, "first second");
    h.run_steps(1);
    let r = h.state().hit.rect("composeBody", 0).unwrap();
    let at = r.min + egui::vec2(1.0, 8.0);
    h.event(Event::PointerMoved(at));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.step();
    }
    h.run_steps(1);
    assert_eq!(sel(&h), Selection::caret(0));
    // A second click soon after: the word (the harness's frames are a quarter of a second apart,
    // so "soon" is a second here); a third: the paragraph.
    h.ctx.options_mut(|o| o.input_options.max_double_click_delay = 1.0);
    let click = |h: &mut Window| {
        for pressed in [true, false] {
            h.event(Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
            h.step();
        }
    };
    click(&mut h);
    assert_eq!(sel(&h), Selection { anchor: 0, head: 5 });
    click(&mut h);
    assert_eq!(sel(&h), Selection { anchor: 0, head: 12 });
}

#[test]
fn a_plain_text_message_has_no_formatting_and_ctrl_k_asks_for_a_link() {
    let mut h = writing();
    type_in(&mut h, "see");
    key_with(&mut h, Modifiers::CTRL | Modifiers::SHIFT, Key::ArrowLeft);
    key_with(&mut h, Modifiers::COMMAND, Key::K);
    assert_eq!(h.state().dialog.as_ref().map(|d| d.name()), Some("link"));
    h.state_mut().dialog = None;
    assert_eq!(crate::dialogs::link_address("someone@example.org"), "mailto:someone@example.org");
    assert_eq!(crate::dialogs::link_address(" www.example.org "), "https://www.example.org");
    assert_eq!(crate::dialogs::link_address("tel:+15550100"), "tel:+15550100");
    h.state_mut().run("compose.link", json!({"url": "https://www.example.org"})).unwrap();
    assert!(crate::ribbon::look(h.state().session.compose.as_ref().unwrap()).link);
    h.state_mut().run("compose.formatPlain", json!({})).unwrap();
    h.run_steps(2);
    assert!(!h.state().enabled("format.bold") && !h.state().enabled("compose.link"));
    key(&mut h, Key::End);
    key_with(&mut h, Modifiers::COMMAND, Key::B);
    key_with(&mut h, Modifiers::COMMAND, Key::K);
    assert!(h.state().dialog.is_none());
    type_in(&mut h, "!");
    let body = h.state().session.compose.as_ref().unwrap().body.clone();
    assert!(body.text().paragraphs()[0].runs.iter().all(|r| !r.style.bold));
    assert_eq!(body.plain(), "see!");
}

/// A long message costs a frame what is on screen: a key lays out the paragraph it changed,
/// not the message.
#[test]
fn a_long_message_lays_out_only_what_changes() {
    let mut a = app();
    a.execute("mail.new", json!({})).unwrap();
    let long: Vec<String> =
        (0..2000).map(|i| format!("Paragraph {i} of a long message, with enough words in it to wrap now and then on a narrow window.")).collect();
    a.execute("compose.set", json!({"body": long.join("\n")})).unwrap();
    a.execute("compose.select", json!({"from": 0})).unwrap();
    a.body_focus_wanted = true;
    let mut h = window(a);
    h.run_steps(2);
    let before = editor::laid(&h.ctx);
    assert!(before >= 2000);
    h.run_steps(5);
    assert_eq!(editor::laid(&h.ctx), before, "nothing is laid out again while nothing changes");
    type_in(&mut h, "x");
    assert!(editor::laid(&h.ctx) - before <= 2, "{}", editor::laid(&h.ctx) - before);
    key(&mut h, Key::Enter);
    assert!(editor::laid(&h.ctx) - before <= 5);
    key_with(&mut h, Modifiers::CTRL, Key::End);
    assert!(plain(&h).starts_with("x\nParagraph 0"));
    let t0 = std::time::Instant::now();
    h.run_steps(10);
    // Generous: a debug build on whatever machine runs the tests.
    assert!(t0.elapsed().as_secs() < 20, "{:?}", t0.elapsed());
}

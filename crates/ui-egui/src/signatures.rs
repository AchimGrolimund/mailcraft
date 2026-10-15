//! File › Signatures (R4.5.3): the signatures on the left (New, Rename, Delete), the one chosen
//! edited on the right with the composer's own editor and formatting, its pictures (Insert
//! Picture, a dropped file, a pasted picture; a picture's width and description; the layout) and
//! the size they add to every message; below, each account's signature for new messages and for
//! replies and forwards.
//!
//! Choosing a signature opens it (`signature.open`); every change is a command, and the page only
//! collects input. A signature with changes is saved when another is chosen and when the page is
//! left.

use egui::{Ui, vec2};
use mailcraft_model::signature::{HEAVY_BYTES, Layout, MAX_PICTURES_BYTES, MAX_WIDTH};
use serde_json::json;

use crate::MailApp;
use crate::compose::editor::{self, Which};
use crate::theme::{Tokens, regular, semibold};

pub(crate) fn page(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    ui.label(egui::RichText::new(tl!("Signatures")).font(semibold(22.0)).color(t.text));
    ui.label(
        egui::RichText::new(tl!(
            "Signatures are kept on this computer. They can't be kept in step with Outlook: on another device set them up there, or paste one from Outlook on the web (Settings › Signatures)."
        ))
        .font(regular(11.5))
        .color(t.text_dim),
    );
    ui.add_space(6.0);
    let names: Vec<String> = app.session.mailbox.signatures.iter().map(|s| s.name.clone()).collect();
    if app.session.signature_edit.is_none()
        && let Some(first) = names.first()
    {
        let _ = app.execute("signature.open", json!({"name": first}));
    }
    let open = app.session.signature_edit.as_ref().map(|e| e.signature.name.clone());
    ui.horizontal_top(|ui| {
        ui.vertical(|ui| {
            ui.set_width(180.0);
            list(app, ui, t, &names, open.as_deref());
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width((ui.available_width()).clamp(320.0, 560.0));
            if open.is_some() {
                editing(app, ui, t);
            } else {
                ui.label(egui::RichText::new(tl!("Make a signature with New.")).color(t.text_dim));
            }
        });
    });
    ui.add_space(10.0);
    ui.label(egui::RichText::new(tl!("Choose default signatures")).font(semibold(14.0)).color(t.accent_text));
    defaults(app, ui, &names);
}

/// Open `name`, keeping what was changed in the one open now.
fn choose(app: &mut MailApp, name: &str) {
    if app.session.signature_edit.as_ref().is_some_and(|e| e.dirty) && app.run("signature.save", json!({})).is_err() {
        return;
    }
    let _ = app.run("signature.open", json!({"name": name}));
}

fn list(app: &mut MailApp, ui: &mut Ui, t: &Tokens, names: &[String], open: Option<&str>) {
    egui::Frame::NONE.fill(t.input).stroke(egui::Stroke::new(1.0, t.border)).inner_margin(4.0).show(ui, |ui| {
        ui.set_min_size(vec2(172.0, 160.0));
        for (i, name) in names.iter().enumerate() {
            let chosen = open.is_some_and(|o| o.eq_ignore_ascii_case(name));
            let r = ui.add_sized(vec2(172.0, 22.0), egui::Button::selectable(chosen, name.as_str()));
            app.hit.add("signature", i as u64, r.rect);
            if r.clicked() && !chosen {
                choose(app, name);
            }
        }
    });
    let renaming_id = egui::Id::new("mailcraft-signature-rename");
    let mut renaming: Option<String> = ui.data(|d| d.get_temp(renaming_id));
    if let (Some(text), Some(from)) = (renaming.as_mut(), open) {
        let r = ui.add(egui::TextEdit::singleline(text).desired_width(172.0).char_limit(100));
        r.request_focus();
        if r.lost_focus() {
            if ui.input(|i| i.key_pressed(egui::Key::Enter)) && !text.trim().is_empty() {
                let _ = app.run("signature.rename", json!({"name": from, "to": text.trim()}));
            }
            renaming = None;
        }
    }
    ui.horizontal(|ui| {
        if ui.button(tl!("New")).clicked() {
            // "Signature", "Signature 2"… whichever is free.
            let name = (1..100)
                .map(|n| if n == 1 { "Signature".to_string() } else { format!("Signature {n}") })
                .find(|n| app.session.mailbox.signature(n).is_none())
                .unwrap_or_else(|| "Signature".to_string());
            let me = app.session.mailbox.default_account().map(|a| a.display_name.clone()).unwrap_or_default();
            if app.run("signature.set", json!({"name": name, "text": me, "default": names.is_empty()})).is_ok() {
                choose(app, &name);
            }
        }
        if ui.add_enabled(open.is_some(), egui::Button::new(tl!("Rename"))).clicked() {
            renaming = open.map(str::to_string);
        }
        if ui.add_enabled(open.is_some(), egui::Button::new(tl!("Delete"))).clicked()
            && let Some(name) = open
        {
            let _ = app.run("signature.remove", json!({"name": name}));
        }
    });
    ui.data_mut(|d| match renaming {
        Some(text) => {
            d.insert_temp(renaming_id, text);
        }
        None => d.remove::<String>(renaming_id),
    });
}

/// The open signature: its formatting bar, the editor, the selected picture's width and
/// description, and the size its pictures add.
fn editing(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let look = crate::ribbon::look_now(app);
    ui.horizontal_wrapped(|ui| {
        ui.menu_button(tl!("Font"), |ui| crate::ribbon::font_menu(ui, app));
        ui.menu_button(tl!("Size"), |ui| crate::ribbon::size_menu(ui, app));
        ui.menu_button(tl!("Colour"), |ui| crate::ribbon::color_menu(ui, app));
        for (label, on, command) in [("B", look.bold, "format.bold"), ("I", look.italic, "format.italic"), ("U", look.underline, "format.underline")]
        {
            if ui.add(egui::Button::selectable(on, egui::RichText::new(label).strong())).on_hover_text(tl!(command_label(command))).clicked() {
                let _ = app.run(command, json!({}));
            }
        }
        if ui.add(egui::Button::selectable(look.link, tl!("Link"))).clicked() {
            let _ = app.run("compose.link", json!({}));
        }
        if ui.button(tl!("Insert Picture")).clicked() {
            let _ = app.run("signature.picture.add", json!({}));
        }
        // (A picture alone on the clipboard makes no paste in the editor: egui pastes text.)
        if app.services.clipboard.is_some() && ui.button(tl!("Paste Picture")).clicked() {
            match app.services.clipboard.as_ref().and_then(|b| b.image()) {
                Some(png) => {
                    let _ = app.run("signature.picture.add", json!({"data": crate::files::base64_lite::encode(&png), "file": "picture.png"}));
                }
                None => app.status(tl!("There is no picture on the clipboard.")),
            }
        }
        let layout = app.session.signature_edit.as_ref().map_or(Layout::Inline, |e| e.signature.layout);
        let label = |l: Layout| match l {
            Layout::Inline => tl!("In line with text"),
            Layout::PictureLeft => tl!("Picture beside text"),
            Layout::PictureAbove => tl!("Picture above text"),
        };
        egui::ComboBox::from_id_salt("signature_layout").selected_text(label(layout)).show_ui(ui, |ui| {
            for (l, code) in [(Layout::Inline, "inline"), (Layout::PictureLeft, "pictureLeft"), (Layout::PictureAbove, "pictureAbove")] {
                if ui.selectable_label(l == layout, label(l)).clicked() {
                    let _ = app.run("signature.layout", json!({"value": code}));
                }
            }
        });
    });
    egui::Frame::NONE.fill(t.surface).stroke(egui::Stroke::new(1.0, t.border)).inner_margin(10.0).show(ui, |ui| {
        ui.set_min_height(150.0);
        editor::show_doc(app, ui, Which::Signature, 130.0, false);
    });
    picture(app, ui);
    let Some(e) = app.session.signature_edit.as_ref() else { return };
    // What its pictures add to every message, counting those its text still holds.
    let held = e.body.text().pictures();
    let bytes: usize = e.signature.pictures().iter().filter(|p| held.contains(&p.id)).map(|p| p.data.len()).sum();
    let dirty = e.dirty;
    ui.horizontal_wrapped(|ui| {
        let kb = |n: usize| n.div_ceil(1024).to_string();
        let total = crate::i18n::fmt(tl!("Pictures: {n} KB of {max} KB"), &[("n", &kb(bytes)), ("max", &kb(MAX_PICTURES_BYTES))]);
        ui.label(egui::RichText::new(total).color(t.text_dim));
        if bytes > HEAVY_BYTES {
            ui.label(egui::RichText::new(tl!("Every message with this signature gets heavier by as much.")).color(t.text_dim));
        }
    });
    ui.horizontal(|ui| {
        if ui.add_enabled(dirty, egui::Button::new(tl!("Save"))).clicked() {
            let _ = app.run("signature.save", json!({}));
        }
        if dirty {
            ui.label(egui::RichText::new(tl!("Changed: saved when you choose another signature or leave this page.")).color(t.text_dim));
        }
    });
}

fn command_label(command: &str) -> &'static str {
    match command {
        "format.bold" => "Bold",
        "format.italic" => "Italic",
        _ => "Underline",
    }
}

/// The picture that is the selection, alone: its width and description, and taking it out.
fn picture(app: &mut MailApp, ui: &mut Ui) {
    let Some(e) = app.session.signature_edit.as_ref() else { return };
    let sel = e.body.selection();
    let Some(id) = e
        .body
        .text()
        .slice(sel.start(), sel.end())
        .ok()
        .map(|part| part.pictures())
        .filter(|p| p.len() == 1 && sel.end() == sel.start() + 1)
        .and_then(|p| p.first().copied())
    else {
        return;
    };
    let Some(p) = e.signature.picture(id) else { return };
    let frame = crate::reading::images::frame(ui.ctx());
    let own = crate::reading::images::Pictures::of(ui.ctx()).lock().ok().and_then(|mut c| c.size(&p.data, frame)).map(|[w, _]| w);
    let mut width = p.width.or_else(|| own.and_then(|w| u16::try_from(w).ok())).unwrap_or(MAX_WIDTH).min(MAX_WIDTH);
    let mut alt = p.alt.clone();
    ui.horizontal(|ui| {
        ui.label(tl!("Width"));
        if ui.add(egui::DragValue::new(&mut width).range(8..=MAX_WIDTH).suffix(" px")).changed() {
            let _ = app.run("signature.picture.set", json!({"id": id, "width": width}));
        }
        ui.label(tl!("Description"));
        if ui.add(egui::TextEdit::singleline(&mut alt).desired_width(200.0).char_limit(200)).changed() {
            let _ = app.execute("signature.picture.set", json!({"id": id, "alt": alt}));
        }
        if ui.button(tl!("Remove Picture")).clicked() {
            let _ = app.run("signature.picture.remove", json!({"id": id}));
        }
    });
}

/// Each account's signature for new messages, and for replies and forwards.
fn defaults(app: &mut MailApp, ui: &mut Ui, names: &[String]) {
    let accounts: Vec<(u64, String, String, String)> =
        app.session.mailbox.accounts.iter().map(|a| (a.id, a.email.clone(), a.signature.clone(), a.reply_signature().to_string())).collect();
    if accounts.is_empty() {
        ui.label(tl!("Add an account first"));
    }
    egui::Grid::new("signature_defaults").num_columns(3).spacing(vec2(12.0, 6.0)).show(ui, |ui| {
        for (id, email, new, reply) in accounts {
            ui.label(email);
            for (key, now, label) in [("new", new, tl!("New messages")), ("reply", reply, tl!("Replies and forwards"))] {
                ui.horizontal(|ui| {
                    ui.label(label);
                    let shown = if now.is_empty() { tl!("(none)").to_string() } else { now.clone() };
                    egui::ComboBox::from_id_salt(("signature_default", id, key)).selected_text(shown).width(150.0).show_ui(ui, |ui| {
                        for name in std::iter::once("").chain(names.iter().map(String::as_str)) {
                            let text = if name.is_empty() { tl!("(none)") } else { name };
                            if ui.selectable_label(name.eq_ignore_ascii_case(&now), text).clicked() {
                                let _ = app.run("signature.defaults", json!({"account": id, key: name}));
                            }
                        }
                    });
                });
            }
            ui.end_row();
        }
    });
}

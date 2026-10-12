//! Mail: the folder pane, the message list and the reading pane (or the message being written).

use egui::{Align2, Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::Pane;
use mailcraft_engine::list::Row;
use mailcraft_model::{FlagStatus, FolderKind, Id, Importance, Message};
use serde_json::json;

use crate::theme::{Tokens, paint_clipped, regular, semibold};
use crate::{MailApp, icons};

/// What a drag out of the message list carries: the messages to file in the folder they are
/// let go over (`nav::folder_row` takes them).
pub(crate) struct MessageDrag(pub Vec<Id>);

/// A group heading in the list (Today, Yesterday…): a box with a line round it.
const GROUP_H: f32 = 25.0;
/// The list's text: sender, subject, preview and date are all 9 pt.
const LIST_TEXT: f32 = 12.0;
/// The list's header (the folder's name, filter and sort).
const HEADER_H: f32 = 47.0;
/// What is in a row keeps this far from the list's left edge: the group headings' boxes and the
/// bar an unread message wears start here.
const ROW_INSET: f32 = 2.0;
/// How wide the read/unread switch at a row's left edge is (the text starts just past it).
const READ_STRIP: f32 = 12.0;

/// While messages are being dragged: a small label by the pointer saying how many.
fn drag_ghost(ui: &Ui) {
    let Some(drag) = egui::DragAndDrop::payload::<MessageDrag>(ui.ctx()) else { return };
    let Some(pos) = ui.ctx().pointer_interact_pos() else { return };
    let t = Tokens::get(ui.ctx());
    ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    let text = if drag.0.len() == 1 { tl!("1 message").to_string() } else { format!("{} {}", drag.0.len(), tl!("messages")) };
    egui::Area::new(egui::Id::new("message_drag_ghost")).order(egui::Order::Tooltip).fixed_pos(pos + vec2(14.0, 16.0)).interactable(false).show(
        ui.ctx(),
        |ui| {
            egui::Frame::NONE
                .fill(t.menu)
                .stroke(Stroke::new(1.0, t.border_strong))
                .corner_radius(4.0)
                .inner_margin(egui::Margin::symmetric(8, 4))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(text).font(regular(12.0)).color(t.text));
                });
        },
    );
}

pub fn show(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    drag_ghost(ui);
    if app.session.view.folder_pane {
        crate::nav::folder_pane(app, ui);
    } else {
        // Hidden: a narrow strip stays, with the button that brings the pane back.
        crate::nav::folder_pane_strip(app, ui);
    }
    let composing = app.session.compose.is_some();
    let pane = app.session.view.reading_pane;
    // The list takes the whole width when the reading pane is off, unless a message is being written.
    if pane == Pane::Off && !composing {
        egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.surface)).show(ui, |ui| list(app, ui));
        return;
    }
    let frame = egui::Frame::NONE.fill(t.surface).stroke(Stroke::new(1.0, t.border));
    if pane == Pane::Bottom {
        let h = (ui.available_height() * 0.42).clamp(140.0, 520.0);
        egui::Panel::top("message_list")
            .default_size(h)
            .size_range(120.0..=(ui.available_height() - 140.0).max(160.0))
            .resizable(true)
            .frame(frame)
            .show(ui, |ui| list(app, ui));
    } else {
        let max = (ui.available_width() - 260.0).max(260.0);
        let shown = egui::Panel::left("message_list")
            .default_size(app.ui.list_width.min(max))
            .size_range(240.0..=max)
            .resizable(true)
            .frame(frame)
            .show(ui, |ui| list(app, ui));
        app.ui.list_width = shown.response.rect.width();
    }
    egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.surface)).show(ui, |ui| {
        if composing {
            crate::compose::show(app, ui);
        } else {
            crate::reading::show(app, ui);
        }
    });
}

/// A row and the line under it: 72 with one line of preview, the lines 18 apart.
fn row_height(preview_lines: u8) -> f32 {
    56.0 + f32::from(preview_lines.min(3)) * 16.0
}

/// The header above the list: the folder's name, sort and filter.
fn list_header(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), HEADER_H), Sense::hover());
    let searching = !app.session.view.search.trim().is_empty();
    let title = if searching {
        tl!("Search Results").to_string()
    } else {
        app.session.mailbox.folder(app.session.view.folder).map(|f| f.name.clone()).unwrap_or_default()
    };
    // Sort and filter menus on the right.
    let sort = app.session.view.sort;
    let label = format!("{} {}", tl!(sort.label()), if app.session.view.sort_descending { "↓" } else { "↑" });
    let w = ui.ctx().fonts_mut(|f| f.layout_no_wrap(label.clone(), regular(LIST_TEXT), t.text).size().x) + 14.0;
    let sr = Rect::from_min_max(pos2(r.max.x - w - 8.0, r.center().y - 11.0), pos2(r.max.x - 8.0, r.center().y + 11.0));
    let sresp = ui.interact(sr, ui.id().with("list_sort"), Sense::click());
    if sresp.hovered() {
        ui.painter().rect_filled(sr, 4.0, t.hover);
    }
    ui.painter().text(sr.center(), Align2::CENTER_CENTER, label, regular(LIST_TEXT), t.text_dim);
    egui::Popup::menu(&sresp).show(|ui| {
        ui.set_min_width(170.0);
        for s in mailcraft_engine::SortBy::ALL {
            crate::widgets::mi_check(ui, app, s.label(), s == sort, "view.sort", json!({"by": s}));
        }
        ui.separator();
        crate::widgets::mi(ui, app, "Reverse Sort", "view.reverseSort", json!({}));
        let conv = app.session.view.conversations;
        crate::widgets::mi_check(ui, app, "Show as Conversations", conv, "view.conversations", json!({}));
    });
    let filter = app.session.view.filter;
    let flabel = if filter == mailcraft_engine::Filter::All { tl!("All").to_string() } else { tl!(filter.label()).to_string() };
    let fw = ui.ctx().fonts_mut(|f| f.layout_no_wrap(flabel.clone(), regular(LIST_TEXT), t.text).size().x) + 26.0;
    let fr = Rect::from_min_max(pos2(sr.min.x - fw - 4.0, sr.min.y), pos2(sr.min.x - 4.0, sr.max.y));
    let fresp = ui.interact(fr, ui.id().with("list_filter"), Sense::click());
    if fresp.hovered() || filter != mailcraft_engine::Filter::All {
        ui.painter().rect_filled(fr, 4.0, if filter != mailcraft_engine::Filter::All { t.checked } else { t.hover });
    }
    icons::paint(ui.painter(), Rect::from_center_size(pos2(fr.min.x + 10.0, fr.center().y), vec2(12.0, 12.0)), "filter", t.text_dim, t.accent);
    ui.painter().text(pos2(fr.min.x + 19.0, fr.center().y), Align2::LEFT_CENTER, flabel, regular(LIST_TEXT), t.text_dim);
    egui::Popup::menu(&fresp).show(|ui| {
        ui.set_min_width(170.0);
        for f in mailcraft_engine::Filter::ALL {
            crate::widgets::mi_check(ui, app, f.label(), f == filter, "view.filter", json!({"value": f}));
        }
    });
    paint_clipped(ui.painter(), pos2(r.min.x + 12.0, r.center().y), (fr.min.x - r.min.x - 20.0).max(0.0), &title, semibold(15.0), t.text);
}

fn list(app: &mut MailApp, ui: &mut Ui) {
    let t = Tokens::get(ui.ctx());
    ui.spacing_mut().item_spacing = vec2(0.0, 0.0);
    list_header(app, ui);
    let rows = app.session.rows();
    if rows.is_empty() {
        let r = ui.available_rect_before_wrap();
        let msg = if !app.session.view.search.trim().is_empty() {
            tl!("Nothing matches the search.")
        } else if app.session.view.filter != mailcraft_engine::Filter::All {
            tl!("Nothing matches this filter.")
        } else {
            tl!("Nothing here yet.")
        };
        ui.painter().text(pos2(r.center().x, r.min.y + 60.0), Align2::CENTER_CENTER, msg, regular(12.5), t.text_dim);
        return;
    }
    let row_h = row_height(app.session.prefs.preview_lines);
    // Where each row starts: rows are the same height, group headings add theirs.
    let mut tops = Vec::with_capacity(rows.len());
    let mut y = 0.0f32;
    for row in rows.iter() {
        if row.group.is_some() {
            y += GROUP_H;
        }
        tops.push(y);
        y += row_h;
    }
    let total = y;
    let reveal = std::mem::take(&mut app.reveal_selection);
    let focus = app.session.sel.focus();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show_viewport(ui, |ui, viewport| {
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), total), Sense::hover());
        let origin = area.min;
        let width = area.width();
        // Only the rows in view are laid out.
        let first = tops.partition_point(|top| top + row_h < viewport.min.y).saturating_sub(1);
        for (i, row) in rows.iter().enumerate().skip(first) {
            let Some(top) = tops.get(i).copied() else { break };
            if top - GROUP_H > viewport.max.y {
                break;
            }
            if let Some(group) = &row.group {
                // A box a step lighter than the list (a step darker on a light one), with a line round it.
                let gr = Rect::from_min_max(pos2(origin.x + ROW_INSET, origin.y + top - GROUP_H), pos2(origin.x + width, origin.y + top));
                let fill = t.surface.lerp_to_gamma(t.selection, if t.dark { 1.0 } else { 0.45 });
                let line = Stroke::new(1.0, t.border_strong);
                ui.painter().rect_filled(gr, 0.0, fill);
                // The line under the row above is this box's top edge, except for the first box.
                let (left, bottom) = (gr.min.x + 0.5, gr.max.y - 0.5);
                let top = if i == 0 { gr.min.y + 0.5 } else { gr.min.y };
                ui.painter().line_segment([pos2(left, top), pos2(left, gr.max.y)], line);
                ui.painter().line_segment([pos2(gr.min.x, bottom), pos2(gr.max.x, bottom)], line);
                if i == 0 {
                    ui.painter().line_segment([pos2(gr.min.x, top), pos2(gr.max.x, top)], line);
                }
                ui.painter().text(pos2(gr.min.x + 10.0, gr.center().y), Align2::LEFT_CENTER, tl!(group), semibold(LIST_TEXT), t.text);
            }
            let rect = Rect::from_min_size(pos2(origin.x, origin.y + top), vec2(width, row_h));
            if reveal && Some(row.id) == focus {
                ui.scroll_to_rect(rect, None);
            }
            message_row(app, ui, row, rect);
        }
    });
}

fn message_row(app: &mut MailApp, ui: &mut Ui, row: &Row, rect: Rect) {
    let t = Tokens::get(ui.ctx());
    // The shared pointer, not a copy of the message.
    let Some(m) = app.session.mailbox.messages.get(&row.id).cloned() else { return };
    let resp = ui.interact(rect, ui.id().with(("msg", row.id)), Sense::click_and_drag());
    app.hit.add("message", row.id, rect);
    let unread = row.unread > 0;
    // The strip down the left edge is a switch between read and unread. It sits on top of the
    // row, so a click there changes the mark and leaves the selection alone.
    let strip = Rect::from_min_max(rect.min, pos2(rect.min.x + READ_STRIP, rect.max.y));
    let read_resp = ui
        .interact(strip, ui.id().with(("msg_read", row.id)), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(if unread { tl!("Mark as read") } else { tl!("Mark as unread") });
    app.hit.add("messageRead", row.id, strip);
    // Quick actions on the row under the pointer, at the right of its subject line: flag and
    // delete. Like the read switch they sit on top of the row and leave the selection alone.
    let over = ui.rect_contains_pointer(rect) && egui::DragAndDrop::payload::<MessageDrag>(ui.ctx()).is_none();
    let delete_rect = Rect::from_center_size(pos2(rect.max.x - 19.0, rect.min.y + 35.0), vec2(20.0, 20.0));
    let flag_rect = delete_rect.translate(vec2(-22.0, 0.0));
    let flagged = m.flag == FlagStatus::Flagged;
    let quick = over.then(|| {
        let delete = ui.interact(delete_rect, ui.id().with(("msg_delete", row.id)), Sense::click()).on_hover_text(tl!("Delete"));
        let flag = ui.interact(flag_rect, ui.id().with(("msg_flag", row.id)), Sense::click()).on_hover_text(if flagged {
            tl!("Clear flag")
        } else {
            tl!("Flag for follow-up")
        });
        app.hit.add("messageDelete", row.id, delete_rect);
        app.hit.add("messageFlag", row.id, flag_rect);
        (delete, flag)
    });
    let selected = app.session.sel.messages.contains(&row.id);
    let focus = app.session.sel.focus() == Some(row.id);
    let p = ui.painter();
    // The row is what is above the line under it; its content starts a little in from the list's edge.
    let body = Rect::from_min_max(pos2(rect.min.x + ROW_INSET, rect.min.y), pos2(rect.max.x, rect.max.y - 1.0));
    if selected {
        p.rect_filled(body, 0.0, if focus { t.selection } else { t.selection.linear_multiply(0.7) });
        if focus {
            // The message the keys act on has a line round it.
            p.rect_stroke(body, 0.0, Stroke::new(1.0, t.group_label), egui::StrokeKind::Inside);
        }
    } else if resp.hovered() || read_resp.hovered() || quick.is_some() {
        p.rect_filled(body, 0.0, t.hover.linear_multiply(0.6));
    }
    // Unread wears the bar; under the pointer it widens, and a read message shows a faint one
    // so the switch can be found.
    let bar = Rect::from_min_max(body.min, pos2(body.min.x + if read_resp.hovered() { 6.0 } else { 4.0 }, body.max.y));
    if unread {
        p.rect_filled(bar, 0.0, t.accent);
    } else if read_resp.hovered() {
        p.rect_filled(bar, 0.0, t.accent.linear_multiply(0.4));
    }
    let rule = if t.dark { t.border_strong } else { t.border };
    p.line_segment([pos2(body.min.x, rect.max.y - 0.5), pos2(rect.max.x, rect.max.y - 0.5)], Stroke::new(1.0, rule));

    // Sent, drafts and outbox show who it went to rather than who wrote it.
    let outgoing = app.session.mailbox.folder(m.folder).is_some_and(|f| matches!(f.kind, FolderKind::Sent | FolderKind::Drafts | FolderKind::Outbox));
    let who = if outgoing {
        let names: Vec<&str> = m.to.iter().map(|a| a.display()).collect();
        if names.is_empty() { tl!("(no recipients)").to_string() } else { names.join("; ") }
    } else {
        m.from.display().to_string()
    };
    let left = rect.min.x + 14.0;
    let right = rect.max.x - 10.0;
    let line1 = rect.min.y + 17.0;
    let line2 = rect.min.y + 35.0;

    // Right side of line one: the date, then the markers.
    let date =
        mailcraft_time::list_label(mailcraft_engine::list::when(&m), app.session.now(), app.session.offset_at(m.date), app.session.prefs.twelve_hour);
    let date_w = p.text(pos2(right, line1), Align2::RIGHT_CENTER, &date, regular(LIST_TEXT), if unread { t.accent_text } else { t.text_dim }).width();
    let mut x = right - date_w - 6.0;
    let mut marker = |icon: &str, color: egui::Color32| {
        icons::paint(p, Rect::from_center_size(pos2(x - 6.0, line1), vec2(12.0, 12.0)), icon, color, t.accent);
        x -= 15.0;
    };
    match m.flag {
        FlagStatus::Flagged => marker("flag", t.icon),
        FlagStatus::Complete => marker("complete", t.icon),
        FlagStatus::None => {}
    }
    // A meeting that came by mail wears its mark; its calendar part is not one of its files.
    let (meeting, files) = crate::reading::list_marks(&m);
    if let Some(meeting) = meeting {
        marker(meeting, t.icon);
    }
    if files {
        marker("attach", t.text_dim);
    }
    match m.importance {
        Importance::High => marker("highImportance", t.icon),
        Importance::Low => marker("lowImportance", t.icon),
        Importance::Normal => {}
    }
    if m.pinned {
        marker("pin", t.text_dim);
    }
    if m.answered {
        marker("reply", t.text_dim);
    } else if m.forwarded {
        marker("forward", t.text_dim);
    }
    let who_font = if unread { semibold(LIST_TEXT) } else { regular(LIST_TEXT) };
    let mut who_x = left;
    if m.draft {
        let w = p.text(pos2(left, line1), Align2::LEFT_CENTER, tl!("[Draft]"), semibold(LIST_TEXT), t.red).width();
        who_x += w + 5.0;
    }
    paint_clipped(p, pos2(who_x, line1), (x - who_x - 4.0).max(0.0), &who, who_font, t.text);

    // Line two: the subject, with the conversation count and category dots on the right (and
    // the quick actions past them while the pointer is over the row).
    let mut sx = right;
    if let Some((delete, flag)) = &quick {
        for (resp, at, icon, color) in
            [(delete, delete_rect, "delete", t.text_dim), (flag, flag_rect, "flag", if flagged { t.icon } else { t.text_dim })]
        {
            if resp.hovered() {
                p.rect_filled(at, 4.0, t.pressed);
            }
            icons::paint(p, at.shrink(3.0), icon, color, t.accent);
        }
        sx = flag_rect.min.x - 4.0;
    }
    for c in m.categories.iter().take(4) {
        let color = crate::theme::c32(app.session.mailbox.category_color(c));
        p.rect_filled(Rect::from_center_size(pos2(sx - 5.0, line2), vec2(10.0, 10.0)), 2.0, color);
        sx -= 13.0;
    }
    if row.members.len() > 1 {
        let n = row.members.len().to_string();
        let w = n.len() as f32 * 6.5 + 10.0;
        let br = Rect::from_min_max(pos2(sx - w, line2 - 8.0), pos2(sx, line2 + 8.0));
        p.rect_filled(br, 8.0, t.pressed);
        p.text(br.center(), Align2::CENTER_CENTER, n, regular(10.5), t.text_dim);
        sx -= w + 5.0;
    }
    let subject = if m.subject.trim().is_empty() { tl!("(no subject)") } else { m.subject.as_str() };
    let (sfont, scolor) = if unread { (semibold(LIST_TEXT), t.accent_text) } else { (regular(LIST_TEXT), t.text) };
    paint_clipped(p, pos2(left, line2), (sx - left - 6.0).max(0.0), subject, sfont, scolor);

    // Then the preview.
    let lines = app.session.prefs.preview_lines.min(3);
    if lines > 0 {
        let snippet = m.snippet(usize::from(lines) * 110);
        let mut job = egui::text::LayoutJob::single_section(
            snippet,
            egui::text::TextFormat { font_id: regular(LIST_TEXT), color: t.text_dim, ..Default::default() },
        );
        job.wrap = egui::text::TextWrapping {
            max_width: (right - left).max(10.0),
            max_rows: usize::from(lines),
            break_anywhere: false,
            overflow_character: Some('…'),
        };
        let galley = p.layout_job(job);
        p.galley(pos2(left, rect.min.y + 45.0), galley, t.text_dim);
    }

    // Quick actions over the row under the pointer.
    if resp.hovered() && !selected {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(400));
    }
    // Dragging a row carries it to a folder: the whole selection when the row is part of it,
    // otherwise just what the row stands for.
    if resp.drag_started() {
        let ids: Vec<Id> = if selected { app.session.sel.messages.to_vec() } else { row.members.clone() };
        resp.dnd_set_drag_payload(MessageDrag(ids));
    }
    if let Some((delete, flag)) = &quick {
        if delete.clicked() {
            let _ = app.run("mail.delete", json!({"ids": row.members}));
            return;
        }
        if flag.clicked() {
            let _ = app.run("mail.toggleFlag", json!({"ids": [row.id]}));
        }
    }
    if read_resp.clicked() {
        // A conversation is read once all of it is; its newest message makes it unread again.
        let (command, ids) = if unread { ("mail.markRead", row.members.clone()) } else { ("mail.markUnread", vec![row.id]) };
        let _ = app.run(command, json!({"ids": ids}));
    }
    clicks(app, ui, &resp, row.id, &m);
}

fn clicks(app: &mut MailApp, ui: &Ui, resp: &egui::Response, id: Id, m: &Message) {
    let mods = ui.input(|i| i.modifiers);
    if resp.clicked() {
        let _ = app.run("mail.select", json!({"id": id, "extend": mods.command, "range": mods.shift}));
    }
    // A double click opens the message: a draft in the composer, any other in a window of its own.
    if resp.double_clicked() {
        let _ = if m.draft { app.run("mail.editDraft", json!({"id": id})) } else { app.run("ui.openWindow", json!({"message": id})) };
    }
    // A right click on a row outside the selection acts on that row.
    if resp.secondary_clicked() && !app.session.sel.messages.contains(&id) {
        let _ = app.execute("mail.select", json!({"id": id}));
    }
    let (read, flagged, pinned) = (m.read, m.flag == FlagStatus::Flagged, m.pinned);
    let in_junk = app.session.mailbox.folder(m.folder).is_some_and(|f| f.kind == FolderKind::Junk);
    resp.context_menu(|ui| {
        ui.set_min_width(200.0);
        use crate::widgets::mi;
        if m.draft {
            mi(ui, app, "Open Draft", "mail.editDraft", json!({}));
            ui.separator();
        } else {
            mi(ui, app, "Open in a Window", "ui.openWindow", json!({"message": id}));
            ui.separator();
        }
        mi(ui, app, "Reply", "mail.reply", json!({}));
        mi(ui, app, "Reply All", "mail.replyAll", json!({}));
        mi(ui, app, "Forward", "mail.forward", json!({}));
        ui.separator();
        mi(ui, app, if read { "Mark as Unread" } else { "Mark as Read" }, if read { "mail.markUnread" } else { "mail.markRead" }, json!({}));
        mi(ui, app, if flagged { "Clear Flag" } else { "Follow Up" }, if flagged { "mail.clearFlag" } else { "mail.flag" }, json!({}));
        mi(ui, app, if pinned { "Unpin" } else { "Pin" }, "mail.pin", json!({}));
        ui.menu_button(tl!("Categorize"), |ui| {
            let ids = app.session.sel.messages.clone();
            crate::ribbon::category_menu(
                ui,
                app,
                "mail.categorize",
                |app, name| ids.iter().all(|id| app.session.mailbox.message(*id).is_some_and(|m| m.has_category(name))),
                Some("mail.clearCategories"),
            );
        });
        ui.separator();
        ui.menu_button(tl!("Move"), |ui| {
            crate::ribbon::folder_menu(ui, app, |app, folder| {
                let _ = app.run("mail.move", json!({"folder": folder}));
            });
        });
        mi(ui, app, "Archive", "mail.archive", json!({}));
        if in_junk {
            mi(ui, app, "Not Junk", "mail.notJunk", json!({}));
        } else {
            mi(ui, app, "Junk", "mail.junk", json!({}));
        }
        mi(ui, app, "Delete", "mail.delete", json!({}));
        ui.separator();
        mi(ui, app, "Add to Tasks", "tasks.fromMessage", json!({}));
        mi(ui, app, "Add Sender to Contacts", "people.fromMessage", json!({}));
        mi(ui, app, "Find Related Messages", "mail.findRelated", json!({}));
        mi(ui, app, "Save Message As…", "file.exportMessage", json!({}));
    });
}

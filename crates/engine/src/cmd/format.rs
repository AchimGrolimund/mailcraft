//! Writing in rich text: the selection in the message being written, typing, formatting and
//! undo inside the message (`mailcraft_model::rich`).
//!
//! These are pure commands: they change the message being written, never the mailbox, so they
//! are not on the mailbox's undo history. The message has its own (`compose.undo`).
//! Positions count characters of the body's text, a paragraph break being one.

use mailcraft_model::rich::{Font, Format, ListKind, MAX_SIZE, MIN_SIZE, safe_color};
use mailcraft_model::{ModelError, RichText};
use serde_json::{Value, json};

use super::{bad, fail, needs_compose};
use crate::{BodyFormat, CmdError, CmdResult, CommandSpec, Compose, Session, p};

const RANGE: &str = r#"{"from"?: number, "to"?: number} (default: the selection)"#;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("compose.select", "Select Text", "Message", select).pure().params(r#"{"from": number, "to"?: number} (to: the caret, default from)"#).when(needs_compose),
        CommandSpec::new("compose.type", "Type", "Message", type_text)
            .pure()
            .params(r#"{"text": string} — over the selection, in the formatting at the caret; a line break starts a paragraph (of the same kind; a line break alone in an empty list item ends the list there)"#)
            .when(needs_compose),
        CommandSpec::new("compose.delete", "Delete Text", "Message", delete)
            .pure()
            .params(r#"{"from"?: number, "to"?: number, "forward"?: bool} (default: the selection, else the character before the caret, or after it with forward)"#)
            .when(needs_compose),
        CommandSpec::new("compose.undo", "Undo Typing", "Message", undo).pure().when(|s| match &s.compose {
            None => Some("no message is being written"),
            Some(c) if !c.body.can_undo() => Some("nothing to undo in the message"),
            Some(_) => None,
        }),
        CommandSpec::new("compose.redo", "Redo Typing", "Message", redo).pure().when(|s| match &s.compose {
            None => Some("no message is being written"),
            Some(c) if !c.body.can_redo() => Some("nothing to redo in the message"),
            Some(_) => None,
        }),
        CommandSpec::new("format.bold", "Bold", "Message › Basic Text", bold).pure().params(RANGE).when(needs_html),
        CommandSpec::new("format.italic", "Italic", "Message › Basic Text", italic).pure().params(RANGE).when(needs_html),
        CommandSpec::new("format.underline", "Underline", "Message › Basic Text", underline).pure().params(RANGE).when(needs_html),
        CommandSpec::new("format.size", "Font Size", "Message › Basic Text", size)
            .pure()
            .params(r#"{"size": number (points, 8–36) | null (the message's own size), "from"?: number, "to"?: number}"#)
            .when(needs_html),
        CommandSpec::new("format.color", "Font Color", "Message › Basic Text", color)
            .pure()
            .params(r##"{"value": "#rrggbb" | null (the message's own colour), "from"?: number, "to"?: number}"##)
            .when(needs_html),
        CommandSpec::new("format.font", "Font", "Message › Basic Text", font)
            .pure()
            .params(r#"{"value": "Arial"|"Calibri"|"Georgia"|"Verdana"|"System" | null (the message's own font), "from"?: number, "to"?: number}"#)
            .when(needs_html),
        CommandSpec::new("format.bullets", "Bullets", "Message › Basic Text", bullets).pure().params(RANGE).when(needs_html),
        CommandSpec::new("format.numbering", "Numbering", "Message › Basic Text", numbering).pure().params(RANGE).when(needs_html),
        CommandSpec::new("format.clear", "Clear All Formatting", "Message › Basic Text", clear).pure().params(RANGE).when(needs_html),
        CommandSpec::new("compose.link", "Link", "Message › Include", link)
            .pure()
            .params(r#"{"url"?: string (http, https, mailto or tel; none: remove the link), "from"?: number, "to"?: number}"#)
            .when(needs_html),
        CommandSpec::new("format.indent", "Increase Indent", "Message › Basic Text", indent)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — the list items there a level deeper (eight levels)"#)
            .when(needs_html),
        CommandSpec::new("format.outdent", "Decrease Indent", "Message › Basic Text", outdent)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — the list items there a level out; from the outermost level, out of the list"#)
            .when(needs_html),
        CommandSpec::new("edit.copy", "Copy", "Message › Clipboard", copy)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — answers the selection as {"html", "text"} for the clipboard (HTML only in an HTML message); changes nothing"#)
            .when(needs_compose),
        CommandSpec::new("edit.cut", "Cut", "Message › Clipboard", cut)
            .pure()
            .params(r#"{"from"?: number, "to"?: number} — answers the selection as edit.copy does, and takes it out of the text"#)
            .when(needs_compose),
        CommandSpec::new("edit.paste", "Paste", "Message › Clipboard", paste)
            .pure()
            .params(r#"{"html"?: string, "text"?: string} — over the selection; HTML is reduced to bold, italic, underline, a size, links and lists (to plain text in a plain-text message)"#)
            .when(needs_compose),
        CommandSpec::new("compose.formatHtml", "HTML", "Format Text › Format", format_html).pure().when(needs_compose),
        CommandSpec::new("compose.formatPlain", "Plain Text", "Format Text › Format", format_plain).pure().when(needs_compose),
    ]
}

/// Enabled only while a message in HTML is being written.
fn needs_html(s: &Session) -> Option<&'static str> {
    match &s.compose {
        None => Some("no message is being written"),
        Some(c) if c.format == BodyFormat::Plain => Some("the message is plain text; switch it to HTML first"),
        Some(_) => None,
    }
}

fn compose_mut(s: &mut Session) -> Result<&mut Compose, CmdError> {
    s.compose.as_mut().ok_or_else(|| fail("no message is being written"))
}

fn model(e: ModelError) -> CmdError {
    fail(e.to_string())
}

/// A position from the params.
fn position(v: &Value, key: &str) -> Result<Option<usize>, CmdError> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => p::u64(v, key).and_then(|n| usize::try_from(n).ok()).map(Some).ok_or_else(|| bad(format!("`{key}` is a position (a number)"))),
    }
}

/// Select what `from` and `to` name, when the params name something.
fn select_range(c: &mut Compose, v: &Value) -> Result<(), CmdError> {
    let (from, to) = (position(v, "from")?, position(v, "to")?);
    if from.is_none() && to.is_none() {
        return Ok(());
    }
    let sel = c.body.selection();
    let from = from.unwrap_or(sel.start());
    let to = to.unwrap_or(from);
    c.body.select(from, to).map_err(model)
}

/// Run an edit of the body; the message is changed when its text is.
fn edit<T>(s: &mut Session, v: &Value, f: impl FnOnce(&mut Compose) -> Result<T, ModelError>) -> Result<T, CmdError> {
    let c = compose_mut(s)?;
    select_range(c, v)?;
    let before = c.body.text().clone();
    let out = f(c).map_err(model)?;
    if *c.body.text() != before {
        c.dirty = true;
    }
    Ok(out)
}

/// What the answer of an edit says: the selection now.
fn state(s: &Session) -> Value {
    match &s.compose {
        Some(c) => {
            let sel = c.body.selection();
            json!({"from": sel.anchor, "to": sel.head, "length": c.body.text().len()})
        }
        None => json!({}),
    }
}

fn select(s: &mut Session, v: &Value) -> CmdResult {
    let from = position(v, "from")?.ok_or_else(|| bad("`from` (number) is required"))?;
    let to = position(v, "to")?.unwrap_or(from);
    compose_mut(s)?.body.select(from, to).map_err(model)?;
    Ok(state(s))
}

fn type_text(s: &mut Session, v: &Value) -> CmdResult {
    let text = p::req_str(v, "text")?.to_string();
    edit(s, &json!({}), |c| c.body.type_text(&text))?;
    Ok(state(s))
}

fn delete(s: &mut Session, v: &Value) -> CmdResult {
    let forward = p::bool(v, "forward").unwrap_or(false);
    let named = position(v, "from")?.is_some() || position(v, "to")?.is_some();
    edit(s, v, |c| {
        let sel = c.body.selection();
        if named || !sel.is_empty() {
            c.body.delete(sel.start(), sel.end())
        } else if forward {
            c.body.delete_forward()
        } else {
            c.body.backspace()
        }
    })?;
    Ok(state(s))
}

fn undo(s: &mut Session, _: &Value) -> CmdResult {
    let c = compose_mut(s)?;
    if !c.body.undo() {
        return Err(fail("nothing to undo in the message"));
    }
    c.dirty = true;
    Ok(state(s))
}

fn redo(s: &mut Session, _: &Value) -> CmdResult {
    let c = compose_mut(s)?;
    if !c.body.redo() {
        return Err(fail("nothing to redo in the message"));
    }
    c.dirty = true;
    Ok(state(s))
}

fn bold(s: &mut Session, v: &Value) -> CmdResult {
    let on = edit(s, v, |c| c.body.toggle_bold())?;
    Ok(json!({"bold": on}))
}

fn italic(s: &mut Session, v: &Value) -> CmdResult {
    let on = edit(s, v, |c| c.body.toggle_italic())?;
    Ok(json!({"italic": on}))
}

fn underline(s: &mut Session, v: &Value) -> CmdResult {
    let on = edit(s, v, |c| c.body.toggle_underline())?;
    Ok(json!({"underline": on}))
}

fn color(s: &mut Session, v: &Value) -> CmdResult {
    let value = match v.get("value") {
        Some(Value::Null) => None,
        Some(_) => Some(p::str(v, "value").and_then(safe_color).ok_or_else(|| bad("`value` is a colour written #rrggbb, or null"))?),
        None => return Err(bad("`value` (#rrggbb, or null) is required")),
    };
    edit(s, v, |c| c.body.format(Format::Color(value.clone())))?;
    Ok(json!({"value": value}))
}

fn font(s: &mut Session, v: &Value) -> CmdResult {
    let value = match v.get("value") {
        Some(Value::Null) => None,
        Some(_) => {
            Some(p::str(v, "value").and_then(Font::parse).ok_or_else(|| bad("`value` is Arial, Calibri, Georgia, Verdana or System, or null"))?)
        }
        None => return Err(bad("`value` (a font, or null) is required")),
    };
    edit(s, v, |c| c.body.format(Format::Font(value)))?;
    Ok(json!({"value": value.map(Font::label)}))
}

fn size(s: &mut Session, v: &Value) -> CmdResult {
    let size = match v.get("size") {
        Some(Value::Null) => None,
        Some(_) => {
            let n = p::u64(v, "size").filter(|n| (u64::from(MIN_SIZE)..=u64::from(MAX_SIZE)).contains(n));
            Some(
                n.and_then(|n| u8::try_from(n).ok())
                    .ok_or_else(|| bad(format!("`size` is a number of points, {MIN_SIZE} to {MAX_SIZE}, or null")))?,
            )
        }
        None => return Err(bad("`size` (points, or null) is required")),
    };
    edit(s, v, |c| c.body.format(Format::Size(size)))?;
    Ok(json!({"size": size}))
}

fn list(s: &mut Session, v: &Value, kind: ListKind) -> CmdResult {
    let now = edit(s, v, |c| c.body.toggle_list(kind))?;
    Ok(json!({"list": now}))
}

fn bullets(s: &mut Session, v: &Value) -> CmdResult {
    list(s, v, ListKind::Bullet)
}

fn numbering(s: &mut Session, v: &Value) -> CmdResult {
    list(s, v, ListKind::Numbered)
}

fn clear(s: &mut Session, v: &Value) -> CmdResult {
    edit(s, v, |c| c.body.clear_formatting())?;
    Ok(state(s))
}

fn link(s: &mut Session, v: &Value) -> CmdResult {
    let url = match v.get("url") {
        None | Some(Value::Null) => None,
        Some(Value::String(u)) => Some(u.clone()),
        Some(_) => return Err(bad("`url` is a string")),
    };
    edit(s, v, |c| c.body.set_link(url.as_deref()))?;
    Ok(state(s))
}

fn indent(s: &mut Session, v: &Value) -> CmdResult {
    edit(s, v, |c| c.body.indent(1))?;
    Ok(json!({"level": s.compose.as_ref().map_or(0, |c| c.body.text().level_at(c.body.selection().start()))}))
}

fn outdent(s: &mut Session, v: &Value) -> CmdResult {
    edit(s, v, |c| c.body.indent(-1))?;
    let c = s.compose.as_ref();
    Ok(json!({
        "level": c.map_or(0, |c| c.body.text().level_at(c.body.selection().start())),
        "list": c.map(|c| c.body.text().list_at(c.body.selection().start())),
    }))
}

/// The selection (after the params' range, when they name one) as the clipboard takes it: HTML
/// of the set, written as the message's own HTML part is, and its plain text.
fn clipboard(s: &mut Session, v: &Value) -> Result<Value, CmdError> {
    let c = compose_mut(s)?;
    select_range(c, v)?;
    let sel = c.body.selection();
    if sel.is_empty() {
        return Err(fail("nothing is selected in the message"));
    }
    let part = c.body.text().slice(sel.start(), sel.end()).map_err(model)?;
    let html = (c.format == BodyFormat::Html).then(|| mailcraft_mime::rich::to_html(&part));
    Ok(json!({"text": part.to_plain(), "html": html}))
}

fn copy(s: &mut Session, v: &Value) -> CmdResult {
    clipboard(s, v)
}

fn cut(s: &mut Session, v: &Value) -> CmdResult {
    let out = clipboard(s, v)?;
    edit(s, &json!({}), |c| {
        let sel = c.body.selection();
        c.body.delete(sel.start(), sel.end())
    })?;
    Ok(out)
}

/// Most HTML or text one paste takes, in bytes.
pub(crate) const MAX_PASTE: usize = 4 << 20;

fn paste(s: &mut Session, v: &Value) -> CmdResult {
    let plain = s.compose.as_ref().is_some_and(|c| c.format == BodyFormat::Plain);
    let fragment = match (p::str(v, "html"), p::str(v, "text")) {
        (Some(html), _) if html.len() <= MAX_PASTE => {
            let rich = mailcraft_mime::rich::from_html(html);
            if plain { RichText::from_plain(&rich.text()) } else { rich }
        }
        (None, Some(text)) if text.len() <= MAX_PASTE => RichText::from_plain(text),
        (None, None) => return Err(bad("`html` or `text` (string) is required")),
        _ => return Err(fail("that is too much to paste into one message")),
    };
    edit(s, &json!({}), |c| c.body.paste(&fragment))?;
    Ok(state(s))
}

fn set_format(s: &mut Session, format: BodyFormat) -> CmdResult {
    let c = compose_mut(s)?;
    if c.format != format {
        c.format = format;
        c.dirty = true;
    }
    Ok(json!({"format": c.format}))
}

fn format_html(s: &mut Session, _: &Value) -> CmdResult {
    set_format(s, BodyFormat::Html)
}

fn format_plain(s: &mut Session, _: &Value) -> CmdResult {
    set_format(s, BodyFormat::Plain)
}

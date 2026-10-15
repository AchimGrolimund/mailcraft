//! Signatures (R4.5): making them, their pictures, and which one each account starts its
//! messages with. The model and its caps are `mailcraft_model::signature`; what goes into a
//! message is `compose.signature`.

use mailcraft_model::rich::RichText;
use mailcraft_model::signature::{Layout, Signature};
use mailcraft_model::{Id, clean_name};
use serde_json::{Value, json};

use super::{bad, done, fail};
use crate::{CmdError, CmdResult, CommandSpec, Session, p};

/// Most signatures one mailbox keeps.
const MAX_SIGNATURES: usize = 100;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("signature.set", "Signatures", "File › Options", set).params(
            r#"{"name": string, "text"?: string (plain text) | "body"?: RichText | "html"?: string (pasted HTML, reduced to the set; pictures in it are not taken), "default"?: bool (use it for this account's new messages, and replies and forwards), "account"?: number} — makes the signature or replaces its text"#,
        ),
        CommandSpec::new("signature.remove", "Delete Signature", "File › Options", remove).params(r#"{"name": string}"#),
        CommandSpec::new("signature.rename", "Rename Signature", "File › Options", rename).params(r#"{"name": string, "to": string}"#),
        CommandSpec::new("signature.defaults", "Choose Default Signature", "File › Options", defaults).params(
            r#"{"account"?: number (default: the default account), "new"?: string ("" for none), "reply"?: string (replies and forwards; "" for none)}"#,
        ),
        CommandSpec::new("signature.picture.add", "Insert Picture", "File › Options › Signatures", picture_add).params(
            r#"{"name": string (the signature), "data"?: base64 | "path"?: string (PNG, JPEG or GIF; never a web address), "file"?: string (its name), "alt"?: string, "at"?: number (a position in the text; default: the start)} — answers the picture's id"#,
        ),
        CommandSpec::new("signature.picture.set", "Picture Size and Description", "File › Options › Signatures", picture_set)
            .params(r#"{"name": string, "id": number, "width"?: number (pixels, at most 600) | null (its own), "alt"?: string}"#),
        CommandSpec::new("signature.picture.remove", "Remove Picture", "File › Options › Signatures", picture_remove)
            .params(r#"{"name": string, "id": number}"#),
        CommandSpec::new("signature.layout", "Picture Layout", "File › Options › Signatures", layout)
            .params(r#"{"name": string, "value": "inline"|"pictureLeft"|"pictureAbove"}"#),
    ]
}

/// The signature `name` names (any letter case), to change.
fn named<'a>(s: &'a mut Session, v: &Value) -> Result<&'a mut Signature, CmdError> {
    let name = p::req_str(v, "name")?.trim().to_string();
    s.mailbox.signatures.iter_mut().find(|x| x.name.eq_ignore_ascii_case(&name)).ok_or_else(|| fail(format!("no signature named \"{name}\"")))
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    let name = clean_name(p::req_str(v, "name")?).ok_or_else(|| bad("`name` is empty"))?;
    let body = match (v.get("body"), p::str(v, "html"), p::str(v, "text")) {
        (Some(body), _, _) => serde_json::from_value::<RichText>(body.clone()).map_err(|e| bad(format!("`body` is not rich text: {e}")))?,
        (None, Some(html), _) => {
            if html.len() > super::format::MAX_PASTE {
                return Err(bad("`html` is too long"));
            }
            mailcraft_mime::rich::from_html(html)
        }
        (None, None, Some(text)) => Signature::plain_text("", text).body().clone(),
        (None, None, None) => return Err(bad("`text`, `body` or `html` is required")),
    };
    match s.mailbox.signatures.iter().position(|x| x.name.eq_ignore_ascii_case(&name)) {
        Some(i) => {
            if let Some(sig) = s.mailbox.signatures.get_mut(i) {
                sig.set_body(body).map_err(|e| fail(e.to_string()))?;
            }
        }
        None if s.mailbox.signatures.len() >= MAX_SIGNATURES => return Err(fail("there are too many signatures")),
        None => {
            let mut sig = Signature::plain_text(name.clone(), "");
            sig.set_body(body).map_err(|e| fail(e.to_string()))?;
            s.mailbox.signatures.push(sig);
        }
    }
    if p::bool(v, "default") == Some(true) {
        let account = account_of(s, v)?;
        let a = s.mailbox.account_mut(account)?;
        a.signature = name.clone();
        a.reply_signature = Some(name);
    }
    done()
}

/// The account a command names, or the default one.
fn account_of(s: &Session, v: &Value) -> Result<Id, CmdError> {
    match p::id(v, "account") {
        Some(id) if s.mailbox.account(id).is_some() => Ok(id),
        Some(id) => Err(fail(format!("no account with id {id}"))),
        None => s.mailbox.default_account().map(|a| a.id).ok_or_else(|| fail("add an account first")),
    }
}

fn remove(s: &mut Session, v: &Value) -> CmdResult {
    let name = p::req_str(v, "name")?.trim().to_string();
    let before = s.mailbox.signatures.len();
    s.mailbox.signatures.retain(|x| !x.name.eq_ignore_ascii_case(&name));
    if s.mailbox.signatures.len() == before {
        return Err(fail(format!("no signature named \"{name}\"")));
    }
    for a in &mut s.mailbox.accounts {
        // An account that had it for one kind of message has none there now; the other stays.
        let reply = a.reply_signature().to_string();
        if a.signature.eq_ignore_ascii_case(&name) {
            a.signature.clear();
        }
        a.reply_signature = Some(if reply.eq_ignore_ascii_case(&name) { String::new() } else { reply });
    }
    done()
}

fn rename(s: &mut Session, v: &Value) -> CmdResult {
    let to = clean_name(p::req_str(v, "to")?).ok_or_else(|| bad("`to` is empty"))?;
    let from = named(s, v)?.name.clone();
    if !to.eq_ignore_ascii_case(&from) && s.mailbox.signature(&to).is_some() {
        return Err(fail(format!("there is already a signature named \"{to}\"")));
    }
    named(s, v)?.name = to.clone();
    for a in &mut s.mailbox.accounts {
        let reply = a.reply_signature().to_string();
        if a.signature.eq_ignore_ascii_case(&from) {
            a.signature = to.clone();
        }
        a.reply_signature = Some(if reply.eq_ignore_ascii_case(&from) { to.clone() } else { reply });
    }
    Ok(json!({"name": to}))
}

fn defaults(s: &mut Session, v: &Value) -> CmdResult {
    let account = account_of(s, v)?;
    // A name, or "" for none; a name has to be a signature's (and is kept as that one spells it).
    let pick = |key: &str| -> Result<Option<String>, CmdError> {
        match p::str(v, key).map(str::trim) {
            None => Ok(None),
            Some("") => Ok(Some(String::new())),
            Some(name) => s.mailbox.signature(name).map(|sig| Some(sig.name.clone())).ok_or_else(|| fail(format!("no signature named \"{name}\""))),
        }
    };
    let (new, reply) = (pick("new")?, pick("reply")?);
    if new.is_none() && reply.is_none() {
        return Err(bad("`new` or `reply` is required"));
    }
    let a = s.mailbox.account_mut(account)?;
    // Pinned before `signature` changes, so a reply that followed it keeps what it had.
    a.reply_signature = Some(reply.unwrap_or_else(|| a.reply_signature().to_string()));
    if let Some(new) = new {
        a.signature = new;
    }
    Ok(json!({"account": account, "new": a.signature, "reply": a.reply_signature()}))
}

fn picture_add(s: &mut Session, v: &Value) -> CmdResult {
    let path = p::str(v, "path").map(str::trim).unwrap_or_default();
    if p::str(v, "data").is_none() && ["http:", "https:", "ftp:", "//"].iter().any(|scheme| path.to_ascii_lowercase().starts_with(scheme)) {
        return Err(fail("pictures from the web are not downloaded: save the picture and add it from the file"));
    }
    let (_, bytes) = super::source(&json!({"data": v.get("data"), "path": v.get("path")}))?;
    let file = p::str(v, "file").or_else(|| (!path.is_empty()).then(|| path.rsplit(['/', '\\']).next().unwrap_or(path))).unwrap_or("picture");
    let at = p::u64(v, "at").and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let alt = p::str(v, "alt");
    let sig = named(s, v)?;
    let id = sig.add_picture(file, bytes, alt, Some(at)).map_err(|e| fail(e.to_string()))?;
    Ok(json!({"id": id, "bytes": sig.picture_bytes(), "heavy": sig.picture_bytes() > mailcraft_model::signature::HEAVY_BYTES}))
}

fn picture_set(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::u64(v, "id").and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad("`id` is a picture's id"))?;
    let width = match v.get("width") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(_) => Some(Some(
            p::u64(v, "width").and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0).ok_or_else(|| bad("`width` is a number of pixels, or null"))?,
        )),
    };
    let alt = p::str(v, "alt");
    named(s, v)?.set_picture(id, width, alt).map_err(|e| fail(e.to_string()))?;
    done()
}

fn picture_remove(s: &mut Session, v: &Value) -> CmdResult {
    let id = p::u64(v, "id").and_then(|n| u32::try_from(n).ok()).ok_or_else(|| bad("`id` is a picture's id"))?;
    named(s, v)?.remove_picture(id).map_err(|e| fail(e.to_string()))?;
    done()
}

fn layout(s: &mut Session, v: &Value) -> CmdResult {
    let layout = p::req_str(v, "value").ok().and_then(Layout::parse).ok_or_else(|| bad("`value` is inline, pictureLeft or pictureAbove"))?;
    named(s, v)?.layout = layout;
    done()
}

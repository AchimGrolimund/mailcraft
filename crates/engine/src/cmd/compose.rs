//! Writing mail: new messages, replies, forwards, drafts, sending.

use mailcraft_model::{AccountKind, Address, Attachment, Body, FolderKind, Id, Importance, Message, format_list};
use serde_json::{Value, json};

use super::{bad, base64_decode, done, fail, needs_compose, needs_message, source};
use crate::{CmdError, CmdResult, CommandSpec, Compose, ComposeMode, Session, p};

/// Largest single attachment and most attachments on one message.
const MAX_ATTACHMENT: usize = 64 << 20;
const MAX_ATTACHMENTS: usize = 200;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("mail.new", "New Email", "Home › New", new)
            .pure()
            .key("Mod+N")
            .params(
                r#"{"to"?: string, "cc"?: string, "bcc"?: string, "subject"?: string, "body"?: string, "account"?: number, "mailto"?: string (a mailto: link: its recipients, subject and body start the message, and nothing else in it is used)}"#,
            ),
        CommandSpec::new("mail.reply", "Reply", "Home › Respond", reply).pure().key("Mod+R").params(SOURCE).selection(needs_message),
        CommandSpec::new("mail.replyAll", "Reply All", "Home › Respond", reply_all).pure().key("Mod+Shift+R").params(SOURCE).selection(needs_message),
        CommandSpec::new("mail.forward", "Forward", "Home › Respond", forward).pure().key("Mod+F").params(SOURCE).selection(needs_message),
        CommandSpec::new("mail.editDraft", "Open Draft", "Mail", edit_draft).pure().params(SOURCE).selection(needs_message),
        CommandSpec::new("compose.set", "Edit Message", "Message", set)
            .pure()
            .params(r#"{"to"?: string, "cc"?: string, "bcc"?: string, "subject"?: string, "body"?: string, "importance"?: "low"|"normal"|"high", "account"?: number}"#)
            .when(needs_compose),
        CommandSpec::new("compose.attach", "Attach File", "Message › Include", attach)
            .pure()
            .params(r#"{"path": string} | {"name": string, "data": base64}, "mime"?: string"#)
            .when(needs_compose),
        CommandSpec::new("compose.removeAttachment", "Remove Attachment", "Message › Include", remove_attachment).pure().params(r#"{"index": number}"#).when(needs_compose),
        CommandSpec::new("compose.signature", "Signature", "Message › Include", signature).pure().params(r#"{"name"?: string} (default: the account's signature)"#).when(needs_compose),
        CommandSpec::new("compose.checkNames", "Check Names", "Message › Names", check_names).pure().key("Mod+K").when(needs_compose),
        CommandSpec::new("compose.suggest", "Suggest Recipients", "Message › Names", suggest_command)
            .pure()
            .params(r#"{"query": string, "limit"?: 1-20} — answers who the start of a name or address could be: contacts first, then people mail was exchanged with, the latest first"#),
        CommandSpec::new("compose.highImportance", "High Importance", "Message › Tags", high_importance).pure().when(needs_compose),
        CommandSpec::new("compose.lowImportance", "Low Importance", "Message › Tags", low_importance).pure().when(needs_compose),
        CommandSpec::new("compose.send", "Send", "Message", send).key("Mod+Enter / Alt+S").when(needs_compose),
        CommandSpec::new("compose.saveDraft", "Save Draft", "Message", save_draft).key("Mod+S").when(needs_compose),
        CommandSpec::new("compose.close", "Close Message", "Message", close).pure().key("Escape").when(needs_compose),
        CommandSpec::new("compose.discard", "Discard", "Message", discard).when(needs_compose),
    ]
}

const SOURCE: &str = r#"{"id"?: number} (default: the selected message)"#;

fn compose_mut(s: &mut Session) -> Result<&mut Compose, CmdError> {
    s.compose.as_mut().ok_or_else(|| fail("no message is being written"))
}

/// The signature block for an account (`\n\n-- \nname`), or nothing.
fn signature_block(s: &Session, account: Id, name: Option<&str>) -> String {
    let name = name.map(str::to_string).or_else(|| s.mailbox.account(account).map(|a| a.signature.clone())).unwrap_or_default();
    match s.mailbox.signature(&name) {
        Some(sig) if !sig.text.trim().is_empty() => format!("\n\n{}", sig.text.trim_end()),
        _ => String::new(),
    }
}

fn new(s: &mut Session, v: &Value) -> CmdResult {
    let account =
        p::id(v, "account").filter(|a| s.mailbox.account(*a).is_some()).or_else(|| s.mailbox.folder(s.view.folder).map(|f| f.account)).unwrap_or(0);
    // A link clicked somewhere else says who the message is for and how it starts; what the
    // caller names itself comes first.
    let link = match p::str(v, "mailto") {
        Some(link) => mailcraft_mime::parse_mailto(link).ok_or_else(|| bad("`mailto` is a mailto: link"))?,
        None => mailcraft_mime::Mailto::default(),
    };
    let text = |key: &str, linked: String| p::str(v, key).map_or(linked, str::to_string);
    let body = text("body", link.body);
    let c = Compose {
        mode: ComposeMode::New,
        account,
        to: text("to", link.to.join("; ")),
        cc: text("cc", link.cc.join("; ")),
        bcc: text("bcc", link.bcc.join("; ")),
        subject: text("subject", link.subject),
        body: format!("{body}{}", signature_block(s, account, None)),
        ..Default::default()
    };
    s.compose = Some(c);
    s.view.module = crate::Module::Mail;
    Ok(json!({"composing": true}))
}

fn source_message(s: &Session, v: &Value) -> Result<Message, CmdError> {
    let id = p::id(v, "id").or(s.sel.focus()).ok_or_else(|| fail("select a message first"))?;
    s.mailbox.message(id).cloned().ok_or_else(|| fail(format!("no message with id {id}")))
}

/// `RE: subject`, unless it already starts that way.
fn prefixed(prefix: &str, subject: &str) -> String {
    let lower = subject.trim_start().to_ascii_lowercase();
    if lower.starts_with(&format!("{}:", prefix.to_ascii_lowercase())) { subject.trim().to_string() } else { format!("{prefix}: {}", subject.trim()) }
}

/// The original message under a reply or forward, as plain text.
fn quoted(s: &Session, m: &Message) -> String {
    let mut out = String::from("\n\n-----Original Message-----\n");
    out.push_str(&format!("From: {}\n", m.from.full()));
    out.push_str(&format!("Sent: {} {}\n", s.local(m.date).date.long(), s.local(m.date).time_short(s.prefs.twelve_hour)));
    if !m.to.is_empty() {
        out.push_str(&format!("To: {}\n", format_list(&m.to)));
    }
    if !m.cc.is_empty() {
        out.push_str(&format!("Cc: {}\n", format_list(&m.cc)));
    }
    out.push_str(&format!("Subject: {}\n\n", m.subject));
    out.push_str(m.body.text.trim_end());
    out.push('\n');
    out
}

fn respond(s: &mut Session, v: &Value, mode: ComposeMode) -> CmdResult {
    let m = source_message(s, v)?;
    let mine: Vec<String> = s.mailbox.accounts.iter().map(|a| a.email.to_lowercase()).filter(|e| !e.is_empty()).collect();
    let is_mine = |a: &Address| mine.contains(&a.email.to_lowercase());
    let (to, cc) = match mode {
        ComposeMode::Forward => (Vec::new(), Vec::new()),
        _ => {
            // A reply to a message I sent goes to the people I sent it to.
            let mut to: Vec<Address> = if is_mine(&m.from) && !m.to.is_empty() {
                m.to.clone()
            } else if m.reply_to.is_empty() {
                vec![m.from.clone()]
            } else {
                m.reply_to.clone()
            };
            let mut cc = Vec::new();
            if mode == ComposeMode::ReplyAll {
                for a in m.to.iter().filter(|a| !is_mine(a)) {
                    if !to.iter().any(|x| x.same_mailbox(a)) {
                        to.push(a.clone());
                    }
                }
                for a in m.cc.iter().filter(|a| !is_mine(a)) {
                    if !to.iter().chain(&cc).any(|x| x.same_mailbox(a)) {
                        cc.push(a.clone());
                    }
                }
            }
            (to, cc)
        }
    };
    let mut references = m.references.clone();
    if !m.message_id.is_empty() {
        references.push(m.message_id.clone());
    }
    // A long thread's References stay bounded: the root and the latest ones.
    if references.len() > 20 {
        let tail = references.split_off(references.len() - 19);
        references.truncate(1);
        references.extend(tail);
    }
    let body = if s.prefs.quote_on_reply || mode == ComposeMode::Forward { quoted(s, &m) } else { String::new() };
    let c = Compose {
        mode,
        source: Some(m.id),
        account: m.account,
        to: format_list(&to),
        cc: format_list(&cc),
        subject: prefixed(if mode == ComposeMode::Forward { "FW" } else { "RE" }, &m.subject),
        body: format!("{}{body}", signature_block(s, m.account, None)),
        attachments: if mode == ComposeMode::Forward { m.attachments.clone() } else { Vec::new() },
        in_reply_to: if mode == ComposeMode::Forward { String::new() } else { m.message_id.clone() },
        references: if mode == ComposeMode::Forward { Vec::new() } else { references },
        ..Default::default()
    };
    s.compose = Some(c);
    Ok(json!({"composing": true, "mode": mode}))
}

fn reply(s: &mut Session, v: &Value) -> CmdResult {
    respond(s, v, ComposeMode::Reply)
}

fn reply_all(s: &mut Session, v: &Value) -> CmdResult {
    respond(s, v, ComposeMode::ReplyAll)
}

fn forward(s: &mut Session, v: &Value) -> CmdResult {
    respond(s, v, ComposeMode::Forward)
}

fn edit_draft(s: &mut Session, v: &Value) -> CmdResult {
    let m = source_message(s, v)?;
    if !m.draft {
        return Err(fail("that message isn't a draft"));
    }
    s.compose = Some(Compose {
        mode: ComposeMode::Draft,
        draft: Some(m.id),
        account: m.account,
        to: format_list(&m.to),
        cc: format_list(&m.cc),
        bcc: format_list(&m.bcc),
        subject: m.subject.clone(),
        body: m.body.text.clone(),
        attachments: m.attachments.clone(),
        importance: m.importance,
        in_reply_to: m.in_reply_to.clone(),
        references: m.references.clone(),
        ..Default::default()
    });
    Ok(json!({"composing": true, "draft": m.id}))
}

fn set(s: &mut Session, v: &Value) -> CmdResult {
    let account = p::id(v, "account");
    if let Some(a) = account
        && s.mailbox.account(a).is_none()
    {
        return Err(fail(format!("no account with id {a}")));
    }
    let importance = match p::str(v, "importance") {
        Some(i) => Some(Importance::parse(i).ok_or_else(|| bad("`importance` is low, normal or high"))?),
        None => None,
    };
    let c = compose_mut(s)?;
    let before = c.clone();
    for (key, field) in [("to", &mut c.to), ("cc", &mut c.cc), ("bcc", &mut c.bcc), ("subject", &mut c.subject), ("body", &mut c.body)] {
        if let Some(text) = p::str(v, key) {
            // One line for everything but the body.
            *field = if key == "body" { text.to_string() } else { text.replace(['\r', '\n'], " ") };
        }
    }
    if let Some(i) = importance {
        c.importance = i;
    }
    if let Some(a) = account {
        c.account = a;
    }
    if *c != before {
        c.dirty = true;
    }
    done()
}

fn attach(s: &mut Session, v: &Value) -> CmdResult {
    let (name, bytes) = match p::str(v, "data") {
        Some(data) => (p::req_str(v, "name")?.to_string(), base64_decode(data)?),
        None => source(v)?,
    };
    if bytes.len() > MAX_ATTACHMENT {
        return Err(fail(format!("{name} is larger than {} MB", MAX_ATTACHMENT >> 20)));
    }
    // Only the file's own name travels, never the folders it was in.
    let name = name.rsplit(['/', '\\']).next().unwrap_or("").trim().to_string();
    let name = if name.is_empty() { "attachment".to_string() } else { name };
    let mime = p::str(v, "mime").map(str::to_string).unwrap_or_else(|| mailcraft_mime::media_type_for(&name).to_string());
    let c = compose_mut(s)?;
    if c.attachments.len() >= MAX_ATTACHMENTS {
        return Err(fail("this message already has as many attachments as it can carry"));
    }
    let size = bytes.len();
    c.attachments.push(Attachment::new(name.clone(), mime, bytes));
    c.dirty = true;
    Ok(json!({"name": name, "size": size, "attachments": c.attachments.len()}))
}

fn remove_attachment(s: &mut Session, v: &Value) -> CmdResult {
    let index = p::u64(v, "index").ok_or_else(|| bad("`index` (number) is required"))? as usize;
    let c = compose_mut(s)?;
    if index >= c.attachments.len() {
        return Err(fail(format!("the message has no attachment {index}")));
    }
    let removed = c.attachments.remove(index);
    c.dirty = true;
    Ok(json!({"removed": removed.name}))
}

fn signature(s: &mut Session, v: &Value) -> CmdResult {
    let account = s.compose.as_ref().map_or(0, |c| c.account);
    if let Some(name) = p::str(v, "name")
        && s.mailbox.signature(name).is_none()
    {
        return Err(fail(format!("no signature named \"{name}\"")));
    }
    let block = signature_block(s, account, p::str(v, "name"));
    if block.is_empty() {
        return Err(fail("this account has no signature; add one in File › Options"));
    }
    let c = compose_mut(s)?;
    // Above the quoted original, at the end of what was typed.
    match c.body.find("\n\n-----Original Message-----") {
        Some(at) => c.body.insert_str(at, &block),
        None => c.body.push_str(&block),
    }
    c.dirty = true;
    done()
}

/// Replace names typed on the recipient lines with the contact they stand for, when exactly one
/// contact matches. Answers the names nobody matched.
fn check_names(s: &mut Session, _: &Value) -> CmdResult {
    let c = s.compose.clone().ok_or_else(|| fail("no message is being written"))?;
    let mut unresolved = Vec::new();
    let mut resolve = |line: &str| -> String {
        let list: Vec<Address> = Address::parse_list(line)
            .into_iter()
            .map(|a| {
                if a.is_valid() {
                    return a;
                }
                let typed = a.email.clone();
                let hits: Vec<Address> = s.mailbox.contacts.values().filter(|k| k.matches(&typed)).filter_map(|k| k.address()).collect();
                match hits.as_slice() {
                    [one] => one.clone(),
                    _ => {
                        unresolved.push(typed);
                        a
                    }
                }
            })
            .collect();
        format_list(&list)
    };
    let (to, cc, bcc) = (resolve(&c.to), resolve(&c.cc), resolve(&c.bcc));
    let now = compose_mut(s)?;
    if (&to, &cc, &bcc) != (&now.to, &now.cc, &now.bcc) {
        (now.to, now.cc, now.bcc) = (to, cc, bcc);
        now.dirty = true;
    }
    Ok(json!({"unresolved": unresolved}))
}

fn toggle_importance(s: &mut Session, level: Importance) -> CmdResult {
    let c = compose_mut(s)?;
    c.importance = if c.importance == level { Importance::Normal } else { level };
    c.dirty = true;
    Ok(json!({"importance": c.importance}))
}

fn high_importance(s: &mut Session, _: &Value) -> CmdResult {
    toggle_importance(s, Importance::High)
}

fn low_importance(s: &mut Session, _: &Value) -> CmdResult {
    toggle_importance(s, Importance::Low)
}

/// The message as written so far. `strict`: every recipient must be a deliverable address.
fn build(s: &Session, c: &Compose, strict: bool) -> Result<Message, CmdError> {
    let account = s.mailbox.account(c.account).or(s.mailbox.default_account()).ok_or_else(|| fail("add an account first (File › Add Account)"))?;
    let parse = |line: &str, what: &str| -> Result<Vec<Address>, CmdError> {
        let list = Address::parse_list(line);
        if strict && let Some(bad_one) = list.iter().find(|a| !a.is_valid()) {
            return Err(fail(format!("\"{}\" on the {what} line isn't an e-mail address", bad_one.email)));
        }
        Ok(list)
    };
    let (to, cc, bcc) = (parse(&c.to, "To")?, parse(&c.cc, "Cc")?, parse(&c.bcc, "Bcc")?);
    if strict && to.is_empty() && cc.is_empty() && bcc.is_empty() {
        return Err(fail("add at least one recipient"));
    }
    let now = s.now();
    let domain = account.email.rsplit_once('@').map_or("", |(_, d)| d);
    Ok(Message {
        account: account.id,
        message_id: mailcraft_mime::new_message_id(
            domain,
            now,
            s.rev().wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ c.body.len() as u64 ^ (c.subject.len() as u64) << 32,
        ),
        in_reply_to: c.in_reply_to.clone(),
        references: c.references.clone(),
        from: account.address(),
        to,
        cc,
        bcc,
        subject: c.subject.trim().to_string(),
        date: now,
        received: now,
        body: Body { text: c.body.clone(), html: None },
        attachments: c.attachments.clone(),
        read: true,
        importance: c.importance,
        ..Default::default()
    })
}

fn send(s: &mut Session, _: &Value) -> CmdResult {
    let c = s.compose.clone().ok_or_else(|| fail("no message is being written"))?;
    let mut m = build(s, &c, true)?;
    let account = s.mailbox.account(m.account).cloned().ok_or_else(|| fail("the account is gone"))?;
    // An account with a server queues the message for the next Send/Receive; a local one has
    // nowhere to send it, so it is filed as sent (and delivered to local accounts it names).
    let queued = account.kind == AccountKind::Imap;
    let kind = if queued { FolderKind::Outbox } else { FolderKind::Sent };
    m.folder = s.mailbox.folder_of_kind(account.id, kind).ok_or_else(|| fail(format!("this account has no {} folder", kind.default_name())))?;
    let id = s.mailbox.add_message(m.clone())?;
    if let Some(src) = c.source
        && let Ok(orig) = s.mailbox.message_mut(src)
    {
        match c.mode {
            ComposeMode::Forward => orig.forwarded = true,
            ComposeMode::Reply | ComposeMode::ReplyAll => orig.answered = true,
            _ => {}
        }
    }
    if let Some(d) = c.draft {
        // The draft became this message.
        let _ = s.mailbox.remove_message(d);
    }
    let mut delivered = 0;
    if !queued {
        let locals: Vec<(Id, String)> = s
            .mailbox
            .accounts
            .iter()
            .filter(|a| a.kind == AccountKind::Local && !a.email.is_empty())
            .map(|a| (a.id, a.email.to_lowercase()))
            .collect();
        for (local, email) in locals {
            if !m.recipients().any(|r| r.email.to_lowercase() == email) {
                continue;
            }
            let Some(inbox) = s.mailbox.folder_of_kind(local, FolderKind::Inbox) else { continue };
            let copy = Message { id: 0, folder: inbox, read: false, bcc: Vec::new(), ..m.clone() };
            let cid = s.mailbox.add_message(copy)?;
            s.mailbox.apply_rules(cid);
            delivered += 1;
        }
    }
    s.compose = None;
    Ok(json!({"id": id, "queued": queued, "delivered": delivered}))
}

fn save_draft(s: &mut Session, _: &Value) -> CmdResult {
    let c = s.compose.clone().ok_or_else(|| fail("no message is being written"))?;
    let mut m = build(s, &c, false)?;
    m.draft = true;
    m.folder = s.mailbox.folder_of_kind(m.account, FolderKind::Drafts).ok_or_else(|| fail("this account has no Drafts folder"))?;
    let id = match c.draft.filter(|d| s.mailbox.message(*d).is_some()) {
        Some(d) => {
            m.id = d;
            *s.mailbox.message_mut(d)? = m;
            d
        }
        None => s.mailbox.add_message(m)?,
    };
    let c = compose_mut(s)?;
    c.draft = Some(id);
    c.dirty = false;
    Ok(json!({"draft": id}))
}

fn close(s: &mut Session, _: &Value) -> CmdResult {
    s.compose = None;
    done()
}

fn discard(s: &mut Session, _: &Value) -> CmdResult {
    let draft = s.compose.take().and_then(|c| c.draft);
    if let Some(d) = draft {
        // Already gone is fine.
        let _ = s.mailbox.remove_message(d);
    }
    Ok(json!({"draftDeleted": draft.is_some()}))
}

/// Someone a name or address being typed could be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub name: String,
    pub email: String,
    /// From the contacts, rather than from mail that was exchanged.
    pub contact: bool,
}

/// Who `query` (the start of a name, of a word in it, or of an address) could be: contacts first,
/// then the people mail was exchanged with, the latest first. The owner's own addresses and
/// addresses that don't look like one are left out. One pass over the mailbox, so it can run on
/// every key press.
pub fn suggest(s: &Session, query: &str, limit: usize) -> Vec<Suggestion> {
    let q = query.trim().to_lowercase();
    if q.is_empty() || q.chars().count() > 200 {
        return Vec::new();
    }
    let limit = limit.clamp(1, 20);
    let fits = |name: &str, email: &str| {
        let (name, email) = (name.to_lowercase(), email.to_lowercase());
        email.starts_with(&q)
            || name.starts_with(&q)
            || name.split(|c: char| !c.is_alphanumeric()).any(|word| !word.is_empty() && word.starts_with(&q))
    };
    let own: Vec<String> = s.mailbox.accounts.iter().map(|a| a.email.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
    let mut seen: std::collections::HashSet<String> = own.into_iter().collect();
    let mut out: Vec<Suggestion> = Vec::new();
    let mut people: Vec<&std::sync::Arc<mailcraft_model::Contact>> = s.mailbox.contacts.values().collect();
    people.sort_by_cached_key(|c| (!c.favorite, c.name().to_lowercase()));
    for c in people {
        let name = c.name();
        for e in &c.emails {
            let email = e.value.trim();
            if mailcraft_model::Address::new("", email).is_valid() && fits(&name, email) && seen.insert(email.to_lowercase()) {
                out.push(Suggestion { name: name.clone(), email: email.to_string(), contact: true });
            }
        }
    }
    if out.len() < limit {
        // Whoever wrote, or was written to: each address once, as it was last seen.
        let mut recent: std::collections::HashMap<String, (i64, &mailcraft_model::Address)> = std::collections::HashMap::new();
        for m in s.mailbox.messages.values() {
            for a in std::iter::once(&m.from).chain(&m.to).chain(&m.cc) {
                let key = a.email.trim().to_lowercase();
                if !seen.contains(&key) && a.is_valid() && fits(&a.name, &a.email) {
                    let known = recent.entry(key).or_insert((m.date, a));
                    if m.date >= known.0 {
                        *known = (m.date, a);
                    }
                }
            }
        }
        let mut recent: Vec<(i64, &mailcraft_model::Address)> = recent.into_values().collect();
        recent.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.email.cmp(&b.1.email)));
        out.extend(recent.into_iter().map(|(_, a)| Suggestion {
            name: a.name.trim().to_string(),
            email: a.email.trim().to_string(),
            contact: false,
        }));
    }
    out.truncate(limit);
    out
}

fn suggest_command(s: &mut Session, v: &Value) -> CmdResult {
    let query = p::req_str(v, "query")?;
    let limit = p::u64(v, "limit").unwrap_or(6).clamp(1, 20) as usize;
    let found: Vec<Value> = suggest(s, query, limit).into_iter().map(|x| json!({"name": x.name, "email": x.email, "contact": x.contact})).collect();
    Ok(json!({"suggestions": found}))
}

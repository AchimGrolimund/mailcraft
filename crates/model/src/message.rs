//! A mail message.

use std::sync::Arc;

use mailcraft_time::Timestamp;
use serde::{Deserialize, Serialize};

use crate::sync::{MovingOut, RemoteFlags, ServerMessage};
use crate::{Address, Id};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Importance {
    Low,
    #[default]
    Normal,
    High,
}

impl Importance {
    pub fn parse(s: &str) -> Option<Importance> {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" | "5" | "non-urgent" => Some(Importance::Low),
            "normal" | "3" | "medium" => Some(Importance::Normal),
            "high" | "1" | "urgent" => Some(Importance::High),
            _ => None,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Importance::Low => "low",
            Importance::Normal => "normal",
            Importance::High => "high",
        }
    }
}

/// Follow-up flag.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FlagStatus {
    #[default]
    None,
    Flagged,
    Complete,
}

/// A file carried by a message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Attachment {
    pub name: String,
    /// Media type (`application/pdf`).
    pub mime: String,
    /// `Content-ID` without angle brackets, for pictures the HTML body shows (`cid:`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content_id: String,
    /// Shown inside the body rather than in the attachment list.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub inline: bool,
    #[serde(with = "crate::b64")]
    pub data: Arc<Vec<u8>>,
}

impl Attachment {
    pub fn new(name: impl Into<String>, mime: impl Into<String>, data: Vec<u8>) -> Attachment {
        Attachment { name: name.into(), mime: mime.into(), content_id: String::new(), inline: false, data: Arc::new(data) }
    }

    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// `12 KB`, `3.4 MB`.
    pub fn size_label(&self) -> String {
        size_label(self.size() as u64)
    }
}

/// A byte count for people.
pub fn size_label(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{} KB", bytes.div_ceil(1024))
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

/// A message body. `text` is always there (readers derive it from the HTML when a message has no
/// plain part); `html` is the original HTML when the message has one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Body {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub html: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Message {
    pub id: Id,
    pub account: Id,
    pub folder: Id,
    /// `Message-ID` without angle brackets.
    pub message_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub in_reply_to: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<String>,
    pub from: Address,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub reply_to: Vec<Address>,
    pub to: Vec<Address>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cc: Vec<Address>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub bcc: Vec<Address>,
    pub subject: String,
    /// When it was sent (`Date:`).
    pub date: Timestamp,
    /// When it arrived here.
    pub received: Timestamp,
    pub body: Body,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<Attachment>,
    pub read: bool,
    pub flag: FlagStatus,
    /// Follow-up due date of a flagged message.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flag_due: Option<Timestamp>,
    pub importance: Importance,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    pub answered: bool,
    pub forwarded: bool,
    pub draft: bool,
    pub pinned: bool,
    /// Headers MailCraft doesn't model, kept so a saved message round-trips.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub headers: Vec<(String, String)>,
    /// The message's UID in the server mailbox of the folder it is in (IMAP); 0 for a message
    /// that isn't on the server there (yet).
    #[serde(skip_serializing_if = "is_zero")]
    pub uid: u32,
    /// The message's flags as the server last had them under `uid`: as last seen there, or as
    /// last pushed. A flag that differs was changed here and is still to be pushed; one that
    /// doesn't follows the server. `None`: the server hasn't been asked about this UID yet.
    ///
    /// With `uid` 0 it marks a message that is in its folder's server mailbox without its UID
    /// there being known (it was just sent, or moved by a server that didn't say where to): the
    /// copy that comes down is matched to it by its Message-ID instead of being added.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "crate::sync::lenient")]
    pub synced: Option<RemoteFlags>,
    /// For a message of a Microsoft account that the server has: its server id and what the
    /// server last said of it ([`ServerMessage`], ADR 0003). `None` for a message the server
    /// hasn't got (or hasn't said so yet), and for every message of any other account; `uid` and
    /// `synced` are IMAP's and stay as they are.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "crate::sync::lenient")]
    pub server: Option<ServerMessage>,
    /// Only its headers and a preview are here: it is not **fetched** yet. Every
    /// message of a file from before this was kept is whole.
    #[serde(skip_serializing_if = "is_false")]
    pub unfetched: bool,
    /// It was moved here, out of a Microsoft account (into a local folder or another account),
    /// from the server's copy this names, and that copy is still on the server: it is **being
    /// moved** (R3 spec S19). The server's copy is removed only once this message is fetched
    /// whole and the mailbox file holding it is saved. `None` for every other message.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "crate::sync::lenient")]
    pub moving_out: Option<MovingOut>,
    /// The server's copy was gone before it could be fetched: only the headers and preview it
    /// had are here, kept as they are, and nothing will fetch the rest (R3 spec S19).
    #[serde(skip_serializing_if = "is_false")]
    pub incomplete: bool,
    /// For a draft of a Microsoft account: its marker on the server, what the server last had
    /// of it, and whether it waits to go up (R3.4.1). `None` for every other message.
    #[serde(skip_serializing_if = "Option::is_none", deserialize_with = "crate::sync::lenient")]
    pub server_draft: Option<crate::sync::ServerDraft>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

impl Message {
    /// Every recipient: To, Cc, Bcc.
    pub fn recipients(&self) -> impl Iterator<Item = &Address> {
        self.to.iter().chain(&self.cc).chain(&self.bcc)
    }

    /// Whether the attachment list has something.
    pub fn has_attachments(&self) -> bool {
        self.listed_attachments().next().is_some()
    }

    /// The attachments the attachment list shows, with their index: everything except pictures
    /// the HTML body shows in place (`cid:` references). A file merely marked "inline" by the
    /// sending program, which nothing in the body refers to, is still listed.
    pub fn listed_attachments(&self) -> impl Iterator<Item = (usize, &Attachment)> {
        let html = self.body.html.as_deref().unwrap_or("");
        self.attachments
            .iter()
            .enumerate()
            .filter(move |(_, a)| !(a.inline && !a.content_id.is_empty() && html.contains(&format!("cid:{}", a.content_id))))
    }

    /// The subject without `Re:` / `Fw:` prefixes: what a conversation is about.
    pub fn topic(&self) -> String {
        normalize_subject(&self.subject)
    }

    /// The key messages of one conversation share (the topic, case-folded); a message without a
    /// subject is its own conversation.
    pub fn thread_key(&self) -> String {
        let t = self.topic().to_lowercase();
        if t.is_empty() { format!("\u{0}{}", self.id) } else { t }
    }

    /// Size in bytes, roughly: text, HTML and attachments.
    pub fn size(&self) -> u64 {
        (self.body.text.len() + self.body.html.as_ref().map_or(0, String::len) + self.attachments.iter().map(Attachment::size).sum::<usize>()) as u64
    }

    pub fn has_category(&self, name: &str) -> bool {
        self.categories.iter().any(|c| c.eq_ignore_ascii_case(name))
    }
}

/// Strip reply and forward prefixes (`Re:`, `RE:`, `Fw:`, `Fwd:`, `AW:`, `WG:`, `Re[2]:`), however
/// many there are.
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim();
    while let Some((prefix, rest)) = s.split_once(':') {
        let p = prefix.trim().to_ascii_lowercase();
        let p = p.split('[').next().unwrap_or("").trim();
        if matches!(p, "re" | "fw" | "fwd" | "aw" | "wg" | "sv" | "vs" | "tr" | "rv") {
            s = rest.trim();
        } else {
            break;
        }
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subjects_normalize() {
        assert_eq!(normalize_subject("Re: RE: Fwd: Lunch on Friday"), "Lunch on Friday");
        assert_eq!(normalize_subject("Re[2]: AW: Plan"), "Plan");
        assert_eq!(normalize_subject("Reminder: dentist"), "Reminder: dentist");
        assert_eq!(normalize_subject("  re:  "), "");
        assert_eq!(normalize_subject("Agenda 10:30"), "Agenda 10:30");
    }

    #[test]
    fn threads_and_sizes() {
        let mut a = Message { id: 1, subject: "Budget".into(), ..Default::default() };
        let b = Message { id: 2, subject: "RE: budget".into(), ..Default::default() };
        assert_eq!(a.thread_key(), b.thread_key());
        a.subject.clear();
        assert_ne!(a.thread_key(), Message { id: 3, ..Default::default() }.thread_key());
        assert_eq!(size_label(900), "900 bytes");
        assert_eq!(size_label(1025), "2 KB");
        assert_eq!(size_label(5 * 1024 * 1024 / 2), "2.5 MB");
        assert_eq!(Importance::parse("HIGH"), Some(Importance::High));
    }

    #[test]
    fn inline_files_nothing_refers_to_are_still_listed() {
        let picture = Attachment { content_id: "logo@x".into(), inline: true, ..Attachment::new("logo.png", "image/png", vec![1]) };
        let report = Attachment { inline: true, ..Attachment::new("report.pdf", "application/pdf", vec![2]) };
        let mut m = Message { attachments: vec![picture, report], ..Default::default() };
        m.body.html = Some("<p>Hello <img src=\"cid:logo@x\"></p>".into());
        assert_eq!(m.listed_attachments().map(|(i, a)| (i, a.name.as_str())).collect::<Vec<_>>(), [(1, "report.pdf")]);
        // Without a body that shows it, the picture is a file like any other.
        m.body.html = None;
        assert_eq!(m.listed_attachments().count(), 2);
        assert!(m.has_attachments() && !Message::default().has_attachments());
    }

    #[test]
    fn attachments_round_trip_as_base64() {
        let m = Message { attachments: vec![Attachment::new("a.bin", "application/octet-stream", vec![0, 255, 7])], ..Default::default() };
        let json = serde_json::to_string(&m).unwrap();
        assert!(json.contains("\"data\":\"AP8H\""), "{json}");
        assert_eq!(serde_json::from_str::<Message>(&json).unwrap(), m);
        assert!(serde_json::from_str::<Attachment>(r#"{"name":"x","data":"***"}"#).is_err());
    }
}

//! Pictures from the web: whether a message may show them, asking the host for them, and what
//! the owner is told and can do about it.
//!
//! A picture on the web tells whoever serves it that the message was opened, when, and from
//! what address. So none is asked for unless the owner said so, in one of three ways: "Download
//! pictures" for the message at hand (it holds until the program closes), "Always download
//! pictures from" its sender (remembered in the settings), or the switch in File › Options that
//! downloads them for all mail (off unless the owner turns it on). The last two do not hold for
//! what is in Junk E-mail.
//!
//! This crate opens no connections. It hands an address to the host ([`Services::pictures`])
//! and takes back bytes or a refusal; the host is the one that checks where an address leads
//! before it goes there. Where there is no such host (the browser, the tests) nothing is offered.
//!
//! What is asked of the host is bounded here: a few pictures at a time, so many for one message
//! and so many bytes, each address once (a picture that failed is not tried again), and nothing
//! for a message that is not on screen. The bytes that come are kept in memory with the other
//! pictures ([`Pictures`]), never on disk, and are looked at as warily as an attachment's.
//!
//! [`Services::pictures`]: crate::Services::pictures

use std::collections::VecDeque;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Sense, Ui, vec2};
use mailcraft_model::{FolderKind, Id, Message};
use serde_json::{Value, json};

use super::Place;
use super::images::{self, Pictures, Remote};
use super::view::Doc;
use crate::theme::{Tokens, regular};
use crate::{MailApp, UiState, icons};

/// Most pictures asked for on behalf of one message while the program runs; the rest stay
/// placeholders.
pub const MAX_PER_MESSAGE: usize = 60;
/// Most bytes taken for one message's pictures; once that many have come, no more are asked for.
pub const MAX_MESSAGE_BYTES: usize = 24 * 1024 * 1024;
/// Largest picture taken from the host, in bytes (the host has the same limit of its own).
pub const MAX_BYTES: usize = 8 * 1024 * 1024;
/// How many pictures are on their way at a time.
pub const AT_A_TIME: usize = 4;
/// How long the host is waited on for one picture, in milliseconds (it gives up sooner by itself).
const PATIENCE_MS: f64 = 45_000.0;
/// Most senders whose pictures are always downloaded.
pub const MAX_SENDERS: usize = 500;
/// Longest address of a sender that is remembered.
const MAX_SENDER_LEN: usize = 320;
/// Most messages the owner asked pictures for that are kept track of.
const MAX_ASKED: usize = 500;
/// Most messages whose share of the downloading is kept track of.
const MAX_SPENT: usize = 200;
/// Most addresses waiting for a turn.
const MAX_WAITING: usize = 4 * MAX_PER_MESSAGE;

/// Whether the pictures a message refers to on the web may be shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    /// This build has no way to download anything.
    Unavailable,
    /// It could, and the owner has not asked.
    Blocked,
    /// The owner asked: for this message, for its sender, or for all mail.
    Allowed,
}

/// A message, told from any other that might come to have its number (another mailbox opened).
type Mark = (Id, i64, u64);

fn mark(m: &Message) -> Mark {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (&m.message_id, &m.from.email, &m.subject).hash(&mut hasher);
    (m.id, m.date, hasher.finish())
}

/// A picture the host is downloading.
struct Flight {
    message: Id,
    url: Arc<str>,
    answer: Receiver<Result<Vec<u8>, String>>,
    /// When it was asked for (milliseconds).
    since: f64,
}

/// What downloading for one message has come to.
#[derive(Clone, Copy, Debug, Default)]
struct Spent {
    asked: usize,
    bytes: usize,
}

impl Spent {
    fn out(self) -> bool {
        self.asked >= MAX_PER_MESSAGE || self.bytes >= MAX_MESSAGE_BYTES
    }
}

/// How a message's pictures are coming along.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Progress {
    /// Asked for or about to be.
    pub waiting: usize,
    pub failed: usize,
    /// Not asked for: the message has more pictures, or heavier ones, than are downloaded for one.
    pub left_out: usize,
}

/// The state of downloading pictures, for as long as the program runs.
#[derive(Default)]
pub struct RemotePictures {
    /// Messages the owner said "Download pictures" for.
    asked: VecDeque<Mark>,
    /// Addresses that messages on screen are waiting for. Gathered while they are laid out and
    /// taken up at the next frame: what is no longer on screen is no longer asked for.
    waiting: Vec<(Id, Arc<str>)>,
    flying: Vec<Flight>,
    spent: VecDeque<(Id, Spent)>,
    /// The frame arrivals were last looked for in.
    ticked: Option<u64>,
}

impl RemotePictures {
    fn spent(&self, message: Id) -> Spent {
        self.spent.iter().find(|(id, _)| *id == message).map(|(_, spent)| *spent).unwrap_or_default()
    }

    fn spend(&mut self, message: Id, asked: usize, bytes: usize) {
        match self.spent.iter_mut().find(|(id, _)| *id == message) {
            Some((_, spent)) => {
                spent.asked = spent.asked.saturating_add(asked);
                spent.bytes = spent.bytes.saturating_add(bytes);
            }
            None => {
                if self.spent.len() >= MAX_SPENT {
                    self.spent.pop_front();
                }
                self.spent.push_back((message, Spent { asked, bytes }));
            }
        }
    }

    /// How many pictures are on their way.
    pub fn flying(&self) -> usize {
        self.flying.len()
    }

    /// Pretend the pictures on their way were asked for `ms` milliseconds earlier.
    #[cfg(test)]
    pub fn age(&mut self, ms: f64) {
        for flight in &mut self.flying {
            flight.since -= ms;
        }
    }
}

/// A sender's address as it is remembered; `None` for what is no address.
fn sender(text: &str) -> Option<String> {
    let text = text.trim();
    let plain = text.len() <= MAX_SENDER_LEN && text.contains('@') && !text.chars().any(|c| c.is_whitespace() || c.is_control());
    (plain && text.len() >= 3).then(|| text.to_lowercase())
}

/// Whether pictures from `address` are always downloaded.
pub fn sender_remembered(ui: &UiState, address: &str) -> bool {
    sender(address).is_some_and(|address| ui.picture_senders.iter().take(MAX_SENDERS).any(|kept| kept.trim().eq_ignore_ascii_case(&address)))
}

/// Whether `m` is in an account's Junk E-mail folder.
fn in_junk(app: &MailApp, m: &Message) -> bool {
    app.session.mailbox.folder(m.folder).is_some_and(|f| f.kind == FolderKind::Junk)
}

/// Whether `m` may show the pictures it refers to on the web.
pub fn access(app: &MailApp, m: &Message) -> Access {
    if app.services.pictures.is_none() {
        return Access::Unavailable;
    }
    // What lands in Junk E-mail is trusted by nobody's say-so but the owner's, message by message.
    let trusted = !in_junk(app, m) && (app.ui.download_pictures || sender_remembered(&app.ui, &m.from.email));
    if trusted || app.remote_pictures.asked.contains(&mark(m)) { Access::Allowed } else { Access::Blocked }
}

/// Take in the pictures the host has brought, and hand it the next ones that messages on
/// screen wait for. Once a frame, wherever it is called from.
pub fn tick(app: &mut MailApp, ctx: &egui::Context) {
    let frame = images::frame(ctx);
    let state = &mut app.remote_pictures;
    if state.ticked == Some(frame) {
        return;
    }
    state.ticked = Some(frame);
    let now = crate::now_ms();
    let mut landed: Vec<(Id, Arc<str>, Option<Vec<u8>>)> = Vec::new();
    state.flying.retain(|flight| {
        let answer = match flight.answer.try_recv() {
            Ok(answer) => answer.map_err(|why| log::debug!("a picture was not downloaded: {}", why.chars().take(200).collect::<String>())).ok(),
            // The host went away, or is taking longer than anything should.
            Err(TryRecvError::Disconnected) => None,
            Err(TryRecvError::Empty) if now - flight.since > PATIENCE_MS => None,
            Err(TryRecvError::Empty) => return true,
        };
        landed.push((flight.message, flight.url.clone(), answer));
        false
    });
    if !landed.is_empty() {
        let pictures = Pictures::of(ctx);
        let mut pictures = pictures.lock().unwrap_or_else(|e| e.into_inner());
        for (message, url, bytes) in landed {
            state.spend(message, 0, bytes.as_ref().map_or(0, Vec::len));
            // Whatever came is a stranger's bytes: a picture within the limits, or nothing.
            let picture = bytes.filter(|bytes| bytes.len() <= MAX_BYTES && images::probe(bytes).is_some());
            pictures.set_remote(url, picture.map_or(Remote::Failed, |bytes| Remote::Ready(Arc::new(bytes))), frame);
        }
        ctx.request_repaint();
    }
    // What was wanted a frame ago is asked for now, if its message may still show it: a say-so
    // that was taken back in between is taken back for these too.
    let waiting = std::mem::take(&mut state.waiting);
    let allowed = |message: &Id| app.session.mailbox.messages.get(message).is_some_and(|m| access(app, m) == Access::Allowed);
    let waiting: Vec<(Id, Arc<str>)> = waiting.into_iter().filter(|(message, _)| allowed(message)).collect();
    let state = &mut app.remote_pictures;
    if let Some(download) = &app.services.pictures
        && !waiting.is_empty()
    {
        let pictures = Pictures::of(ctx);
        let pictures = pictures.lock().unwrap_or_else(|e| e.into_inner());
        for (message, url) in waiting {
            if state.flying.len() >= AT_A_TIME {
                // The rest is asked for again by its message, for as long as that is on screen.
                break;
            }
            // Each address once: not one that is on its way, nor one whose fate is known.
            if state.flying.iter().any(|flight| flight.url == url) || pictures.remote_peek(&url).is_some() || state.spent(message).out() {
                continue;
            }
            state.spend(message, 1, 0);
            let answer = download(url.to_string());
            state.flying.push(Flight { message, url, answer, since: now });
        }
    }
    if !state.flying.is_empty() {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}

/// `m` is on screen and may show its pictures: note which of them are still to come, so that
/// they are asked for, and say how they are coming along.
pub fn want(app: &mut MailApp, ctx: &egui::Context, m: &Message, doc: &Doc) -> Progress {
    let mut progress = Progress { left_out: doc.remote_left_out, ..Progress::default() };
    if doc.remote.is_empty() {
        return progress;
    }
    let frame = images::frame(ctx);
    let state = &mut app.remote_pictures;
    let out = state.spent(m.id).out();
    let pictures = Pictures::of(ctx);
    let mut pictures = pictures.lock().unwrap_or_else(|e| e.into_inner());
    for url in &doc.remote {
        match pictures.remote(url, frame) {
            Some(Remote::Ready(_)) => {}
            Some(Remote::Failed) => progress.failed += 1,
            None if state.flying.iter().any(|flight| flight.url == *url) => progress.waiting += 1,
            None if out => progress.left_out += 1,
            None => {
                progress.waiting += 1;
                if state.waiting.len() < MAX_WAITING && !state.waiting.iter().any(|(_, waiting)| waiting == url) {
                    state.waiting.push((m.id, url.clone()));
                }
            }
        }
    }
    if progress.waiting > 0 {
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
    progress
}

/// The line above a message that has pictures on the web: that they are not shown and why, with
/// what the owner can do about it; or how downloading them is going.
pub fn line(app: &mut MailApp, ui: &mut Ui, m: &Message, doc: &Doc, access: Access, progress: Progress, place: Place, t: &Tokens) {
    if doc.remote.is_empty() {
        return;
    }
    let text = match access {
        Access::Unavailable => tl!("This message has pictures on the web. This build can't download them.").to_string(),
        Access::Blocked => tl!("Pictures in this message aren't downloaded, to protect your privacy.").to_string(),
        Access::Allowed if progress.waiting > 0 => tl!("Downloading pictures…").to_string(),
        Access::Allowed => match progress.failed + progress.left_out {
            0 => return,
            1 => tl!("A picture couldn't be downloaded.").to_string(),
            n => crate::i18n::fmt(tl!("{n} pictures couldn't be downloaded."), &[("n", &n.to_string())]),
        },
    };
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 2.0);
        let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
        icons::paint(ui.painter(), r.shrink(1.0), "private", t.text_dim, t.accent);
        ui.add(egui::Label::new(egui::RichText::new(text).font(regular(11.5)).color(t.text_dim)).wrap());
        if access != Access::Blocked {
            return;
        }
        // What can be done about it: each a few words that stay together, on the next line when
        // this one is full.
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        let action =
            |ui: &mut Ui, label: String, tip: &str| ui.add(egui::Link::new(egui::RichText::new(label).font(regular(11.5)))).on_hover_text(tip);
        let download =
            action(ui, tl!("Download pictures").to_string(), tl!("Show this message's pictures. Whoever sent it can then tell that you opened it."));
        if place == Place::Pane {
            app.hit.add("downloadPictures", m.id, download.rect);
        }
        if download.clicked() {
            let _ = app.run("ui.downloadPictures", json!({"message": m.id}));
        }
        // Remembering a sender does nothing for what is in Junk E-mail, so it is not offered there.
        let Some(from) = sender(&m.from.email).filter(|_| !in_junk(app, m)) else { return };
        let shown: String = if from.chars().count() > 60 { from.chars().take(59).chain(['…']).collect() } else { from.clone() };
        let label = crate::i18n::fmt(tl!("Always download pictures from {sender}"), &[("sender", &shown)]);
        let always = action(ui, label, tl!("Remembered in the settings; File › Options lists the senders and takes them off again."));
        if place == Place::Pane {
            app.hit.add("alwaysDownloadPictures", m.id, always.rect);
        }
        if always.clicked() {
            let _ = app.run("ui.alwaysDownloadPictures", json!({"sender": from}));
        }
    });
}

/// The rows of File › Options: the switch for all mail, and the senders that are remembered.
pub fn options(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let mut all = app.ui.download_pictures;
    let tip = tl!(
        "A picture on the web tells its sender that you opened the message, and when. Left off, pictures are downloaded only for the messages and senders you say."
    );
    if ui.checkbox(&mut all, tl!("Download pictures in mail automatically")).on_hover_text(tip).changed() {
        let _ = app.run("ui.downloadPicturesAutomatically", json!({"value": all}));
    }
    if app.services.pictures.is_none() {
        ui.label(egui::RichText::new(tl!("This build can't download pictures: they stay placeholders.")).color(t.text_dim));
    }
    let senders: Vec<String> = app.ui.picture_senders.iter().take(MAX_SENDERS).map(|s| s.chars().take(MAX_SENDER_LEN).collect()).collect();
    if senders.is_empty() {
        let none = tl!("No senders are remembered. \"Always download pictures from\" above a message adds its sender here.");
        ui.label(egui::RichText::new(none).color(t.text_dim));
        return;
    }
    ui.label(tl!("Pictures are always downloaded from:"));
    for address in senders {
        ui.horizontal(|ui| {
            if ui.small_button(tl!("Remove")).on_hover_text(tl!("Ask before downloading this sender's pictures again")).clicked() {
                let _ = app.run("ui.alwaysDownloadPictures", json!({"sender": address, "value": false}));
            }
            ui.add(egui::Label::new(address.as_str()).truncate());
        });
    }
}

/// The message a command is about: the one it names, else the one being read.
fn subject(app: &MailApp, p: &Value) -> Result<Arc<Message>, String> {
    let id = match p.get("message") {
        None | Some(Value::Null) => app.session.sel.focus().ok_or_else(|| tl!("Select a message first").to_string())?,
        Some(id) => id.as_u64().ok_or_else(|| "`message` is a message's id".to_string())?,
    };
    app.session.mailbox.messages.get(&id).cloned().ok_or_else(|| format!("no message with id {id}"))
}

/// The window commands that are about pictures (`UI_COMMANDS` lists them).
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        "ui.downloadPictures" => download(app, p),
        "ui.alwaysDownloadPictures" => always(app, p),
        "ui.downloadPicturesAutomatically" => {
            let on = p.get("value").and_then(Value::as_bool).unwrap_or(!app.ui.download_pictures);
            app.ui.download_pictures = on;
            Ok(json!({"value": on}))
        }
        // The other commands of a message that is being read: its selected text, and the
        // meeting it carries.
        _ => return super::select::ui_command(app, id, p).or_else(|| super::invitation::ui_command(app, id, p)),
    })
}

/// "Download pictures": this message's, from now until the program closes.
fn download(app: &mut MailApp, p: &Value) -> Result<Value, String> {
    let m = subject(app, p)?;
    if app.services.pictures.is_none() {
        return Err(tl!("This build can't download pictures").to_string());
    }
    let mark = mark(&m);
    let asked = &mut app.remote_pictures.asked;
    if !asked.contains(&mark) {
        if asked.len() >= MAX_ASKED {
            asked.pop_front();
        }
        asked.push_back(mark);
    }
    Ok(json!({"message": m.id}))
}

/// "Always download pictures from": remember a sender, or forget one.
fn always(app: &mut MailApp, p: &Value) -> Result<Value, String> {
    let named = match p.get("sender") {
        None | Some(Value::Null) => subject(app, p)?.from.email.clone(),
        Some(text) => text.as_str().ok_or_else(|| "`sender` is an address".to_string())?.to_string(),
    };
    let Some(address) = sender(&named) else { return Err(tl!("That is not a sender's address").to_string()) };
    let on = p.get("value").and_then(Value::as_bool).unwrap_or(true);
    let kept = &mut app.ui.picture_senders;
    kept.retain(|other| !other.trim().eq_ignore_ascii_case(&address));
    kept.truncate(MAX_SENDERS);
    if on {
        if kept.len() >= MAX_SENDERS {
            return Err(tl!("The list of senders is full: remove some in File › Options").to_string());
        }
        kept.push(address.clone());
    }
    Ok(json!({"sender": address, "value": on, "senders": kept.len()}))
}

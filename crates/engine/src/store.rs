//! The mailbox on disk, kept so that saving it costs what changed rather than what it holds.
//!
//! One self-contained file ([`crate::io::write_mailbox`]) carries every attachment as base64 and is
//! written whole each time: right for an export, hopeless for years of mail that change with every
//! message read. The mailbox a program works on is kept in two parts instead:
//!
//! ```text
//! mailbox.mailcraft           the mailbox without its content (JSON, format 2): accounts, folders,
//!                             every message's headers and flags, contacts, events, tasks, and under
//!                             "blobs" the content each message keeps outside: [SHA-256, length]
//! mailbox.mailcraft.blobs/    the content, one file per distinct body or attachment
//!     3f/3f2a…                named by the SHA-256 of its bytes (64 lowercase hex digits), in a
//!                             folder named by the first two
//! ```
//!
//! - **What moves out:** a body or an attachment of more than [`INLINE_MAX`] bytes. A content file
//!   is written once and never changed, so a save after a message was read writes the mailbox file
//!   and nothing else. The same file attached to ten messages is kept once (and shared in memory).
//! - **Saving** ([`save`]) writes the content files that aren't there yet, then the mailbox file,
//!   each to a temporary file beside it that is flushed and renamed over. A crash at any point
//!   leaves the old mailbox file, which names only content that is still there, or the new one.
//!   [`Saver`] does this on a thread of its own for a window that must not wait.
//! - **Opening** ([`open`]) reads one self-contained file as before (it becomes this form the next
//!   time it is saved) or this form. Everything read back is hostile: a content file is taken only
//!   when it is a plain file of the length and SHA-256 the mailbox file names; names are 64 hex
//!   digits or nothing; sizes are capped. Content that is missing or damaged never stops the
//!   mailbox from opening: the message keeps everything else and says its text is unavailable,
//!   and what the mailbox file said about it is written back unchanged, so content that turns up
//!   again (a folder copied late) is found the next time.
//! - **Sweeping** ([`sweep`]) removes content files the mailbox file no longer names, never one
//!   written in the last few minutes: a save in flight has written its content before its mailbox
//!   file names it.
//!
//! Not in the browser, which has no folder to keep content in: there the mailbox stays one file.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, Weak};
use std::time::{Duration, Instant, SystemTime};

use mailcraft_model::{Id, Mailbox, Message};
use serde::Deserialize;

use crate::io::{BackgroundSave, MAX_FILE, SaveReport};

/// A body or an attachment up to this many bytes stays in the mailbox file.
pub const INLINE_MAX: usize = 1024;

/// Largest content file read.
pub const MAX_BLOB: u64 = MAX_FILE;

/// Most content one mailbox brings into memory (8 GB); what is beyond it opens as unavailable.
pub const MAX_CONTENT: u64 = 8 << 30;

/// A content file nothing names is left alone for this long after it was written.
pub const GRACE: Duration = Duration::from_secs(10 * 60);

/// The mailbox file's format number in this form (one self-contained file is format 1).
pub const SPLIT_VERSION: u32 = 2;

/// What a message says instead of text that couldn't be read back.
pub const UNAVAILABLE: &str =
    "[This message's text is unavailable: the file that keeps it is missing from the mailbox's content folder, or damaged.]";

/// Attachments of one message kept outside the mailbox file; any after these stay inside it.
const MAX_PARTS: usize = 4096;

/// Directory entries one sweep looks at.
const MAX_ENTRIES: usize = 4_000_000;

/// Threads reading or writing content files at once.
const MAX_THREADS: usize = 8;

/// How often a running program looks for content nothing names any more.
const SWEEP_EVERY: Duration = Duration::from_secs(6 * 60 * 60);

/// How long [`Saver::flush`] waits for the disk before giving up.
const FLUSH_WAIT: Duration = Duration::from_secs(120);

/// Where the content of the mailbox file at `path` is kept: `<file>.blobs` beside it.
pub fn blob_dir(path: &Path) -> PathBuf {
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".blobs");
    path.with_file_name(name)
}

/// The file a mailbox path stands for: itself, or where it leads when it is a link. A mailbox
/// kept somewhere else through a link stays there, with its content beside it, instead of the
/// link being replaced by a file the first time it is saved.
fn resolve(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
        _ => path.to_path_buf(),
    }
}

// ---- names ----

/// The SHA-256 of a piece of content: its name on disk.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Digest([u8; 32]);

impl Digest {
    pub fn of(bytes: &[u8]) -> Digest {
        let mut out = [0u8; 32];
        for (o, b) in out.iter_mut().zip(ring::digest::digest(&ring::digest::SHA256, bytes).as_ref()) {
            *o = *b;
        }
        Digest(out)
    }

    /// 64 lowercase hex digits.
    pub fn hex(&self) -> String {
        let digit = |n: u8| char::from_digit(u32::from(n), 16).unwrap_or('0');
        self.0.iter().flat_map(|b| [digit(b >> 4), digit(b & 15)]).collect()
    }

    /// From exactly 64 lowercase hex digits. Nothing else names a content file, so nothing read
    /// from a mailbox file can become any other path.
    pub fn parse(text: &str) -> Option<Digest> {
        let digits = text.as_bytes();
        if digits.len() != 64 || !is_hex(text) {
            return None;
        }
        let mut out = [0u8; 32];
        for (o, pair) in out.iter_mut().zip(digits.chunks(2)) {
            let value = |i: usize| pair.get(i).and_then(|c| char::from(*c).to_digit(16));
            *o = (value(0)? * 16 + value(1)?) as u8;
        }
        Some(Digest(out))
    }
}

impl std::fmt::Debug for Digest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.hex())
    }
}

fn is_hex(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

/// A content file as the mailbox file names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct Ref {
    hash: Digest,
    len: u64,
}

impl Ref {
    fn of(bytes: &[u8]) -> Ref {
        Ref { hash: Digest::of(bytes), len: bytes.len() as u64 }
    }
}

/// One part of a message kept outside the mailbox file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Part {
    r: Ref,
    /// Its bytes are in memory. Not so for content that couldn't be read when the mailbox was
    /// opened: that keeps the name it had, and nothing can be written for it.
    held: bool,
}

/// The parts of one message kept outside the mailbox file; attachments by their index.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Parts {
    text: Option<Part>,
    html: Option<Part>,
    attachments: Vec<Option<Part>>,
}

impl Parts {
    fn is_empty(&self) -> bool {
        self.text.is_none() && self.html.is_none() && self.attachments.iter().all(Option::is_none)
    }

    fn all(&self) -> impl Iterator<Item = &Part> {
        self.text.iter().chain(&self.html).chain(self.attachments.iter().flatten())
    }

    /// As the mailbox file has it under `blobs`: `{"text":[hash,length],"attachments":[…,null]}`.
    fn json(&self) -> String {
        let one = |p: &Option<Part>| p.map_or_else(|| "null".to_string(), |p| format!("[\"{}\",{}]", p.r.hash.hex(), p.r.len));
        let mut fields = Vec::new();
        if self.text.is_some() {
            fields.push(format!("\"text\":{}", one(&self.text)));
        }
        if self.html.is_some() {
            fields.push(format!("\"html\":{}", one(&self.html)));
        }
        if !self.attachments.is_empty() {
            fields.push(format!("\"attachments\":[{}]", self.attachments.iter().map(one).collect::<Vec<_>>().join(",")));
        }
        format!("{{{}}}", fields.join(","))
    }

    /// The bytes of the parts that are in memory, out of the message they were worked out from.
    fn content<'a>(&self, m: &'a Message) -> Vec<(Ref, &'a [u8])> {
        let held = |p: &Option<Part>| p.filter(|p| p.held).map(|p| p.r);
        let mut out = Vec::new();
        if let Some(r) = held(&self.text) {
            out.push((r, m.body.text.as_bytes()));
        }
        if let (Some(r), Some(html)) = (held(&self.html), &m.body.html) {
            out.push((r, html.as_bytes()));
        }
        for (p, a) in self.attachments.iter().zip(&m.attachments) {
            if let Some(r) = held(p) {
                out.push((r, a.data.as_slice()));
            }
        }
        out
    }
}

/// `blobs` as a mailbox file has it. Strictly typed: a file that says anything else there isn't
/// one of ours, and guessing would end in content being written back empty.
#[derive(Default, Deserialize)]
#[serde(default)]
struct Head {
    version: u32,
    blobs: BTreeMap<Id, WireParts>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct WireParts {
    text: Option<(String, u64)>,
    html: Option<(String, u64)>,
    #[serde(deserialize_with = "capped")]
    attachments: Vec<Option<(String, u64)>>,
}

/// At most [`MAX_PARTS`] entries of a list; the rest is skipped without being kept.
fn capped<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<Option<(String, u64)>>, D::Error> {
    struct Capped;
    impl<'de> serde::de::Visitor<'de> for Capped {
        type Value = Vec<Option<(String, u64)>>;
        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a list of content names")
        }
        fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while out.len() < MAX_PARTS {
                match seq.next_element()? {
                    Some(item) => out.push(item),
                    None => return Ok(out),
                }
            }
            while seq.next_element::<serde::de::IgnoredAny>()?.is_some() {}
            Ok(out)
        }
    }
    d.deserialize_seq(Capped)
}

fn parse_ref(wire: &(String, u64)) -> Option<Ref> {
    Digest::parse(&wire.0).map(|hash| Ref { hash, len: wire.1 })
}

// ---- what is known about a mailbox file ----

/// A mailbox file as this program last read or wrote it, to notice anybody else writing it.
struct Stamp {
    len: u64,
    modified: Option<SystemTime>,
    digest: Digest,
}

impl Stamp {
    fn of(path: &Path, bytes: &[u8]) -> Option<Stamp> {
        let meta = fs::metadata(path).ok()?;
        (meta.len() == bytes.len() as u64).then(|| Stamp { len: meta.len(), modified: meta.modified().ok(), digest: Digest::of(bytes) })
    }

    /// The file is still the one this was taken of.
    fn still(&self, path: &Path) -> bool {
        fs::metadata(path).is_ok_and(|m| m.len() == self.len && m.modified().ok() == self.modified)
    }
}

/// A message as it was last written: what it comes to in the mailbox file, so that a message
/// nobody touched since costs a lookup, not a hash of its attachments.
struct Cached {
    /// The message this was worked out from. While this is alive the allocation can't be reused
    /// and the message in it can't change (changing a shared message makes a new one), so the
    /// same address is the same message.
    of: Weak<Message>,
    json: Box<[u8]>,
    parts: Parts,
    /// `parts` as JSON; empty when the message keeps nothing outside.
    refs: Box<[u8]>,
}

/// A mailbox as it was opened, to know it again without keeping it: its messages by where they
/// are in memory (a message that changes moves, see [`Cached`]), the rest by its SHA-256.
struct Loaded {
    messages: Vec<(Id, Weak<Message>)>,
    rest: Digest,
}

impl Loaded {
    /// Everything but the messages, as the model writes it.
    fn rest(mailbox: &Mailbox) -> Option<Digest> {
        let mut shell = mailbox.clone();
        shell.messages = BTreeMap::new();
        serde_json::to_vec(&shell).ok().map(|bytes| Digest::of(&bytes))
    }

    fn of(mailbox: &Mailbox) -> Option<Loaded> {
        let messages = mailbox.messages.iter().map(|(id, m)| (*id, Arc::downgrade(m))).collect();
        Some(Loaded { messages, rest: Loaded::rest(mailbox)? })
    }

    fn is(&self, mailbox: &Mailbox) -> bool {
        self.messages.len() == mailbox.messages.len()
            && self.messages.iter().zip(&mailbox.messages).all(|((id, was), (now, m))| id == now && std::ptr::eq(was.as_ptr(), Arc::as_ptr(m)))
            && Loaded::rest(mailbox) == Some(self.rest)
    }
}

/// What the mailbox file said about content of one message that couldn't be read when it was
/// opened, and enough about the message to know it again. Only that message gets those names
/// back when it is saved, and only for what is still as it was opened: a draft written anew, or
/// another mailbox saved to the same file, is new content.
struct Lost {
    parts: Parts,
    message_id: String,
    /// The SHA-256 of the message's text as it was opened.
    text: Digest,
    /// Its attachments' names.
    files: Vec<String>,
}

#[derive(Default)]
struct State {
    /// The content the mailbox file on disk names, as last read or written here. A sweep, by
    /// this program or another, leaves these alone; anything else is written (or its time
    /// renewed) before a mailbox file names it.
    live: HashSet<Digest>,
    stamp: Option<Stamp>,
    /// The mailbox as it was opened, until something else is saved: a mailbox nobody changed is
    /// not rewritten, whatever form its file has.
    loaded: Option<Loaded>,
    cache: HashMap<usize, Cached>,
    /// The SHA-256 of attachments already worked out, by where their bytes are.
    hashes: HashMap<usize, (Weak<Vec<u8>>, Digest)>,
    /// What the mailbox file said about content that couldn't be read when it was opened.
    missing: BTreeMap<Id, Lost>,
    /// The newest state written (see [`next_seq`]): an older one arriving late is dropped.
    newest: u64,
    swept: Option<Instant>,
}

#[derive(Default)]
struct Shared {
    /// A host keeps this file up to date by itself ([`BackgroundSave::follow`]).
    followed: AtomicBool,
    state: Mutex<State>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// What this program knows about the mailbox file at `path`. One per file, so that everybody
/// saving it (a command, the background saver) takes turns.
fn shared(path: &Path) -> Arc<Shared> {
    static ALL: OnceLock<Mutex<HashMap<PathBuf, Arc<Shared>>>> = OnceLock::new();
    let key = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    lock(ALL.get_or_init(Default::default)).entry(key).or_default().clone()
}

/// States of a mailbox are numbered as they are handed over to be written, so that of two saves
/// racing for the same file the later state wins whichever gets to the disk first.
fn next_seq() -> u64 {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    SEQ.fetch_add(1, Ordering::Relaxed) + 1
}

/// Whether a host keeps the mailbox file at `path` up to date by itself.
pub fn followed(path: &Path) -> bool {
    shared(path).followed.load(Ordering::Relaxed)
}

// ---- files ----

/// A name for a temporary file beside `path`.
fn temp_beside(path: &Path) -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let mut name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(format!(".{}-{nanos:x}-{}.tmp", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    path.with_file_name(name)
}

/// Whether `name` is a temporary file [`temp_beside`] made for `stem`.
fn is_temp_of(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem).and_then(|rest| rest.strip_prefix('.')).and_then(|rest| rest.strip_suffix(".tmp")).is_some_and(|middle| {
        let mut parts = middle.split('-');
        let ok = |p: Option<&str>, hex: bool| p.is_some_and(|p| if hex { is_hex(p) } else { !p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()) });
        ok(parts.next(), false) && ok(parts.next(), true) && ok(parts.next(), false) && parts.next().is_none()
    })
}

/// Rename `from` over `to`. On Windows a file somebody has open (a virus scanner, a backup) can't
/// be replaced that instant: try again for a moment before giving up.
fn rename_over(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut tries = 0;
    loop {
        match fs::rename(from, to) {
            Err(e) if cfg!(windows) && e.kind() == std::io::ErrorKind::PermissionDenied && tries < 20 => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(25));
            }
            done => return done,
        }
    }
}

/// Write `bytes` to `path` so that a crash leaves what was there or all of the new file: into a
/// temporary file beside it, flushed to the disk, then renamed over.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut tries = 0;
    let (tmp, mut file) = loop {
        let tmp = temp_beside(path);
        // A new file or nothing: never through something already there by that name.
        let mut open = fs::OpenOptions::new();
        open.write(true).create_new(true);
        // Mail is private: nobody else on the machine reads it.
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
        match open.open(&tmp) {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && tries < 8 => tries += 1,
            Err(e) => return Err(e),
        }
    };
    let written = file.write_all(bytes).and_then(|()| file.sync_all());
    drop(file);
    if let Err(e) = written.and_then(|()| rename_over(&tmp, path)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // The rename itself reaches the disk with its folder (where folders can be flushed).
    #[cfg(unix)]
    {
        let dir = path.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
        if let Ok(dir) = fs::File::open(dir) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}

/// Make the folder `dir` (its parent is there), for its owner only.
fn make_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(dir)
}

/// Make sure `dir` is a folder of its own. Anything else by that name (a link to somewhere else, a
/// file) is removed first, without being followed: content goes into this folder or nowhere.
fn own_dir(dir: &Path) -> std::io::Result<()> {
    let is_dir = |dir: &Path| fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_dir());
    // Looked at once: between two looks another thread saving into the same folder can make it,
    // and taking that for "something else by that name" would remove the folder under its feet.
    match fs::symlink_metadata(dir) {
        Ok(m) if m.file_type().is_dir() => return Ok(()),
        Ok(_) => {
            let _ = fs::remove_file(dir).or_else(|_| fs::remove_dir(dir));
        }
        Err(_) => {}
    }
    match make_dir(dir) {
        // Another thread writing into the same folder got there first.
        Err(_) if is_dir(dir) => Ok(()),
        made => made,
    }
}

/// Where the content named `hash` lives under `dir`.
fn blob_path(dir: &Path, hash: &Digest) -> Option<(PathBuf, PathBuf)> {
    let name = hash.hex();
    let sub = dir.join(name.get(..2)?);
    let file = sub.join(&name);
    Some((sub, file))
}

/// Make sure the content `r` is on disk under `dir`, and safe from a sweep for a while. Returns
/// whether it had to be written. A file of the right length that is already there is this
/// content (nothing else is ever written under its name): only its time is renewed.
fn put(dir: &Path, r: &Ref, bytes: &[u8]) -> std::io::Result<bool> {
    let (sub, file) = blob_path(dir, &r.hash).ok_or_else(|| std::io::Error::other("no name"))?;
    let there = |file: &Path| fs::symlink_metadata(file).is_ok_and(|m| m.file_type().is_file() && m.len() == r.len);
    if there(&file) {
        let renewed = fs::OpenOptions::new().write(true).open(&file).and_then(|f| f.set_modified(SystemTime::now()));
        if renewed.is_ok() {
            return Ok(false);
        }
    }
    // Twice at most: a folder that went missing between making it and writing into it (a sweep, a
    // tidy-up tool) is made again.
    let mut last = None;
    for _ in 0..2 {
        own_dir(&sub)?;
        match write_atomic(&file, bytes) {
            Ok(()) => return Ok(true),
            // Somebody else saving the same content at the same moment is no failure.
            Err(_) if there(&file) => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("the content folder keeps going away")))
}

/// Which of the 256 folders of two digits are folders of their own (not links to somewhere
/// else), each looked at once however many files are read from it.
struct Folders([OnceLock<bool>; 256]);

impl Folders {
    fn new() -> Folders {
        Folders(std::array::from_fn(|_| OnceLock::new()))
    }

    fn real(&self, hash: &Digest, sub: &Path) -> bool {
        let first = hash.0.first().copied().unwrap_or(0);
        self.0.get(usize::from(first)).is_some_and(|seen| *seen.get_or_init(|| fs::symlink_metadata(sub).is_ok_and(|m| m.file_type().is_dir())))
    }
}

/// Open `file` for reading if it is a plain file; a link is never followed.
fn open_plain(file: &Path) -> Option<(fs::File, u64)> {
    #[cfg(windows)]
    let opened = {
        use std::os::windows::fs::OpenOptionsExt as _;
        // FILE_FLAG_OPEN_REPARSE_POINT: a link opens as itself, and then isn't a plain file.
        fs::OpenOptions::new().read(true).custom_flags(0x0020_0000).open(file).ok()?
    };
    #[cfg(not(windows))]
    let opened = fs::symlink_metadata(file).ok().filter(|m| m.file_type().is_file()).and_then(|_| fs::File::open(file).ok())?;
    let meta = opened.metadata().ok()?;
    meta.file_type().is_file().then_some((opened, meta.len()))
}

/// The content `r` from under `dir`, if it is there and is what its name says: a plain file (not
/// a link, and in a folder that isn't one either) of that length with that SHA-256.
fn get(dir: &Path, r: &Ref, folders: &Folders) -> Option<Vec<u8>> {
    let (sub, file) = blob_path(dir, &r.hash)?;
    if r.len > MAX_BLOB || !folders.real(&r.hash, &sub) {
        return None;
    }
    let (file, on_disk) = open_plain(&file)?;
    if on_disk != r.len {
        return None;
    }
    let len = usize::try_from(r.len).ok()?;
    let mut bytes = Vec::new();
    // Memory that isn't there is content that is unavailable, not a crash.
    bytes.try_reserve_exact(len.checked_add(1)?).ok()?;
    // Never more than was promised, however the file grows while it is read.
    file.take(r.len.checked_add(1)?).read_to_end(&mut bytes).ok()?;
    (bytes.len() == len && Digest::of(&bytes) == r.hash).then_some(bytes)
}

/// Do `work` for each of `0..n`, on a few threads when there is a lot of it: content files are
/// many and mostly small, and one after another the disk spends its time waiting. Results come
/// back by index; `None` where a thread gave up. A thread that can't start costs only speed.
fn fan_out<T: Send>(n: usize, work: impl Fn(usize) -> T + Sync) -> Vec<Option<T>> {
    let threads = std::thread::available_parallelism().map_or(1, |p| p.get()).min(MAX_THREADS).min(n / 8).max(1);
    let next = AtomicUsize::new(0);
    let run = || {
        let mut done = Vec::new();
        loop {
            let i = next.fetch_add(1, Ordering::Relaxed);
            if i >= n {
                return done;
            }
            done.push((i, work(i)));
        }
    };
    let mut out: Vec<Option<T>> = (0..n).map(|_| None).collect();
    std::thread::scope(|scope| {
        let others: Vec<_> =
            (1..threads).filter_map(|_| std::thread::Builder::new().name("mailbox content".into()).spawn_scoped(scope, run).ok()).collect();
        let mine = run();
        for (i, value) in others.into_iter().filter_map(|t| t.join().ok()).flatten().chain(mine) {
            if let Some(slot) = out.get_mut(i) {
                *slot = Some(value);
            }
        }
    });
    out
}

// ---- opening ----

/// A mailbox read from its file.
pub struct Opened {
    pub mailbox: Mailbox,
    /// The file keeps its content in a folder beside it (`false`: one self-contained file).
    pub split: bool,
    /// Bodies and attachments whose content file is missing or damaged.
    pub unavailable: usize,
}

impl std::fmt::Debug for Opened {
    /// Without the mail itself.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Opened {{ messages: {}, split: {}, unavailable: {} }}", self.mailbox.messages.len(), self.split, self.unavailable)
    }
}

/// Open the mailbox file at `path`, in either form. Whatever the file and its content folder
/// hold, the result is consistent (`Mailbox::repair`).
pub fn open(path: &Path) -> Result<Opened, String> {
    let shared = shared(path);
    let mut st = lock(&shared.state);
    let show = path.display();
    let file = resolve(path);
    let len = fs::metadata(&file).map_err(|e| format!("{show}: {e}"))?.len();
    if len > MAX_FILE {
        return Err(format!("{show} is larger than {} MB", MAX_FILE >> 20));
    }
    let bytes = fs::read(&file).map_err(|e| format!("{show}: {e}"))?;
    let mut mailbox: Mailbox = serde_json::from_slice(&bytes).map_err(|e| format!("{show}: this isn't a MailCraft mailbox ({e})"))?;
    // Whatever was known about this file was about another state of it.
    *st = State { newest: st.newest, swept: st.swept, ..State::default() };
    let (split, unavailable) = match mailbox.version {
        0 | 1 => (false, 0),
        SPLIT_VERSION => {
            let head: Head = serde_json::from_slice(&bytes).map_err(|e| format!("{show}: this mailbox file is damaged ({e})"))?;
            (true, fill(&mut st, &blob_dir(&file), &mut mailbox, head))
        }
        newer => return Err(format!("{show}: this mailbox was written by a newer MailCraft (format {newer})")),
    };
    // In memory a mailbox is whole, whichever form it came from.
    mailbox.version = 1;
    for line in mailbox.repair() {
        log::warn!("mailbox repaired: {line}");
    }
    if unavailable > 0 {
        log::warn!("{show}: {unavailable} bodies or attachments are missing from {} or damaged", blob_dir(&file).display());
    }
    st.stamp = Stamp::of(&file, &bytes);
    st.loaded = Loaded::of(&mailbox);
    Ok(Opened { mailbox, split, unavailable })
}

/// What a mailbox file wants for one part of a message.
struct Want {
    /// `None`: what the file gives as a name isn't one.
    r: Option<Ref>,
    /// Where its bytes will be (an index into what is read); `None` when it can't be read.
    slot: Option<usize>,
}

/// Put the content the mailbox file names back into `mailbox`'s messages. Returns how many
/// bodies and attachments couldn't be read.
fn fill(st: &mut State, dir: &Path, mailbox: &mut Mailbox, head: Head) -> usize {
    // Every distinct piece of content is read once, and only as much as fits the budget: a file
    // can name the same large content for a million bodies.
    let mut slots: Vec<(Ref, u32)> = Vec::new();
    let mut index: HashMap<Ref, usize> = HashMap::new();
    let mut budget = MAX_CONTENT;
    let mut want = |wire: &Option<(String, u64)>, wanted: bool, body: bool, live: &mut HashSet<Digest>| -> Option<Want> {
        let wire = wire.as_ref()?;
        let Some(r) = parse_ref(wire) else { return Some(Want { r: None, slot: None }) };
        live.insert(r.hash);
        if !wanted {
            return None;
        }
        // A body is a copy of its own; an attachment shares its bytes with every other use.
        let cost = if body || !index.contains_key(&r) { r.len } else { 0 };
        if r.len > MAX_BLOB || cost > budget {
            return Some(Want { r: Some(r), slot: None });
        }
        budget -= cost;
        let slot = *index.entry(r).or_insert_with(|| {
            slots.push((r, 0));
            slots.len().saturating_sub(1)
        });
        if let Some((_, uses)) = slots.get_mut(slot) {
            *uses = uses.saturating_add(1);
        }
        Some(Want { r: Some(r), slot: Some(slot) })
    };
    let mut plan = Vec::new();
    for (id, wire) in &head.blobs {
        // Content of a message that isn't there is still named by the file on disk.
        let wanted = mailbox.messages.contains_key(id);
        let text = want(&wire.text, wanted, true, &mut st.live);
        let html = want(&wire.html, wanted, true, &mut st.live);
        let attachments: Vec<Option<Want>> = wire.attachments.iter().map(|a| want(a, wanted, false, &mut st.live)).collect();
        if wanted {
            plan.push((*id, text, html, attachments));
        }
    }
    let folders = Folders::new();
    let read = fan_out(slots.len(), |i| slots.get(i).and_then(|(r, _)| get(dir, r, &folders)));
    let mut data: Vec<Option<Arc<Vec<u8>>>> = read.into_iter().map(|b| b.flatten().map(Arc::new)).collect();
    let mut uses: Vec<u32> = slots.iter().map(|(_, uses)| *uses).collect();
    // The bytes for one more use; the last use takes them along instead of a copy.
    let mut take = |want: &Want| -> Option<Arc<Vec<u8>>> {
        let slot = want.slot?;
        let left = uses.get_mut(slot)?;
        *left = left.saturating_sub(1);
        if *left == 0 { data.get_mut(slot)?.take() } else { data.get(slot)?.clone() }
    };
    let text_of = |bytes: Arc<Vec<u8>>| {
        let bytes = Arc::try_unwrap(bytes).unwrap_or_else(|used_elsewhere| used_elsewhere.to_vec());
        String::from_utf8(bytes).unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned())
    };
    let kept = |want: &Want| want.r.map(|r| Part { r, held: false });
    let mut unavailable = 0;
    for (id, text, html, attachments) in plan {
        let Some(m) = mailbox.messages.get_mut(&id).map(Arc::make_mut) else { continue };
        let mut gone = Parts { attachments: vec![None; m.attachments.len()], ..Parts::default() };
        if let Some(want) = &text {
            match take(want) {
                Some(bytes) => m.body.text = text_of(bytes),
                None => {
                    m.body.text = UNAVAILABLE.to_string();
                    gone.text = kept(want);
                    unavailable += 1;
                }
            }
        }
        if let Some(want) = &html {
            match take(want) {
                Some(bytes) => m.body.html = Some(text_of(bytes)),
                // Without its HTML a message shows its plain text.
                None => {
                    m.body.html = None;
                    gone.html = kept(want);
                    unavailable += 1;
                }
            }
        }
        for (i, want) in attachments.iter().enumerate() {
            let (Some(want), Some(a)) = (want, m.attachments.get_mut(i)) else { continue };
            match (take(want), want.r) {
                (Some(bytes), Some(r)) => {
                    st.hashes.insert(Arc::as_ptr(&bytes) as usize, (Arc::downgrade(&bytes), r.hash));
                    a.data = bytes;
                }
                // The attachment keeps its name and type, with nothing in it.
                _ => {
                    a.data = Arc::default();
                    if let Some(slot) = gone.attachments.get_mut(i) {
                        *slot = kept(want);
                    }
                    unavailable += 1;
                }
            }
        }
        if !gone.is_empty() {
            let (text, files) = (Digest::of(m.body.text.as_bytes()), m.attachments.iter().map(|a| a.name.clone()).collect());
            st.missing.insert(id, Lost { parts: gone, message_id: m.message_id.clone(), text, files });
        }
    }
    unavailable
}

// ---- saving ----

/// What a save wrote.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Saved {
    /// Bytes of the mailbox file written (0 when it already was as it should be).
    pub bytes: u64,
    /// Content files written, and their bytes.
    pub blobs: usize,
    pub blob_bytes: u64,
    /// Nothing was written: the file already holds this state, or a newer one.
    pub unchanged: bool,
}

/// Save `mailbox` to the file at `path` and its content folder. An error leaves the file as it
/// was (and readable).
pub fn save(path: &Path, mailbox: &Mailbox) -> Result<Saved, String> {
    save_as_of(path, mailbox, next_seq())
}

fn save_as_of(path: &Path, mailbox: &Mailbox, seq: u64) -> Result<Saved, String> {
    let shared = shared(path);
    let mut st = lock(&shared.state);
    if seq < st.newest {
        return Ok(Saved { unchanged: true, ..Saved::default() });
    }
    let saved = write_split(&mut st, &resolve(path), mailbox).map_err(|e| format!("{}: {e}", path.display()))?;
    st.newest = seq;
    Ok(saved)
}

/// Save `mailbox` to `path` as one self-contained file (an export, a copy to carry somewhere),
/// written whole and atomically. Returns the bytes written.
pub fn save_single(path: &Path, mailbox: &Mailbox) -> Result<u64, String> {
    let bytes = crate::io::write_mailbox(mailbox)?;
    let shared = shared(path);
    let mut st = lock(&shared.state);
    write_atomic(&resolve(path), &bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    // The file is another one now: nothing known about the old one holds, and a state of it
    // still waiting to be written has had its turn.
    *st = State { newest: next_seq(), swept: st.swept, ..State::default() };
    Ok(bytes.len() as u64)
}

/// What a message comes to in the mailbox file: itself without the content that goes outside,
/// and the names of that content.
fn prepare(m: &Arc<Message>, lost: Option<&Lost>, hashes: &mut HashMap<usize, (Weak<Vec<u8>>, Digest)>) -> Result<Cached, String> {
    let outside = |bytes: &[u8]| (bytes.len() > INLINE_MAX).then(|| Part { r: Ref::of(bytes), held: true });
    // Content that couldn't be read when the mailbox was opened keeps the name it had, as long
    // as this is that message and still shows what was put in the content's place: the notice
    // for its text, no HTML beside the text it was opened with, an empty file of the same name.
    let lost = lost.filter(|l| l.message_id == m.message_id);
    let same_text = lost.is_some_and(|l| l.text == Digest::of(m.body.text.as_bytes()));
    let mut parts = Parts {
        text: lost.filter(|_| m.body.text == UNAVAILABLE).and_then(|l| l.parts.text).or_else(|| outside(m.body.text.as_bytes())),
        html: match &m.body.html {
            None => lost.filter(|_| same_text).and_then(|l| l.parts.html),
            Some(html) => outside(html.as_bytes()),
        },
        attachments: Vec::new(),
    };
    let same_files = lost.is_some_and(|l| l.files.len() == m.attachments.len());
    for (i, a) in m.attachments.iter().enumerate().take(MAX_PARTS) {
        let was = lost
            .filter(|l| same_files && a.data.is_empty() && l.files.get(i) == Some(&a.name))
            .and_then(|l| l.parts.attachments.get(i).copied().flatten());
        let part = was.or_else(|| {
            (a.data.len() > INLINE_MAX).then(|| {
                let key = Arc::as_ptr(&a.data) as usize;
                let known = hashes.get(&key).filter(|(of, _)| std::ptr::eq(of.as_ptr(), Arc::as_ptr(&a.data))).map(|(_, hash)| *hash);
                let hash = known.unwrap_or_else(|| {
                    let hash = Digest::of(&a.data);
                    hashes.insert(key, (Arc::downgrade(&a.data), hash));
                    hash
                });
                Part { r: Ref { hash, len: a.data.len() as u64 }, held: true }
            })
        });
        parts.attachments.push(part);
    }
    if parts.attachments.iter().all(Option::is_none) {
        parts.attachments.clear();
    }
    let mut lean = Message::clone(m);
    if parts.text.is_some() {
        lean.body.text = String::new();
    }
    if parts.html.is_some() {
        lean.body.html = None;
    }
    let nothing: Arc<Vec<u8>> = Arc::default();
    for (a, part) in lean.attachments.iter_mut().zip(&parts.attachments) {
        if part.is_some() {
            a.data = nothing.clone();
        }
    }
    let json = serde_json::to_vec(&lean).map_err(|e| e.to_string())?;
    let refs = if parts.is_empty() { Vec::new() } else { parts.json().into_bytes() };
    Ok(Cached { of: Arc::downgrade(m), json: json.into(), parts, refs: refs.into() })
}

fn write_split(st: &mut State, path: &Path, mailbox: &Mailbox) -> Result<Saved, String> {
    // Somebody else wrote the file since: what it names now is theirs to say, and what it named
    // before may be swept any time.
    let ours = st.stamp.as_ref().is_some_and(|s| s.still(path));
    if !ours {
        st.live.clear();
        st.stamp = None;
    }
    if ours && st.loaded.as_ref().is_some_and(|loaded| loaded.is(mailbox)) {
        return Ok(Saved { unchanged: true, ..Saved::default() });
    }
    st.loaded = None;

    // Each message: as it was last written when nobody touched it since, worked out otherwise.
    let mut cache = HashMap::with_capacity(mailbox.messages.len());
    let mut live: HashSet<Digest> = HashSet::new();
    let mut wanted: HashMap<Ref, &[u8]> = HashMap::new();
    let (mut messages, mut blobs) = (Vec::new(), Vec::new());
    let mut failed = None;
    for (id, m) in &mailbox.messages {
        let key = Arc::as_ptr(m) as usize;
        let known = st.cache.remove(&key).filter(|c| std::ptr::eq(c.of.as_ptr(), Arc::as_ptr(m)));
        let entry = match known.map_or_else(|| prepare(m, st.missing.get(id), &mut st.hashes), Ok) {
            Ok(entry) => entry,
            Err(e) => {
                failed = Some(e);
                break;
            }
        };
        if entry.parts.all().any(|p| p.held && !st.live.contains(&p.r.hash)) {
            for (r, bytes) in entry.parts.content(m) {
                if !st.live.contains(&r.hash) {
                    wanted.entry(r).or_insert(bytes);
                }
            }
        }
        live.extend(entry.parts.all().map(|p| p.r.hash));
        for (out, value) in [(&mut messages, &entry.json), (&mut blobs, &entry.refs)] {
            if !value.is_empty() {
                if !out.is_empty() {
                    out.push(b',');
                }
                out.extend_from_slice(format!("\"{id}\":").as_bytes());
                out.extend_from_slice(value);
            }
        }
        cache.insert(key, entry);
    }
    st.cache = cache;
    st.hashes.retain(|_, (of, _)| of.strong_count() > 0);
    if let Some(e) = failed {
        return Err(e);
    }

    // The content first: a mailbox file never names what isn't on disk yet.
    let mut saved = Saved::default();
    let wanted: Vec<(Ref, &[u8])> = wanted.into_iter().collect();
    if !wanted.is_empty() {
        // The folder beside the mailbox file, and no more: a mailbox isn't saved to a place that
        // isn't there.
        let dir = blob_dir(path);
        if let Err(e) = make_dir(&dir)
            && !dir.is_dir()
        {
            return Err(format!("{}: {e}", dir.display()));
        }
        let done = fan_out(wanted.len(), |i| wanted.get(i).map(|(r, bytes)| put(&dir, r, bytes)));
        for (result, (r, _)) in done.into_iter().zip(&wanted) {
            match result.flatten() {
                Some(Ok(true)) => {
                    saved.blobs += 1;
                    saved.blob_bytes += r.len;
                }
                Some(Ok(false)) => {}
                Some(Err(e)) => return Err(format!("content file {}: {e}", r.hash.hex())),
                None => return Err("the content couldn't be written".into()),
            }
        }
    }

    // Everything but the messages as the model writes it, with the messages spliced in.
    let mut shell = mailbox.clone();
    shell.messages = BTreeMap::new();
    shell.version = SPLIT_VERSION;
    let serde_json::Value::Object(shell) = serde_json::to_value(&shell).map_err(|e| e.to_string())? else {
        return Err("the mailbox didn't come out as expected".into());
    };
    // Never a file without its messages, or with two ideas of where the content is.
    if !shell.contains_key("messages") || shell.contains_key("blobs") {
        return Err("the mailbox didn't come out as expected".into());
    }
    let mut out = Vec::with_capacity(messages.len() + blobs.len() + 4096);
    out.push(b'{');
    for (key, value) in &shell {
        serde_json::to_writer(&mut out, key).map_err(|e| e.to_string())?;
        out.push(b':');
        if key == "messages" {
            out.push(b'{');
            out.extend_from_slice(&messages);
            out.push(b'}');
        } else {
            serde_json::to_writer(&mut out, value).map_err(|e| e.to_string())?;
        }
        out.push(b',');
    }
    out.extend_from_slice(b"\"blobs\":{");
    out.extend_from_slice(&blobs);
    out.extend_from_slice(b"}}");

    // The file already says exactly this (a change that was undone, a save asked for twice).
    let digest = Digest::of(&out);
    if st.stamp.as_ref().is_some_and(|s| s.digest == digest && s.len == out.len() as u64) {
        saved.unchanged = saved.blobs == 0;
        st.live = live;
        return Ok(saved);
    }
    write_atomic(path, &out).map_err(|e| e.to_string())?;
    st.stamp = fs::metadata(path).ok().map(|m| Stamp { len: m.len(), modified: m.modified().ok(), digest });
    st.live = live;
    saved.bytes = out.len() as u64;
    Ok(saved)
}

// ---- sweeping ----

/// What a sweep did.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Swept {
    /// Files removed, and their bytes.
    pub removed: usize,
    pub bytes: u64,
    /// Content files left in place.
    pub kept: usize,
}

/// The content the mailbox file at `path` names, read from the file itself. `None` when the file
/// isn't in the form that keeps content outside.
fn named_by(path: &Path) -> Result<Option<HashSet<Digest>>, String> {
    let len = fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > MAX_FILE {
        return Err("too large".into());
    }
    let head: Head = serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    if head.version != SPLIT_VERSION {
        return Ok(None);
    }
    let all =
        |w: &WireParts| w.text.iter().chain(&w.html).chain(w.attachments.iter().flatten()).filter_map(parse_ref).map(|r| r.hash).collect::<Vec<_>>();
    Ok(Some(head.blobs.values().flat_map(all).collect()))
}

/// Remove the content files next to the mailbox file at `path` that it no longer names, and
/// temporary files a crash left behind, when they are older than `grace` ([`GRACE`]): a save in
/// flight has written its content before its mailbox file names it. Without a readable mailbox
/// file in this form nothing is removed, and nothing that isn't recognisably ours ever is.
pub fn sweep(path: &Path, grace: Duration) -> Result<Swept, String> {
    let shared = shared(path);
    let mut st = lock(&shared.state);
    st.swept = Some(Instant::now());
    let file = resolve(path);
    let dir = blob_dir(&file);
    let Some(named) = named_by(&file).map_err(|e| format!("{}: {e}", path.display()))? else { return Ok(Swept::default()) };
    let now = SystemTime::now();
    // A time in the future is no age at all.
    let old = |meta: &fs::Metadata| meta.modified().ok().and_then(|t| now.duration_since(t).ok()).is_some_and(|age| age >= grace);
    let mut swept = Swept::default();
    let remove = |stale: &Path, meta: &fs::Metadata, swept: &mut Swept| {
        // Looked at again right before it goes: a save may just have written or renewed it.
        if fs::symlink_metadata(stale).is_ok_and(|m| !m.is_dir() && old(&m)) && fs::remove_file(stale).is_ok() {
            swept.removed += 1;
            swept.bytes += meta.len();
        }
    };
    let mut seen = 0usize;
    'folders: for sub in fs::read_dir(&dir).into_iter().flatten().flatten() {
        let folder = sub.file_name();
        let Some(folder) = folder.to_str().filter(|n| n.len() == 2 && is_hex(n)) else { continue };
        // Only real folders are walked: a link is never followed.
        if !sub.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        for entry in fs::read_dir(sub.path()).into_iter().flatten().flatten() {
            seen += 1;
            if seen > MAX_ENTRIES {
                break 'folders;
            }
            let name = entry.file_name();
            let (Some(name), Ok(meta)) = (name.to_str(), entry.metadata()) else { continue };
            if meta.is_dir() {
                continue;
            }
            let stale = match Digest::parse(name) {
                Some(hash) => !name.starts_with(folder) || !named.contains(&hash),
                None => name.get(..64).is_some_and(|stem| Digest::parse(stem).is_some() && is_temp_of(name, stem)),
            };
            if stale && old(&meta) {
                remove(&entry.path(), &meta, &mut swept);
            } else if !stale {
                swept.kept += 1;
            }
        }
    }
    // Temporary files of the mailbox file itself.
    if let (Some(parent), Some(stem)) = (file.parent(), file.file_name().and_then(|n| n.to_str())) {
        let parent = if parent.as_os_str().is_empty() { Path::new(".") } else { parent };
        for entry in fs::read_dir(parent).into_iter().flatten().flatten().take(MAX_ENTRIES) {
            let name = entry.file_name();
            let (Some(name), Ok(meta)) = (name.to_str(), entry.metadata()) else { continue };
            if !meta.is_dir() && is_temp_of(name, stem) && old(&meta) {
                remove(&entry.path(), &meta, &mut swept);
            }
        }
    }
    Ok(swept)
}

// ---- saving in the background ----

/// Writes mailbox files on a thread of its own, so whoever hands it a state never waits for the
/// disk. A state handed in while another is being written waits its turn; a newer one handed in
/// meanwhile takes its place, so a save in flight is followed by at most one more, with the
/// newest state. Now and then it sweeps what a mailbox no longer names.
pub struct Saver {
    queue: Arc<Queue>,
}

struct Queue {
    jobs: Mutex<Jobs>,
    /// Something to do (for the thread).
    work: Condvar,
    /// Nothing left to write (for [`Saver::flush`]).
    idle: Condvar,
    /// Wakes whoever wants to hear that a state was written (the window).
    notify: Box<dyn Fn() + Send + Sync>,
}

#[derive(Default)]
struct Jobs {
    /// States waiting to be written: the newest one of each mailbox file.
    waiting: Vec<Job>,
    /// The file being written and the state going into it.
    writing: Option<(PathBuf, u64)>,
    sweeps: Vec<PathBuf>,
    reports: Vec<SaveReport>,
    closed: bool,
}

struct Job {
    path: PathBuf,
    mailbox: Mailbox,
    rev: u64,
    seq: u64,
}

enum Task {
    Save(Box<Job>),
    Sweep(PathBuf),
}

impl Saver {
    /// Start the thread. `notify` is called (on that thread) each time a state has been written
    /// or has failed to be: wake whatever reads [`BackgroundSave::reports`].
    pub fn spawn(notify: impl Fn() + Send + Sync + 'static) -> std::io::Result<Saver> {
        let queue = Arc::new(Queue { jobs: Mutex::default(), work: Condvar::new(), idle: Condvar::new(), notify: Box::new(notify) });
        let theirs = queue.clone();
        std::thread::Builder::new().name("mailbox saver".into()).spawn(move || theirs.run())?;
        Ok(Saver { queue })
    }

    /// Sweep the content folder of the mailbox file at `path` when there is nothing to write
    /// (after opening it, say).
    pub fn sweep_later(&self, path: &Path) {
        let mut jobs = lock(&self.queue.jobs);
        if !jobs.closed && !jobs.sweeps.iter().any(|p| p == path) {
            jobs.sweeps.push(path.to_path_buf());
            self.queue.work.notify_one();
        }
    }
}

impl BackgroundSave for Saver {
    fn follow(&self, path: &Path) {
        shared(path).followed.store(true, Ordering::Relaxed);
    }

    fn submit(&self, path: &Path, mailbox: Mailbox, rev: u64) {
        self.follow(path);
        let mut jobs = lock(&self.queue.jobs);
        // The same state again (the window asks until it hears back) is already on its way.
        if jobs.closed || jobs.writing.as_ref().is_some_and(|(p, r)| p == path && *r == rev) {
            return;
        }
        match jobs.waiting.iter_mut().find(|j| j.path == path) {
            Some(job) if job.rev == rev => return,
            Some(job) => (job.mailbox, job.rev, job.seq) = (mailbox, rev, next_seq()),
            None => jobs.waiting.push(Job { path: path.to_path_buf(), mailbox, rev, seq: next_seq() }),
        }
        self.queue.work.notify_one();
    }

    fn reports(&self) -> Vec<SaveReport> {
        std::mem::take(&mut lock(&self.queue.jobs).reports)
    }

    fn flush(&self) {
        let deadline = Instant::now() + FLUSH_WAIT;
        let mut jobs = lock(&self.queue.jobs);
        while !jobs.waiting.is_empty() || jobs.writing.is_some() {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                log::warn!("gave up waiting for the mailbox to be written");
                return;
            }
            jobs = self.queue.idle.wait_timeout(jobs, left).map_or_else(|e| e.into_inner().0, |(jobs, _)| jobs);
        }
    }
}

impl Drop for Saver {
    /// The thread writes what is still waiting, then ends.
    fn drop(&mut self) {
        lock(&self.queue.jobs).closed = true;
        self.queue.work.notify_all();
    }
}

impl Queue {
    fn run(&self) {
        loop {
            let task = {
                let mut jobs = lock(&self.jobs);
                loop {
                    if !jobs.waiting.is_empty() {
                        let job = jobs.waiting.remove(0);
                        jobs.writing = Some((job.path.clone(), job.rev));
                        break Task::Save(Box::new(job));
                    }
                    self.idle.notify_all();
                    if jobs.closed {
                        return;
                    }
                    if let Some(path) = jobs.sweeps.pop() {
                        break Task::Sweep(path);
                    }
                    jobs = self.work.wait(jobs).unwrap_or_else(|e| e.into_inner());
                }
            };
            match task {
                Task::Save(job) => {
                    let Job { path, mailbox, rev, seq } = *job;
                    // Never-crash: a panic in here is a save that failed, and the thread lives on.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| save_as_of(&path, &mailbox, seq)))
                        .unwrap_or_else(|_| Err("saving the mailbox stopped unexpectedly".to_string()));
                    drop(mailbox);
                    let due = result.is_ok() && lock(&shared(&path).state).swept.is_none_or(|at| at.elapsed() >= SWEEP_EVERY);
                    let mut jobs = lock(&self.jobs);
                    jobs.writing = None;
                    if due && !jobs.sweeps.contains(&path) {
                        jobs.sweeps.push(path.clone());
                    }
                    jobs.reports.push(SaveReport { path, rev, result: result.map(|_| ()) });
                    drop(jobs);
                    (self.notify)();
                }
                Task::Sweep(path) => match std::panic::catch_unwind(|| sweep(&path, GRACE)) {
                    Ok(Ok(swept)) if swept.removed > 0 => {
                        log::info!("{}: removed {} content file(s) nothing names any more ({} bytes)", path.display(), swept.removed, swept.bytes)
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(e)) => log::warn!("sweeping the mailbox's content folder failed: {e}"),
                    Err(_) => log::error!("sweeping the mailbox's content folder stopped unexpectedly"),
                },
            }
        }
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

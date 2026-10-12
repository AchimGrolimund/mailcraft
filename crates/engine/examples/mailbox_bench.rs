//! How long keeping a real-sized mailbox on disk takes.
//!
//! `cargo run --release -p mailcraft-engine --example mailbox_bench [-- --messages 5000 --attachments 300 --keep DIR]`
//!
//! Builds an invented mailbox the size of a few years of somebody's mail (5,000 messages with plain
//! and HTML bodies, 300 attachments from 20 KB to 5 MB, about 300 MB in all), then times saving and
//! opening it both ways: as one self-contained file (how every save used to go, and how an export
//! still does), and as the desktop app keeps it, with the content in a folder beside the mailbox
//! file (`mailcraft_engine::store`). Everything is written under the system's temporary folder and
//! removed afterwards; `--keep DIR` writes into `DIR` instead and leaves both mailboxes there, to try
//! the app on (`mailcraft --mailbox DIR/single.mailcraft`).

use std::path::Path;
use std::time::{Duration, Instant};

use mailcraft_engine::Session;
use mailcraft_engine::io;
use mailcraft_model::{Address, Attachment, FolderKind, Mailbox, Message};

/// xorshift64*: the same mailbox every run.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// 0.0 ≤ x < 1.0.
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn pick<'a>(&mut self, list: &[&'a str]) -> &'a str {
        list.get(self.below(list.len())).copied().unwrap_or("")
    }

    /// Bytes nothing compresses, like the pictures, PDFs and archives mail carries.
    fn bytes(&mut self, len: usize) -> Vec<u8> {
        let mut out = Vec::with_capacity(len + 8);
        while out.len() < len {
            out.extend_from_slice(&self.next().to_le_bytes());
        }
        out.truncate(len);
        out
    }
}

const FIRST: &[&str] =
    &["Ada", "Bram", "Chidi", "Dana", "Elif", "Farid", "Greta", "Hugo", "Ines", "Jonas", "Keiko", "Liam", "Mira", "Nils", "Oona", "Pavel"];
const LAST: &[&str] = &["Aldrin", "Brooks", "Castell", "Dufour", "Eklund", "Fontaine", "Garza", "Holm", "Ibarra", "Jansen", "Kowal", "Lindqvist"];
const WORDS: &[&str] = &[
    "the",
    "report",
    "meeting",
    "budget",
    "please",
    "review",
    "attached",
    "schedule",
    "quarter",
    "figures",
    "draft",
    "thanks",
    "tomorrow",
    "update",
    "team",
    "project",
    "deadline",
    "invoice",
    "proposal",
    "feedback",
    "client",
    "delivery",
    "numbers",
    "plan",
    "agenda",
    "notes",
    "version",
    "changes",
    "approve",
    "follow",
    "regards",
    "morning",
    "afternoon",
    "question",
    "answer",
    "details",
    "summary",
    "forecast",
    "contract",
    "design",
];
const KINDS: &[(&str, &str)] =
    &[("pdf", "application/pdf"), ("png", "image/png"), ("jpg", "image/jpeg"), ("zip", "application/zip"), ("xlsx", "application/octet-stream")];

fn sentence(rng: &mut Rng, out: &mut String) {
    let n = 5 + rng.below(14);
    for i in 0..n {
        let w = rng.pick(WORDS);
        if i == 0 {
            let mut c = w.chars();
            out.extend(c.next().map(|f| f.to_ascii_uppercase()));
            out.push_str(c.as_str());
        } else {
            out.push(' ');
            out.push_str(w);
        }
    }
    out.push_str(". ");
}

/// Plain text of about `len` bytes, in paragraphs.
fn text(rng: &mut Rng, len: usize) -> String {
    let mut out = String::with_capacity(len + 200);
    while out.len() < len {
        for _ in 0..2 + rng.below(4) {
            sentence(rng, &mut out);
        }
        out.push_str("\n\n");
    }
    out
}

/// HTML of about `len` bytes, the way mail programs and newsletters write it: nested tables and
/// inline styles around not much text.
fn html(rng: &mut Rng, len: usize) -> String {
    let mut out = String::with_capacity(len + 600);
    out.push_str("<html><head><meta http-equiv=\"Content-Type\" content=\"text/html; charset=utf-8\"></head><body style=\"margin:0;padding:0\">");
    out.push_str("<table role=\"presentation\" width=\"100%\" cellpadding=\"0\" cellspacing=\"0\" border=\"0\">");
    while out.len() < len {
        out.push_str("<tr><td style=\"font-family:Segoe UI,Arial,sans-serif;font-size:14px;line-height:21px;color:#1f1f1f;padding:8px 24px\"><p style=\"margin:0 0 12px 0\">");
        for _ in 0..1 + rng.below(4) {
            sentence(rng, &mut out);
        }
        out.push_str("</p></td></tr>");
    }
    out.push_str("</table></body></html>");
    out
}

fn person(rng: &mut Rng) -> Address {
    let (first, last) = (rng.pick(FIRST), rng.pick(LAST));
    Address::new(format!("{first} {last}"), format!("{}.{}@example.org", first.to_lowercase(), last.to_lowercase()))
}

/// The invented mailbox, and how many bytes of content it carries.
fn build(messages: usize, attachments: usize) -> Result<(Mailbox, u64), String> {
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let mut mb = Mailbox::with_local_account("Avery Quinn", "avery@example.org");
    let folder = |mb: &Mailbox, kind: FolderKind| mb.folders.iter().find(|f| f.kind == kind).map(|f| f.id);
    let inbox = mb.inbox().ok_or("no inbox")?;
    let sent = folder(&mb, FolderKind::Sent).unwrap_or(inbox);
    let archive = folder(&mb, FolderKind::Archive).unwrap_or(inbox);
    let me = Address::new("Avery Quinn", "avery@example.org");
    // Which messages carry a file: spread evenly.
    let every = (messages / attachments.max(1)).max(1);
    let mut files: Vec<Attachment> = Vec::new();
    let mut size = 0u64;
    let start = 1_700_000_000i64;
    for i in 0..messages {
        let other = person(&mut rng);
        let place = match rng.below(20) {
            0..=13 => inbox,
            14..=16 => sent,
            _ => archive,
        };
        let (from, to) = if place == sent { (me.clone(), other) } else { (other, me.clone()) };
        let mut subject = String::new();
        for k in 0..3 + rng.below(5) {
            if k > 0 {
                subject.push(' ');
            }
            subject.push_str(rng.pick(WORDS));
        }
        // Most mail is short; some is long.
        let body_len = 300 + (rng.unit().powi(2) * 8000.0) as usize;
        let mut m = Message {
            folder: place,
            message_id: format!("{i:06}.{:016x}@mail.example.org", rng.next()),
            from,
            to: vec![to],
            subject,
            date: start + (i as i64) * 12_345,
            received: start + (i as i64) * 12_345 + 40,
            read: rng.below(4) != 0,
            headers: vec![("X-Mailer".into(), "Example Mailer 4.2".into()), ("Thread-Index".into(), format!("{:032x}", rng.next()))],
            ..Default::default()
        };
        m.body.text = text(&mut rng, body_len);
        if rng.below(10) < 6 {
            // 4 KB to 80 KB, mostly towards the small end.
            let len = 4_000 + (rng.unit().powi(3) * 76_000.0) as usize;
            m.body.html = Some(html(&mut rng, len));
        }
        if i % every == 0 && files.len() < attachments {
            // One in twelve is a file that went round before (a forward, a reply to all).
            let again = if files.len() > 8 && rng.below(12) == 0 { files.get(rng.below(files.len())).cloned() } else { None };
            let file = match again {
                Some(file) => file,
                None => {
                    // 20 KB to 5 MB, each size as likely as the one twice as big.
                    let len = (20_000.0 * (250.0f64).powf(rng.unit())) as usize;
                    let (ext, mime) = KINDS.get(rng.below(KINDS.len())).copied().unwrap_or(("bin", "application/octet-stream"));
                    Attachment::new(format!("{}-{}.{ext}", rng.pick(WORDS), files.len()), mime, rng.bytes(len))
                }
            };
            files.push(file.clone());
            m.attachments.push(file);
        }
        size += m.size();
        mb.add_message(m).map_err(|e| e.to_string())?;
    }
    Ok((mb, size))
}

fn mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
}

fn ms(d: Duration) -> String {
    format!("{:>8.1} ms", d.as_secs_f64() * 1000.0)
}

fn timed<T>(work: impl FnOnce() -> T) -> (T, Duration) {
    let t = Instant::now();
    let out = work();
    (out, t.elapsed())
}

fn dir_size(dir: &Path) -> (u64, u64) {
    let (mut bytes, mut files) = (0, 0);
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) => {
                    bytes += m.len();
                    files += 1;
                }
                Err(_) => {}
            }
        }
    }
    (bytes, files)
}

/// The mailbox as one self-contained file: what every save wrote before the content moved out.
fn single_file(mailbox: &Mailbox, dir: &Path, keep: bool) -> Result<(), String> {
    println!("\nOne self-contained file (attachments as base64 inside it)");
    let path = dir.join("single.mailcraft");
    let (bytes, serialise) = timed(|| io::write_mailbox(mailbox));
    let bytes = bytes?;
    let (written, write) = timed(|| std::fs::write(&path, &bytes));
    written.map_err(|e| e.to_string())?;
    println!("  save: serialise            {}", ms(serialise));
    println!("  save: write {:<14} {}", mb(bytes.len() as u64), ms(write));
    println!("  save: in all               {}   <- the window waits for this", ms(serialise + write));
    drop(bytes);
    // Again after one message is marked read: the same work, whatever changed.
    let mut s = Session::new(mailbox.clone());
    let first = s.mailbox.messages.keys().next().copied().unwrap_or(0);
    s.mailbox.message_mut(first).map_err(|e| e.to_string())?.read ^= true;
    let (again, resave) = timed(|| io::write_mailbox(&s.mailbox).and_then(|b| std::fs::write(&path, b).map_err(|e| e.to_string())));
    again?;
    println!("  save after one read mark   {}", ms(resave));
    let (opened, open) = timed(|| io::open_path(&path));
    let opened = opened?;
    println!("  open                       {}", ms(open));
    if opened.messages.len() != mailbox.messages.len() {
        return Err("the single file didn't come back whole".into());
    }
    println!("  on disk: {} in one file", mb(std::fs::metadata(&path).map_or(0, |m| m.len())));
    if !keep {
        let _ = std::fs::remove_file(&path);
    }
    Ok(())
}

/// The mailbox as the desktop app keeps it: the mailbox file, and the content in a folder beside it.
fn split(mailbox: &Mailbox, dir: &Path) -> Result<(), String> {
    use mailcraft_engine::io::BackgroundSave as _;
    use mailcraft_engine::store;
    println!("\nThe mailbox file with its content folder (mailbox.mailcraft + mailbox.mailcraft.blobs/)");
    let path = dir.join("mailbox.mailcraft");
    let (first, took) = timed(|| store::save(&path, mailbox));
    let first = first?;
    println!("  first save (an old single file becoming this form): {}", ms(took));
    println!("      {} content files, {}; mailbox file {}", first.blobs, mb(first.blob_bytes), mb(first.bytes));

    let mut s = Session::new(mailbox.clone());
    let ids: Vec<u64> = s.mailbox.messages.keys().copied().collect();
    let flip = |s: &mut Session, n: usize| -> Result<(), String> {
        let id = ids.get(n).copied().unwrap_or(0);
        s.mailbox.message_mut(id).map_err(|e| e.to_string())?.read ^= true;
        Ok(())
    };
    flip(&mut s, 0)?;
    let (saved, took) = timed(|| store::save(&path, &s.mailbox));
    let saved = saved?;
    println!("  save after one read mark   {}   wrote {} ({} content files)", ms(took), mb(saved.bytes), saved.blobs);
    let mut times = Vec::new();
    for n in 1..=20 {
        flip(&mut s, n)?;
        let (saved, took) = timed(|| store::save(&path, &s.mailbox));
        saved?;
        times.push(took);
    }
    times.sort();
    println!(
        "      20 more: median {}, slowest {}",
        ms(times.get(times.len() / 2).copied().unwrap_or_default()).trim(),
        ms(times.last().copied().unwrap_or_default()).trim()
    );

    // A Send/Receive round: fifty new messages, five of them with a file.
    let mut rng = Rng(42);
    let inbox = s.mailbox.inbox().ok_or("no inbox")?;
    for i in 0..50 {
        let mut m = Message { folder: inbox, subject: format!("new {i}"), from: person(&mut rng), ..Default::default() };
        m.body.text = text(&mut rng, 2_000);
        m.body.html = Some(html(&mut rng, 20_000));
        if i % 10 == 0 {
            m.attachments.push(Attachment::new(format!("new-{i}.pdf"), "application/pdf", rng.bytes(400_000)));
        }
        s.mailbox.add_message(m).map_err(|e| e.to_string())?;
    }
    let (saved, took) = timed(|| store::save(&path, &s.mailbox));
    let saved = saved?;
    println!("  save after 50 new messages {}   wrote {} + {} in {} content files", ms(took), mb(saved.bytes), mb(saved.blob_bytes), saved.blobs);

    // What the window waits for when a background thread does the writing.
    let saver = store::Saver::spawn(|| {}).map_err(|e| e.to_string())?;
    flip(&mut s, 30)?;
    let (_, handed) = timed(|| saver.submit(&path, s.mailbox.clone(), 1));
    let (_, flushed) = timed(|| saver.flush());
    println!("  in the background: the window waits {}; written {} later", ms(handed).trim(), ms(flushed).trim());
    for report in saver.reports() {
        report.result?;
    }

    let (opened, took) = timed(|| store::open(&path));
    let opened = opened?;
    println!("  open                       {}   ({} unavailable)", ms(took), opened.unavailable);
    if opened.mailbox != s.mailbox {
        return Err("the mailbox didn't come back as it was saved".into());
    }
    // The first save after opening works every message out once more (their text is hashed again).
    let mut s = Session::new(opened.mailbox);
    flip(&mut s, 40)?;
    let (saved, took) = timed(|| store::save(&path, &s.mailbox));
    println!("  first save after opening   {}   wrote {}", ms(took), mb(saved?.bytes));
    let (swept, took) = timed(|| store::sweep(&path, store::GRACE));
    let swept = swept?;
    println!("  sweep                      {}   ({} kept, {} removed)", ms(took), swept.kept, swept.removed);
    let (bytes, files) = dir_size(&store::blob_dir(&path));
    println!("  on disk: mailbox file {}, content {} in {files} files", mb(std::fs::metadata(&path).map_or(0, |m| m.len())), mb(bytes));
    Ok(())
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |name: &str, default: usize| {
        args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse().ok()).unwrap_or(default)
    };
    let (messages, attachments) = (arg("--messages", 5000), arg("--attachments", 300));
    let (made, built) = timed(|| build(messages, attachments));
    let (mailbox, size) = made?;
    println!("{messages} messages, {attachments} attachments, {} of content (built in {})", mb(size), ms(built).trim());
    let (copy, clone) = timed(|| mailbox.clone());
    println!("a snapshot (clone of the mailbox) {}", ms(clone));
    drop(copy);
    let keep = args.iter().position(|a| a == "--keep").and_then(|i| args.get(i + 1)).map(std::path::PathBuf::from);
    let dir = keep.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("mailcraft-bench-{}", std::process::id())));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let result = single_file(&mailbox, &dir, keep.is_some()).and_then(|()| split(&mailbox, &dir));
    match keep {
        Some(dir) => println!("\nkept in {}", dir.display()),
        None => {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    result
}

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

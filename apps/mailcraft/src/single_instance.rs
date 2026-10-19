//! One MailCraft per mailbox.
//!
//! Two windows on one mailbox file would each save over the other's work. So the first to start
//! takes a lock beside the mailbox and listens on a loopback port; one started later (a second
//! click on the shortcut, a `mailto:` link, a file opened from the file manager) finds the lock
//! taken, hands what it was started with to the first, which comes to the front, and exits.
//!
//! Beside the mailbox: `<mailbox>.lock`, held locked for as long as the first one runs (the
//! system lets go of it however the program ends, so a crash leaves nothing to clean up), and
//! `<mailbox>.instance`, the port and a random key (mode 0600 on Unix; the settings folder is the
//! user's own on every system), removed on exit. The sample mailbox and runs without preferences
//! keep nothing in a file and are not guarded.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::control_server::{keys_match, random_key, write_key_file};

/// Longer lines are refused.
const MAX_LINE: usize = 1 << 20;
/// The most things one start can hand over, and the longest of them.
const MAX_ARGS: usize = 256;
const MAX_ARG: usize = 32 * 1024;
/// How long either side waits for the other on the connection.
const ANSWER: Duration = Duration::from_secs(5);
/// How long a later start keeps trying to reach the first (which may itself be starting).
const REACH: Duration = Duration::from_secs(8);

/// What starting found.
pub enum Start {
    /// Nobody else has this mailbox open: carry on, and keep the guard for as long as it is open.
    First(Guard),
    /// Another MailCraft has it open and took what this one was started with: exit.
    Handed,
    /// Another MailCraft has it open and could not be reached: say so and exit.
    Unreachable(String),
}

/// The first instance's hold on a mailbox, and what later starts hand to it.
pub struct Guard {
    /// Held, not read: the lock goes when the file is closed.
    _lock: Option<File>,
    info: Option<PathBuf>,
    rx: Receiver<Vec<String>>,
    wake: Arc<Mutex<Option<egui::Context>>>,
}

impl Guard {
    /// The window to wake when something is handed over.
    pub fn wake(&self, ctx: &egui::Context) {
        *self.wake.lock().unwrap_or_else(|e| e.into_inner()) = Some(ctx.clone());
    }

    /// What later starts were started with, since the last look; `Some` (even when empty) says
    /// one of them asked for the window.
    pub fn handed_over(&self) -> Option<Vec<String>> {
        let mut all: Option<Vec<String>> = None;
        while let Ok(args) = self.rx.try_recv() {
            all.get_or_insert_default().extend(args);
        }
        all
    }

    /// The window is closing: later starts have nobody to reach from here on.
    pub fn close(&self) {
        if let Some(info) = &self.info {
            let _ = std::fs::remove_file(info);
        }
    }
}

/// `<mailbox>.<what>`, beside the mailbox.
fn beside(mailbox: &Path, what: &str) -> PathBuf {
    let mut name = mailbox.file_name().unwrap_or_default().to_os_string();
    name.push(".");
    name.push(what);
    mailbox.with_file_name(name)
}

/// Take the mailbox, or hand `args` (files, `mailto:` links) to the MailCraft that has it.
pub fn start(mailbox: &Path, args: &[String]) -> Start {
    start_within(mailbox, args, REACH)
}

fn start_within(mailbox: &Path, args: &[String], reach: Duration) -> Start {
    let (lock_path, info_path) = (beside(mailbox, "lock"), beside(mailbox, "instance"));
    if let Some(dir) = lock_path.parent() {
        let _ = mailcraft_engine::store::create_private_dir_all(dir);
    }
    let lock = match OpenOptions::new().create(true).truncate(false).write(true).open(&lock_path) {
        Ok(f) => f,
        // Nowhere to keep a lock: there is nothing to guard with, so start as if alone.
        Err(e) => {
            log::warn!("no lock file {} ({e}): not guarding against a second window", lock_path.display());
            return Start::First(unguarded());
        }
    };
    match lock.try_lock() {
        Ok(()) => Start::First(first(lock, info_path)),
        Err(TryLockError::WouldBlock) => hand_over(&info_path, args, reach),
        // A file system without locks: start as if alone, as before there was a guard.
        Err(TryLockError::Error(e)) => {
            log::warn!("{} can't be locked ({e}): not guarding against a second window", lock_path.display());
            Start::First(unguarded())
        }
    }
}

fn unguarded() -> Guard {
    Guard { _lock: None, info: None, rx: channel().1, wake: Default::default() }
}

/// Listen for later starts. Without a port, a key or the file that tells them, the lock is still
/// held: a later start can't reach this one and says so instead of opening the mailbox twice.
fn first(lock: File, info_path: PathBuf) -> Guard {
    let (tx, rx) = channel();
    let wake: Arc<Mutex<Option<egui::Context>>> = Default::default();
    let mut guard = Guard { _lock: Some(lock), info: None, rx, wake: wake.clone() };
    // A file left by a run that didn't end cleanly names a port nobody answers on.
    let _ = std::fs::remove_file(&info_path);
    let listening = TcpListener::bind(("127.0.0.1", 0)).and_then(|l| l.local_addr().map(|a| (l, a.port()))).map_err(|e| e.to_string());
    let (listener, port, key) = match listening.and_then(|(l, port)| random_key().map(|key| (l, port, key))) {
        Ok(ready) => ready,
        Err(e) => {
            log::error!("a second MailCraft won't be able to reach this one: {e}");
            return guard;
        }
    };
    if let Err(e) = write_key_file(&info_path, &json!({"port": port, "key": key}).to_string()) {
        log::error!("a second MailCraft won't be able to reach this one: {e}");
        return guard;
    }
    guard.info = Some(info_path);
    let key: Arc<str> = key.into();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let (tx, wake, key) = (tx.clone(), wake.clone(), key.clone());
            std::thread::spawn(move || serve(stream, &tx, &wake, &key));
        }
    });
    guard
}

/// One line, at most [`MAX_LINE`] long; `None` for anything else.
fn read_line(stream: &TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let n = BufReader::new(stream).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut buf).ok()?;
    if n == 0 || buf.len() > MAX_LINE {
        return None;
    }
    String::from_utf8(buf).ok()
}

/// A later start's one request: its key and what it was started with.
fn serve(mut stream: TcpStream, tx: &Sender<Vec<String>>, wake: &Mutex<Option<egui::Context>>, key: &str) {
    let _ = stream.set_read_timeout(Some(ANSWER));
    let _ = stream.set_write_timeout(Some(ANSWER));
    let Some(request) = read_line(&stream).and_then(|line| serde_json::from_str::<Value>(&line).ok()) else { return };
    if !keys_match(request.get("key").and_then(Value::as_str).unwrap_or(""), key) {
        let _ = writeln!(stream, "{}", json!({"ok": false}));
        return;
    }
    let args: Vec<String> = request
        .get("open")
        .and_then(Value::as_array)
        .map(|list| {
            list.iter().filter_map(Value::as_str).filter(|a| !a.is_empty() && a.len() <= MAX_ARG).take(MAX_ARGS).map(str::to_string).collect()
        })
        .unwrap_or_default();
    if tx.send(args).is_err() {
        let _ = writeln!(stream, "{}", json!({"ok": false}));
        return;
    }
    if let Some(ctx) = wake.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
        ctx.request_repaint();
    }
    let _ = writeln!(stream, "{}", json!({"ok": true}));
}

/// A file named on the command line is named from where this start was run: the first instance
/// runs somewhere else, so it gets the whole path. Links are left as they are.
fn whole(arg: &str) -> String {
    if arg.contains("://") || arg.get(..7).is_some_and(|scheme| scheme.eq_ignore_ascii_case("mailto:")) {
        return arg.to_string();
    }
    std::path::absolute(arg).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| arg.to_string())
}

/// Give `args` to the instance `info_path` tells of, trying for as long as `reach`.
fn hand_over(info_path: &Path, args: &[String], reach: Duration) -> Start {
    let args: Vec<String> = args.iter().map(|a| whole(a)).collect();
    let until = Instant::now() + reach;
    let mut why;
    loop {
        match try_hand_over(info_path, &args) {
            Ok(()) => return Start::Handed,
            Err(e) => why = e,
        }
        if Instant::now() >= until {
            return Start::Unreachable(why);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

fn try_hand_over(info_path: &Path, args: &[String]) -> Result<(), String> {
    let info = std::fs::read(info_path).map_err(|e| format!("{}: {e}", info_path.display()))?;
    let info: Value = serde_json::from_slice(&info).map_err(|e| format!("{}: {e}", info_path.display()))?;
    let port = info.get("port").and_then(Value::as_u64).and_then(|p| u16::try_from(p).ok()).filter(|p| *p != 0).ok_or("no port")?;
    let key = info.get("key").and_then(Value::as_str).ok_or("no key")?;
    let mut stream = TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), ANSWER).map_err(|e| format!("127.0.0.1:{port}: {e}"))?;
    let _ = stream.set_read_timeout(Some(ANSWER));
    let _ = stream.set_write_timeout(Some(ANSWER));
    writeln!(stream, "{}", json!({"key": key, "open": args})).map_err(|e| e.to_string())?;
    let answer = read_line(&stream).and_then(|line| serde_json::from_str::<Value>(&line).ok()).ok_or("no answer")?;
    if answer.get("ok").and_then(Value::as_bool) == Some(true) { Ok(()) } else { Err("it refused".to_string()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_mailbox(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mailcraft-instance-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("mailbox.mailcraft")
    }

    fn first_of(start: Start) -> Guard {
        match start {
            Start::First(g) => g,
            Start::Handed => panic!("handed over"),
            Start::Unreachable(e) => panic!("unreachable: {e}"),
        }
    }

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_second_start_hands_over_to_the_first_and_the_next_first_starts_afresh() {
        let mailbox = temp_mailbox("hand");
        let first = first_of(start(&mailbox, &[]));
        assert!(beside(&mailbox, "lock").exists() && beside(&mailbox, "instance").exists());
        assert_eq!(first.handed_over(), None, "nobody asked yet");

        // A later start with a link and a file: the first gets both, the file by its whole path.
        let link = "mailto:maya@lumen.example?subject=Hi";
        assert!(matches!(start(&mailbox, &strings(&[link, "note.eml"])), Start::Handed));
        let got = first.handed_over().unwrap();
        assert_eq!(got.first().map(String::as_str), Some(link));
        let file = Path::new(got.get(1).unwrap());
        assert!(file.is_absolute() && file.ends_with("note.eml"), "{file:?}");
        // A later start with nothing still asks for the window.
        assert!(matches!(start(&mailbox, &[]), Start::Handed));
        assert_eq!(first.handed_over(), Some(vec![]));
        assert_eq!(first.handed_over(), None);

        // The first closes: the next start is a first again, even over what the last one left.
        first.close();
        assert!(!beside(&mailbox, "instance").exists());
        drop(first);
        std::fs::write(beside(&mailbox, "instance"), "left behind").unwrap();
        let again = first_of(start(&mailbox, &[]));
        assert!(matches!(start(&mailbox, &strings(&["a.ics"])), Start::Handed));
        assert_eq!(again.handed_over().map(|a| a.len()), Some(1));
        drop(again);
        let _ = std::fs::remove_dir_all(mailbox.parent().unwrap());
    }

    #[test]
    fn mailboxes_are_guarded_one_by_one() {
        let (a, b) = (temp_mailbox("one"), temp_mailbox("two"));
        let (first_a, first_b) = (first_of(start(&a, &[])), first_of(start(&b, &[])));
        assert!(matches!(start(&a, &strings(&["x.eml"])), Start::Handed));
        assert_eq!(first_b.handed_over(), None);
        assert!(first_a.handed_over().is_some());
        for m in [a, b] {
            let _ = std::fs::remove_dir_all(m.parent().unwrap());
        }
    }

    #[test]
    fn a_first_that_cannot_be_reached_is_said_so_and_the_mailbox_is_not_opened_twice() {
        let mailbox = temp_mailbox("deaf");
        let first = first_of(start(&mailbox, &[]));
        // No word of where it listens, then a port nobody answers on, then a wrong key.
        std::fs::remove_file(beside(&mailbox, "instance")).unwrap();
        assert!(matches!(start_within(&mailbox, &[], Duration::from_millis(300)), Start::Unreachable(_)));
        let closed = TcpListener::bind(("127.0.0.1", 0)).unwrap().local_addr().unwrap().port();
        std::fs::write(beside(&mailbox, "instance"), json!({"port": closed, "key": "k"}).to_string()).unwrap();
        assert!(matches!(start_within(&mailbox, &[], Duration::from_millis(300)), Start::Unreachable(_)));
        for junk in ["", "{", "[]", r#"{"port": 0, "key": "k"}"#, r#"{"port": 70000, "key": "k"}"#, r#"{"port": "x"}"#] {
            std::fs::write(beside(&mailbox, "instance"), junk).unwrap();
            assert!(matches!(start_within(&mailbox, &[], Duration::from_millis(50)), Start::Unreachable(_)), "{junk}");
        }
        assert_eq!(first.handed_over(), None);
        drop(first);
        let _ = std::fs::remove_dir_all(mailbox.parent().unwrap());
    }

    #[test]
    fn only_the_key_holder_is_listened_to() {
        let mailbox = temp_mailbox("key");
        let first = first_of(start(&mailbox, &[]));
        let info: Value = serde_json::from_slice(&std::fs::read(beside(&mailbox, "instance")).unwrap()).unwrap();
        let (port, key) = (u16::try_from(info["port"].as_u64().unwrap()).unwrap(), info["key"].as_str().unwrap().to_string());
        assert_eq!(key.len(), 64);
        let ask = |line: String| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            s.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let _ = s.write_all(line.as_bytes());
            let mut answer = String::new();
            let _ = BufReader::new(s).read_line(&mut answer);
            answer
        };
        // No key, a wrong key, junk, a line that never ends: nothing reaches the window.
        assert!(ask(format!("{}\n", json!({"open": ["mailto:x@evil.example"]}))).contains("false"));
        assert!(ask(format!("{}\n", json!({"key": "0".repeat(64), "open": ["mailto:x@evil.example"]}))).contains("false"));
        assert_eq!(ask("GET / HTTP/1.1\r\n\r\n".to_string()), "");
        assert_eq!(ask("x".repeat(MAX_LINE + 10)), "");
        assert_eq!(first.handed_over(), None);
        // The key holder: what it names is taken, within bounds.
        let many: Vec<String> = (0..MAX_ARGS + 50).map(|i| format!("{i}.eml")).collect();
        assert!(ask(format!("{}\n", json!({"key": key, "open": many}))).contains("true"));
        assert_eq!(first.handed_over().map(|a| a.len()), Some(MAX_ARGS));
        assert!(ask(format!("{}\n", json!({"key": key, "open": ["", 7, "x".repeat(MAX_ARG + 1), "ok.vcf"]}))).contains("true"));
        assert_eq!(first.handed_over(), Some(strings(&["ok.vcf"])));
        drop(first);
        let _ = std::fs::remove_dir_all(mailbox.parent().unwrap());
    }
}

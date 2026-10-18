//! The word index on its own: what it finds, and its file read back, cut short, damaged, of
//! another version, or junk (R6.2).

use std::path::PathBuf;
use std::sync::Arc;

use mailcraft_model::{Address, Attachment, Body, Message, words};

use super::*;

/// A folder of its own under the system's temporary folder, removed afterwards.
pub(crate) struct Temp(PathBuf);

impl Temp {
    pub(crate) fn new(name: &str) -> Temp {
        static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("mailcraft-words-{}-{n}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }

    pub(crate) fn file(&self) -> PathBuf {
        self.0.join("mailbox.mailcraft.words")
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn message(id: u64, subject: &str, from: &str, text: &str) -> Message {
    Message {
        id,
        subject: subject.into(),
        from: Address::new(from, format!("{}@example.org", from.to_lowercase())),
        to: vec![Address::new("Avery Quinn", "avery@mail.example")],
        body: Body { text: text.into(), html: None },
        ..Default::default()
    }
}

fn messages() -> Vec<Message> {
    let mut with_file = message(3, "Figures", "Kit", "See the attached sheet.");
    with_file.attachments = vec![Attachment::new("estimate-Q4.xlsx", "application/octet-stream", vec![1, 2, 3])];
    vec![
        message(1, "Meeting on Tuesday", "Ada", "The budget review meets at noon in the Gvožđar room."),
        message(2, "Re: Budget", "Bram", "Numbers attached. Café at three?"),
        with_file,
    ]
}

fn filled(file: Option<PathBuf>) -> Index {
    let mut index = Index::new(file);
    for m in messages() {
        index.insert(Doc::of(&m, &m.body.text), None);
    }
    index
}

/// The message ids `index` finds for `query` (its words) in `fields`.
fn found(index: &Index, query: &str, fields: u8) -> Vec<Id> {
    let bits = index.matching(&words::words(query), fields);
    let mut ids: Vec<Id> = index.ids().filter(|id| index.slot_of(*id).is_some_and(|s| bits.has(s))).collect();
    ids.sort_unstable();
    ids
}

#[test]
fn it_finds_word_starts_in_the_parts_asked_for() {
    let index = filled(None);
    assert_eq!(index.len(), 3);
    assert_eq!(found(&index, "meet", words::ALL), [1]);
    assert_eq!(found(&index, "budget", words::ALL), [1, 2]);
    assert_eq!(found(&index, "budget", words::SUBJECT), [2]);
    assert_eq!(found(&index, "budget", words::BODY), [1]);
    assert_eq!(found(&index, "gvozdar", words::ALL), [1]);
    assert_eq!(found(&index, "cafe", words::BODY), [2]);
    assert_eq!(found(&index, "ada", words::FROM), [1]);
    assert_eq!(found(&index, "avery", words::TO), [1, 2, 3]);
    assert_eq!(found(&index, "avery", words::FROM), Vec::<Id>::new());
    assert_eq!(found(&index, "estimate q4", words::ATTACHMENT), [3]);
    assert_eq!(found(&index, "udget", words::ALL), Vec::<Id>::new());
    assert_eq!(found(&index, "zyxwvut", words::ALL), Vec::<Id>::new());
    assert_eq!(found(&index, "", words::ALL), [1, 2, 3]);
}

#[test]
fn a_message_put_in_again_replaces_what_it_held_and_one_taken_out_goes() {
    let mut index = filled(None);
    let mut m = message(2, "Re: Invoice", "Bram", "Paid.");
    let doc = Doc::of(&m, &m.body.text);
    assert!(!index.current(2, fingerprint(&m)));
    index.insert(doc, None);
    assert!(index.current(2, fingerprint(&m)));
    assert_eq!(found(&index, "budget", words::ALL), [1]);
    assert_eq!(found(&index, "invoice", words::ALL), [2]);
    index.remove(1);
    assert_eq!(found(&index, "budget", words::ALL), Vec::<Id>::new());
    assert_eq!(index.len(), 2);
    // A flag or a folder isn't read: the same fingerprint.
    let before = fingerprint(&m);
    (m.read, m.folder, m.flag) = (true, 9, mailcraft_model::FlagStatus::Flagged);
    assert_eq!(fingerprint(&m), before);
    m.to.push(Address::new("Kit", "kit@example.org"));
    assert_ne!(fingerprint(&m), before);
}

#[test]
fn a_body_kept_on_disk_has_the_fingerprint_it_has_in_memory() {
    let m = message(4, "Long", "Ada", "A long text that lives in the content folder.");
    let r = mailcraft_model::ContentRef { sha256: sha256(m.body.text.as_bytes()), len: m.body.text.len() as u64 };
    let mut lean = m.clone();
    lean.body.text.clear();
    lean.kept = Some(Box::new(mailcraft_model::KeptBody { text: Some(r), ..Default::default() }));
    assert_eq!(fingerprint(&lean), fingerprint(&m));
    assert!(body_in_memory(&lean).is_none() && body_in_memory(&m).is_some());
    // Its reading failed and its text says so: still the file it is kept in.
    lean.kept.as_mut().unwrap().failed = true;
    lean.body.text = crate::store_placeholder().into();
    assert_eq!(fingerprint(&lean), fingerprint(&m));
}

#[test]
fn the_file_reads_back_as_it_was_written() {
    let tmp = Temp::new("round-trip");
    let file = tmp.file();
    let mut index = filled(Some(file.clone()));
    index.flush();
    index.remove(3);
    index.flush();
    let again = Index::open(&file).unwrap();
    assert_eq!((again.len(), again.word_count()), (2, index.word_count()));
    for (query, fields) in [("budget", words::ALL), ("meet", words::ALL), ("ada", words::FROM), ("estimate", words::ALL), ("cafe", words::BODY)] {
        assert_eq!(found(&again, query, fields), found(&index, query, fields), "{query}");
    }
    let m = &messages()[0];
    assert!(again.current(1, fingerprint(m)));
    // Written whole again: what was taken out is gone from the file too.
    let mut again = again;
    again.remove(2);
    again.compact();
    let bytes = std::fs::read(&file).unwrap();
    assert!(!bytes.windows(4).any(|w| w == b"cafe"), "a word only the removed message had");
    let third = Index::open(&file).unwrap();
    assert_eq!((third.len(), found(&third, "budget", words::ALL)), (1, vec![1]));
    // And goes on from there.
    let mut third = third;
    let m = message(7, "Picnic", "Oona", "Bring sandwiches.");
    third.insert(Doc::of(&m, &m.body.text), None);
    third.flush();
    let fourth = Index::open(&file).unwrap();
    assert_eq!((fourth.len(), found(&fourth, "sandwich", words::ALL)), (2, vec![7]));
    assert_eq!(found(&fourth, "meet", words::ALL), [1]);
}

#[test]
fn a_file_cut_short_keeps_what_was_whole_and_goes_on_after_it() {
    let tmp = Temp::new("cut-short");
    let file = tmp.file();
    let mut index = Index::new(Some(file.clone()));
    let ms = messages();
    index.insert(Doc::of(&ms[0], &ms[0].body.text), None);
    index.flush();
    let whole = std::fs::metadata(&file).unwrap().len();
    index.insert(Doc::of(&ms[1], &ms[1].body.text), None);
    index.flush();
    let len = std::fs::metadata(&file).unwrap().len();
    // The program stopped while the second message was being written.
    let f = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
    f.set_len(len - 3).unwrap();
    drop(f);
    let mut again = Index::open(&file).unwrap();
    assert_eq!((again.len(), found(&again, "meet", words::ALL)), (1, vec![1]));
    assert!(again.file_len >= whole && again.file_len < len - 3, "{} {whole} {len}", again.file_len);
    again.insert(Doc::of(&ms[2], &ms[2].body.text), None);
    again.flush();
    let third = Index::open(&file).unwrap();
    assert_eq!(found(&third, "figures", words::ALL), [3]);
    assert_eq!(third.len(), 2);
}

#[test]
fn a_damaged_file_or_one_of_another_version_is_not_believed() {
    let tmp = Temp::new("damaged");
    let file = tmp.file();
    let mut index = filled(Some(file.clone()));
    index.flush();
    let bytes = std::fs::read(&file).unwrap();
    // A byte changed in the middle of a record.
    let mut damaged = bytes.clone();
    let at = damaged.len() / 2;
    damaged[at] ^= 0x55;
    std::fs::write(&file, &damaged).unwrap();
    assert!(Index::open(&file).is_err());
    // Another version.
    let mut other = bytes.clone();
    other[8] = 9;
    std::fs::write(&file, &other).unwrap();
    assert!(Index::open(&file).unwrap_err().contains("version"));
    // Not an index at all.
    std::fs::write(&file, b"{\"messages\": []}                   ").unwrap();
    assert!(Index::open(&file).is_err());
    // Nothing there: an empty index.
    std::fs::remove_file(&file).unwrap();
    assert!(Index::open(&file).unwrap().is_empty());
}

/// Records whose checksums are right but whose contents lie: never believed, never a panic.
#[test]
fn records_that_lie_are_not_believed() {
    let tmp = Temp::new("lying");
    let file = tmp.file();
    let with = |records: &[(u8, Vec<u8>)]| {
        let mut out = header().to_vec();
        for (kind, p) in records {
            record(&mut out, *kind, p);
        }
        std::fs::write(&file, out).unwrap();
        Index::open(&file)
    };
    let words_rec = |ws: &[&str]| {
        let mut p = Vec::new();
        put_varint(&mut p, ws.len() as u64);
        for w in ws {
            put_varint(&mut p, w.len() as u64);
            p.extend_from_slice(w.as_bytes());
        }
        (WORDS, p)
    };
    let doc = |id: u64, entries: &[(u64, u8)]| {
        let mut p = Vec::new();
        p.extend_from_slice(&id.to_le_bytes());
        p.extend_from_slice(&0u64.to_le_bytes());
        put_varint(&mut p, entries.len() as u64);
        for (w, mask) in entries {
            put_varint(&mut p, *w);
            p.push(*mask);
        }
        (DOC, p)
    };
    assert!(with(&[words_rec(&["one", "two"]), doc(1, &[(0, 1), (1, 16)])]).is_ok());
    // A word number past the words.
    assert!(with(&[words_rec(&["one"]), doc(1, &[(5, 1)])]).is_err());
    // The same word twice in one message, a mask of no part, a part that doesn't exist.
    assert!(with(&[words_rec(&["one"]), doc(1, &[(0, 1), (0, 2)])]).is_err());
    assert!(with(&[words_rec(&["one"]), doc(1, &[(0, 0)])]).is_err());
    assert!(with(&[words_rec(&["one"]), doc(1, &[(0, 64)])]).is_err());
    // A word twice, an empty word, one too long, bytes that aren't text.
    assert!(with(&[words_rec(&["one", "one"])]).is_err());
    assert!(with(&[words_rec(&[""])]).is_err());
    assert!(with(&[words_rec(&[&"x".repeat(words::MAX_WORD + 1)])]).is_err());
    assert!(with(&[(WORDS, vec![1, 2, 0xff, 0xfe])]).is_err());
    // A message taken out that isn't there; postings past the messages, or not rising.
    let mut kill = Vec::new();
    put_varint(&mut kill, 1);
    put_varint(&mut kill, 4);
    assert!(with(&[(KILL, kill)]).is_err());
    let postings = |bytes: &[u8]| {
        let mut p = Vec::new();
        put_varint(&mut p, 1);
        put_varint(&mut p, 0);
        put_varint(&mut p, bytes.len() as u64);
        p.extend_from_slice(bytes);
        (POSTINGS, p)
    };
    let mut slots = Vec::new();
    put_varint(&mut slots, 2);
    for id in [1u64, 2] {
        slots.extend_from_slice(&id.to_le_bytes());
        slots.extend_from_slice(&0u64.to_le_bytes());
        slots.push(1);
    }
    let index = with(&[words_rec(&["one"]), (SLOTS, slots.clone()), postings(&[0, 1, 1, 16])]).unwrap();
    assert_eq!(found(&index, "one", words::ALL), [1, 2]);
    assert_eq!(found(&index, "one", words::BODY), [2]);
    assert!(with(&[words_rec(&["one"]), (SLOTS, slots.clone()), postings(&[0, 1, 5, 1])]).is_err());
    assert!(with(&[words_rec(&["one"]), (SLOTS, slots.clone()), postings(&[1, 1, 0, 1])]).is_err());
    // An unknown kind of record, and bytes left over after one.
    assert!(with(&[(42, vec![])]).is_err());
    let (kind, mut p) = words_rec(&["one"]);
    p.push(0);
    assert!(with(&[(kind, p)]).is_err());
}

#[test]
fn a_message_counts_its_words_up_to_the_limit() {
    let text: String = (0..MAX_DOC_WORDS + 500).map(|n| format!("w{n} ")).collect();
    let m = message(1, "Many", "Ada", &text);
    let doc = Doc::of(&m, &m.body.text);
    assert_eq!(doc.words.len(), MAX_DOC_WORDS);
}

#[test]
fn a_message_seen_is_known_by_itself_until_it_is_replaced() {
    let mut index = Index::new(None);
    let m = Arc::new(messages().remove(0));
    index.insert(Doc::of(&m, &m.body.text), Some(&m));
    assert!(index.seen(&m));
    let copy = Arc::new(Message::clone(&m));
    assert!(!index.seen(&copy));
    index.note_seen(&copy);
    assert!(index.seen(&copy) && !index.seen(&m));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig { cases: 256, ..Default::default() })]

    /// Whatever the file holds, opening it says what is wrong or reads what is right: it never
    /// panics.
    #[test]
    fn junk_never_panics(junk in proptest::collection::vec(proptest::prelude::any::<u8>(), 0..2048), keep_header in proptest::prelude::any::<bool>()) {
        let tmp = Temp::new("junk");
        let file = tmp.file();
        let mut bytes = if keep_header { header().to_vec() } else { Vec::new() };
        bytes.extend_from_slice(&junk);
        std::fs::write(&file, &bytes).unwrap();
        if let Ok(mut index) = Index::open(&file) {
            let _ = index.matching(&words::words("a b"), words::ALL);
            let m = message(1, "x", "y", "z");
            index.insert(Doc::of(&m, &m.body.text), None);
            index.flush();
            index.compact();
        }
    }
}

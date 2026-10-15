//! The operating system's credential store: Windows Credential Manager, the macOS keychain, and
//! on Linux and FreeBSD the desktop's Secret Service (GNOME Keyring, KWallet…) over D-Bus.
//!
//! A password or a refresh token the owner asks to be remembered is kept here, encrypted by the
//! system for this user, and nowhere else: never in the mailbox file, the settings or a log.
//! When the store can't be used (no Secret Service is running, or it stays locked), nothing is
//! remembered between runs, as on systems without a store, and the window is told why
//! ([`SecretStore::take_trouble`]) so that it can say so once.

use mailcraft_ui_egui::SecretStore;

/// The store on systems that have one.
pub fn store() -> Option<Box<dyn SecretStore>> {
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd"))]
    {
        Some(Box::new(os::OsStore::default()))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd")))]
    {
        None
    }
}

/// Windows keeps at most 2560 bytes per credential (1280 UTF-16 units), and a refresh token can
/// be longer, so a secret is kept in pieces: `<key>#n` holds how many, `<key>#0`… hold them.
/// (The same on every system, so that what is kept looks alike everywhere.)
#[cfg_attr(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd", test)), allow(dead_code))]
const PIECE: usize = 1000;
/// More pieces than any secret needs; a count beyond it is damage, not a secret.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd", test)), allow(dead_code))]
const MAX_PIECES: usize = 16;

/// `secret` cut at character boundaries into pieces of at most [`PIECE`] characters.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd", test)), allow(dead_code))]
fn pieces(secret: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = secret.chars().collect();
    let pieces: Vec<String> = chars.chunks(PIECE).map(|c| c.iter().collect()).collect();
    (pieces.len() <= MAX_PIECES).then_some(pieces)
}

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd"))]
mod os {
    use std::sync::Mutex;

    use keyring::Entry;
    use mailcraft_ui_egui::{SecretStore, StoreTrouble};

    use super::{MAX_PIECES, pieces};

    /// What the entries are filed under in the system's store.
    const SERVICE: &str = "MailCraft";

    /// Whether an error says the store itself can't be used, and how, rather than that nothing is
    /// kept under a key or that one entry is unreadable. Decided from the error's kind alone,
    /// never from its text.
    pub(super) fn trouble(e: &keyring::Error) -> Option<StoreTrouble> {
        match e {
            // No store answered: on Linux no Secret Service on the session bus, or no session bus.
            keyring::Error::PlatformFailure(_) => Some(StoreTrouble::Unavailable),
            // The store is there but locked, and unlocking it was dismissed or refused.
            keyring::Error::NoStorageAccess(_) => Some(StoreTrouble::Locked),
            _ => None,
        }
    }

    #[derive(Default)]
    pub struct OsStore {
        /// Why the store couldn't be used the last time, until the window takes it.
        trouble: Mutex<Option<StoreTrouble>>,
    }

    impl OsStore {
        /// Remember why the store couldn't be used, when that is what `e` says.
        pub(super) fn note(&self, e: &keyring::Error) {
            if let Some(t) = trouble(e)
                && let Ok(mut kept) = self.trouble.lock()
            {
                *kept = Some(t);
            }
        }

        fn entry(&self, key: &str, part: &str) -> Option<Entry> {
            Entry::new(SERVICE, &format!("{key}#{part}")).map_err(|e| self.note(&e)).ok()
        }

        fn read(&self, key: &str, part: &str) -> Option<String> {
            self.entry(key, part)?.get_password().map_err(|e| self.note(&e)).ok()
        }

        fn count(&self, key: &str) -> Option<usize> {
            self.read(key, "n")?.parse::<usize>().ok().filter(|n| (1..=MAX_PIECES).contains(n))
        }
    }

    impl SecretStore for OsStore {
        fn get(&self, key: &str) -> Option<String> {
            let mut secret = String::new();
            for i in 0..self.count(key)? {
                secret.push_str(&self.read(key, &i.to_string())?);
            }
            Some(secret)
        }

        fn set(&self, key: &str, secret: &str) -> Result<(), String> {
            // The error text names the store's complaint, never the secret.
            let refused = |e: keyring::Error| {
                self.note(&e);
                format!("the system's credential store refused: {e}")
            };
            let pieces = pieces(secret).ok_or("it is too long for the system's credential store")?;
            self.delete(key);
            for (i, piece) in pieces.iter().enumerate() {
                Entry::new(SERVICE, &format!("{key}#{i}")).and_then(|e| e.set_password(piece)).map_err(refused)?;
            }
            Entry::new(SERVICE, &format!("{key}#n")).and_then(|e| e.set_password(&pieces.len().to_string())).map_err(refused)
        }

        fn delete(&self, key: &str) {
            let pieces = self.count(key).unwrap_or(0);
            for part in (0..pieces).map(|i| i.to_string()).chain(["n".to_string()]) {
                if let Some(e) = self.entry(key, &part) {
                    // Nothing kept under the key is what deleting wants anyway.
                    if let Err(err) = e.delete_credential() {
                        self.note(&err);
                    }
                }
            }
        }

        fn take_trouble(&self) -> Option<StoreTrouble> {
            self.trouble.lock().ok()?.take()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_long_secret_is_cut_at_character_boundaries_and_goes_back_together() {
        let secret: String = "töken-ñ".chars().cycle().take(PIECE * 2 + 7).collect();
        let cut = pieces(&secret).unwrap();
        assert_eq!(cut.len(), 3);
        assert!(cut.iter().all(|p| p.chars().count() <= PIECE));
        assert_eq!(cut.concat(), secret);
        assert_eq!(pieces("short").unwrap(), ["short"]);
        assert_eq!(pieces("").unwrap().len(), 0);
        // Longer than any secret: refused rather than filling the store.
        assert!(pieces(&"x".repeat(PIECE * MAX_PIECES + 1)).is_none());
    }

    /// What each of the store's complaints means for the owner, without a store to complain:
    /// only a store that can't be reached or stays locked is trouble worth telling.
    #[test]
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd"))]
    fn only_a_store_that_cant_be_used_is_trouble() {
        use mailcraft_ui_egui::StoreTrouble;
        use os::trouble;
        let boxed = |s: &str| -> Box<dyn std::error::Error + Send + Sync> { Box::new(std::io::Error::other(s.to_string())) };
        // As the Secret Service answers when none is running, and when the owner won't unlock it.
        let none_running = keyring::Error::PlatformFailure(boxed("org.freedesktop.DBus.Error.ServiceUnknown"));
        assert_eq!(trouble(&none_running), Some(StoreTrouble::Unavailable));
        assert_eq!(trouble(&keyring::Error::NoStorageAccess(boxed("prompt dismissed"))), Some(StoreTrouble::Locked));
        // Nothing kept, or one damaged entry: the store works, and there is nothing to say.
        assert_eq!(trouble(&keyring::Error::NoEntry), None);
        assert_eq!(trouble(&keyring::Error::BadEncoding(vec![0xff])), None);
        assert_eq!(trouble(&keyring::Error::TooLong("password".into(), 10)), None);
        assert_eq!(trouble(&keyring::Error::Invalid("user".into(), "empty".into())), None);
        assert_eq!(trouble(&keyring::Error::Ambiguous(Vec::new())), None);
    }

    /// The store remembers the last trouble until it is taken, and then has nothing to say.
    #[test]
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd"))]
    fn trouble_is_taken_once() {
        use mailcraft_ui_egui::StoreTrouble;
        let store = os::OsStore::default();
        assert_eq!(store.take_trouble(), None);
        let locked = keyring::Error::NoStorageAccess(Box::new(std::io::Error::other("locked")));
        store.note(&locked);
        // An entry that isn't there says nothing about the store, and leaves the trouble as it was.
        store.note(&keyring::Error::NoEntry);
        assert_eq!(store.take_trouble(), Some(StoreTrouble::Locked));
        assert_eq!(store.take_trouble(), None);
    }

    /// Against the real store of the machine running the test, with a made-up key that is removed
    /// again. Run it by name: `cargo test -p mailcraft -- --ignored the_systems_store`. On Linux it
    /// needs a Secret Service running and unlocked (GNOME Keyring, KWallet); without one, the
    /// store's trouble says so.
    #[test]
    #[ignore = "writes to (and cleans up in) this user's credential store"]
    #[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd"))]
    fn the_systems_store_keeps_a_long_secret_and_lets_it_go() {
        let store = store().unwrap();
        let key = "refresh:store-test@mailcraft.example";
        // Longer than one Windows credential holds, like a real refresh token.
        let secret: String = "0.AXoA-made_up.token~".chars().cycle().take(3_300).collect();
        store.delete(key);
        assert_eq!(store.get(key), None);
        assert_eq!(store.take_trouble(), None, "the store can't be used here");
        store.set(key, &secret).unwrap();
        assert_eq!(store.get(key).as_deref(), Some(secret.as_str()));
        // A shorter one over it leaves nothing of the longer one behind.
        store.set(key, "short").unwrap();
        assert_eq!(store.get(key).as_deref(), Some("short"));
        store.delete(key);
        assert_eq!(store.get(key), None);
        store.delete(key);
        assert_eq!(store.take_trouble(), None);
    }
}

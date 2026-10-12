//! The operating system's credential store: Windows Credential Manager and the macOS keychain.
//!
//! A password or a refresh token the owner asks to be remembered is kept here, encrypted by the
//! system for this user, and nowhere else: never in the mailbox file, the settings or a log.
//! Other systems have no store wired up yet, so nothing is remembered there between runs.

use mailcraft_ui_egui::SecretStore;

/// The store on systems that have one.
pub fn store() -> Option<Box<dyn SecretStore>> {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        Some(Box::new(os::OsStore))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        None
    }
}

/// Windows keeps at most 2560 bytes per credential (1280 UTF-16 units), and a refresh token can
/// be longer, so a secret is kept in pieces: `<key>#n` holds how many, `<key>#0`… hold them.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos", test)), allow(dead_code))]
const PIECE: usize = 1000;
/// More pieces than any secret needs; a count beyond it is damage, not a secret.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos", test)), allow(dead_code))]
const MAX_PIECES: usize = 16;

/// `secret` cut at character boundaries into pieces of at most [`PIECE`] characters.
#[cfg_attr(not(any(target_os = "windows", target_os = "macos", test)), allow(dead_code))]
fn pieces(secret: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = secret.chars().collect();
    let pieces: Vec<String> = chars.chunks(PIECE).map(|c| c.iter().collect()).collect();
    (pieces.len() <= MAX_PIECES).then_some(pieces)
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod os {
    use keyring::Entry;
    use mailcraft_ui_egui::SecretStore;

    use super::{MAX_PIECES, pieces};

    /// What the entries are filed under in the system's store.
    const SERVICE: &str = "MailCraft";

    pub struct OsStore;

    fn entry(key: &str, part: &str) -> Option<Entry> {
        Entry::new(SERVICE, &format!("{key}#{part}")).ok()
    }

    fn count(key: &str) -> Option<usize> {
        entry(key, "n")?.get_password().ok()?.parse::<usize>().ok().filter(|n| (1..=MAX_PIECES).contains(n))
    }

    impl SecretStore for OsStore {
        fn get(&self, key: &str) -> Option<String> {
            let mut secret = String::new();
            for i in 0..count(key)? {
                secret.push_str(&entry(key, &i.to_string())?.get_password().ok()?);
            }
            Some(secret)
        }

        fn set(&self, key: &str, secret: &str) -> Result<(), String> {
            // The error text names the store's complaint, never the secret.
            let refused = |e: keyring::Error| format!("the system's credential store refused: {e}");
            let pieces = pieces(secret).ok_or("it is too long for the system's credential store")?;
            self.delete(key);
            for (i, piece) in pieces.iter().enumerate() {
                Entry::new(SERVICE, &format!("{key}#{i}")).and_then(|e| e.set_password(piece)).map_err(refused)?;
            }
            Entry::new(SERVICE, &format!("{key}#n")).and_then(|e| e.set_password(&pieces.len().to_string())).map_err(refused)
        }

        fn delete(&self, key: &str) {
            let pieces = count(key).unwrap_or(0);
            for part in (0..pieces).map(|i| i.to_string()).chain(["n".to_string()]) {
                if let Some(e) = entry(key, &part) {
                    let _ = e.delete_credential();
                }
            }
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

    /// Against the real store of the machine running the test, with a made-up key that is removed
    /// again. Run it by name: `cargo test -p mailcraft -- --ignored the_systems_store`.
    #[test]
    #[ignore = "writes to (and cleans up in) this user's credential store"]
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    fn the_systems_store_keeps_a_long_secret_and_lets_it_go() {
        let store = store().unwrap();
        let key = "refresh:store-test@mailcraft.example";
        // Longer than one Windows credential holds, like a real refresh token.
        let secret: String = "0.AXoA-made_up.token~".chars().cycle().take(3_300).collect();
        store.delete(key);
        assert_eq!(store.get(key), None);
        store.set(key, &secret).unwrap();
        assert_eq!(store.get(key).as_deref(), Some(secret.as_str()));
        // A shorter one over it leaves nothing of the longer one behind.
        store.set(key, "short").unwrap();
        assert_eq!(store.get(key).as_deref(), Some("short"));
        store.delete(key);
        assert_eq!(store.get(key), None);
        store.delete(key);
    }
}

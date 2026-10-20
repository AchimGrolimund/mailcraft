// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface text. Every string the interface shows goes through [`t`] (the `tl!` macro), keyed by
//! its English text, so language catalogs can be added without touching the widgets. English is
//! the only language so far; `pseudo` (a test language) proves that no string bypasses the lookup.

use std::sync::atomic::{AtomicU8, Ordering};

/// The interface language setting that follows the system.
pub const AUTO: &str = "auto";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    English,
    /// Accented look-alikes of the English text, for finding strings that skip translation.
    Pseudo,
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

impl Lang {
    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Pseudo => "pseudo",
        }
    }

    /// The language for a saved setting; anything unknown is English.
    pub fn from_pref(pref: &str) -> Lang {
        if pref.trim().eq_ignore_ascii_case("pseudo") { Lang::Pseudo } else { Lang::English }
    }
}

pub fn set_current(lang: Lang) {
    CURRENT.store(lang as u8, Ordering::Relaxed);
}

pub fn current() -> Lang {
    if CURRENT.load(Ordering::Relaxed) == Lang::Pseudo as u8 { Lang::Pseudo } else { Lang::English }
}

/// `s` in the interface language.
pub fn t(s: &str) -> &str {
    match current() {
        Lang::English => s,
        Lang::Pseudo => pseudo(s),
    }
}

/// The pseudo-language form of `s`, interned so it can be handed out as `&'static str`.
fn pseudo(s: &str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<String, &'static str>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if let Some(done) = cache.get(s) {
        return done;
    }
    // `{placeholders}` stay as they are.
    let mut out = String::with_capacity(s.len() + 2);
    let mut inside = false;
    for c in s.chars() {
        match c {
            '{' => inside = true,
            '}' => inside = false,
            _ => {}
        }
        out.push(if inside {
            c
        } else {
            match c {
                'a' => 'á',
                'e' => 'é',
                'i' => 'í',
                'o' => 'ó',
                'u' => 'ú',
                'A' => 'Á',
                'E' => 'É',
                'O' => 'Ó',
                'U' => 'Ú',
                _ => c,
            }
        });
    }
    // The catalog is small and fixed (interface strings), so leaking each form once is bounded.
    let leaked: &'static str = Box::leak(out.into_boxed_str());
    cache.insert(s.to_string(), leaked);
    leaked
}

/// Fill `{name}` placeholders in a (translated) template.
pub fn fmt(template: &str, args: &[(&str, &str)]) -> String {
    let mut out = template.to_string();
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

/// A command's location (`Home › Respond`) in the interface language, part by part.
pub fn location(loc: &str) -> String {
    loc.split(" › ").map(t).collect::<Vec<_>>().join(" › ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_and_pseudo() {
        assert_eq!(fmt("{n} of {total} items", &[("n", "3"), ("total", "10")]), "3 of 10 items");
        assert_eq!(pseudo("Reply {name}"), "Réply {name}");
        assert_eq!(Lang::from_pref("PSEUDO"), Lang::Pseudo);
        assert_eq!(Lang::from_pref("klingon"), Lang::English);
        assert_eq!(location("Home › Respond"), "Home › Respond");
    }
}

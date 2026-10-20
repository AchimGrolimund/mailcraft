// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Interface languages. Every text the window shows is looked up by its English wording in the
//! catalogue of the interface language: a gettext `.po` file in `po/`, compiled into the program
//! and read by [`po`], our own small reader. A text a catalogue lacks is shown in English, never
//! blank. Plurals follow the catalogue's `Plural-Forms` ([`plural`]); `{name}` placeholders are
//! filled with [`fmt`]. `pseudo`, a test language, proves that no text bypasses the lookup.
//!
//! Commands, their parameters, the CLI, MCP, the control channel and logs stay English: only what
//! the window shows a person is translated, and only what the host asks for ([`Lang`] is passed
//! in; there is no global here).

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod plural;
pub mod po;

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use plural::Rule;

mod files {
    include!(concat!(env!("OUT_DIR"), "/catalogues.rs"));
}

/// The interface language setting that follows the system's.
pub const AUTO: &str = "auto";

/// An interface language.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Lang {
    #[default]
    English,
    French,
    /// Accented look-alikes of the English text, for finding text that skips translation. Set by
    /// command only; never offered.
    Pseudo,
}

impl Lang {
    /// Every language there is a name for (whether or not it has a catalogue yet).
    pub const ALL: [Lang; 3] = [Lang::English, Lang::French, Lang::Pseudo];

    /// Its code: `en`, `fr`, `pseudo` (the settings' and the commands' value; the `.po` file's name).
    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::French => "fr",
            Lang::Pseudo => "pseudo",
        }
    }

    /// The language a code or a system locale names (`fr`, `fr-CA`, `fr_BE.UTF-8`, `pseudo`),
    /// whether or not it has a catalogue.
    pub fn from_code(code: &str) -> Option<Lang> {
        let code = code.trim();
        if code.eq_ignore_ascii_case("pseudo") {
            return Some(Lang::Pseudo);
        }
        let primary = code.split(['-', '_', '.', '@']).next().unwrap_or("");
        match primary.to_ascii_lowercase().as_str() {
            "en" => Some(Lang::English),
            "fr" => Some(Lang::French),
            _ => None,
        }
    }

    /// Its name in its own language, as the language picker shows it.
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::French => "Français",
            Lang::Pseudo => "Pseudo",
        }
    }

    /// No native speaker has reviewed its main screens yet: named "(preview)" where it is chosen.
    pub fn preview(self) -> bool {
        matches!(self, Lang::French)
    }

    /// English needs none; the others have one compiled in, or can't be chosen.
    pub fn has_catalogue(self) -> bool {
        match self {
            Lang::English | Lang::Pseudo => true,
            lang => files::FILES.iter().any(|(code, _)| *code == lang.code()),
        }
    }

    /// The languages offered to the owner: English, and each other one with a catalogue.
    pub fn available() -> Vec<Lang> {
        Lang::ALL.into_iter().filter(|l| *l != Lang::Pseudo && l.has_catalogue()).collect()
    }

    /// The language a system locale (`fr-FR`) asks for: English when there is no catalogue for it.
    pub fn from_locale(locale: Option<&str>) -> Lang {
        match locale.and_then(Lang::from_code) {
            Some(Lang::Pseudo) | None => Lang::English,
            Some(lang) if lang.has_catalogue() => lang,
            Some(_) => Lang::English,
        }
    }

    /// The language a setting (`auto`, `en`, `fr`, `pseudo`) means, with the system's locale for
    /// `auto`. Anything unknown, or a language without a catalogue, is English.
    pub fn from_pref(pref: &str, system: Option<&str>) -> Lang {
        let pref = pref.trim();
        if pref.is_empty() || pref.eq_ignore_ascii_case(AUTO) {
            return Lang::from_locale(system);
        }
        match Lang::from_code(pref) {
            Some(lang) if lang.has_catalogue() => lang,
            _ => Lang::English,
        }
    }

    /// `s` (English) in this language: English when the catalogue lacks it.
    pub fn t(self, s: &str) -> &str {
        match self {
            Lang::English => s,
            Lang::Pseudo => pseudo(s),
            lang => catalogue(lang).and_then(|c| c.get(s)).unwrap_or(s),
        }
    }

    /// The form of `one` / `other` (English singular and plural) a count of `n` takes in this
    /// language; its placeholders, `{n}` among them, are left to [`fmt`].
    pub fn tn<'a>(self, n: u64, one: &'a str, other: &'a str) -> &'a str {
        let english = if Rule::NotOne.index(n) == 0 { one } else { other };
        match self {
            Lang::English => english,
            Lang::Pseudo => pseudo(english),
            lang => catalogue(lang).and_then(|c| c.get_plural(one, n)).unwrap_or(english),
        }
    }
}

/// A language's catalogue, read once.
#[derive(Debug, Default)]
pub struct Catalogue {
    messages: HashMap<String, Message>,
    rule: Rule,
}

#[derive(Debug)]
struct Message {
    strs: Vec<String>,
    fuzzy: bool,
}

impl Catalogue {
    /// Read a catalogue: what can't be read is left out (and English shown for it).
    pub fn parse(text: &str) -> Catalogue {
        let file = po::parse(text);
        let rule = file.header().and_then(|h| h.header_field("Plural-Forms")).map(Rule::from_header).unwrap_or_default();
        let messages = file
            .entries
            .into_iter()
            .filter(|e| !e.obsolete && !e.is_header())
            .map(|e| {
                let fuzzy = e.fuzzy();
                (e.key(), Message { strs: e.strs, fuzzy })
            })
            .collect();
        Catalogue { messages, rule }
    }

    /// The translation of `id`, when it has one. Drafts (`fuzzy`) are used too.
    pub fn get(&self, id: &str) -> Option<&str> {
        self.messages.get(id)?.strs.first().map(String::as_str).filter(|s| !s.is_empty())
    }

    /// The form of the plural entry `one` for a count of `n`, when it is translated.
    pub fn get_plural(&self, one: &str, n: u64) -> Option<&str> {
        let i = self.rule.index(n);
        self.messages.get(one)?.strs.get(i).map(String::as_str).filter(|s| !s.is_empty())
    }

    /// Entries translated, and of those how many are still drafts (`fuzzy`).
    pub fn counts(&self) -> (usize, usize) {
        let done: Vec<&Message> = self.messages.values().filter(|m| !m.strs.is_empty() && m.strs.iter().all(|s| !s.is_empty())).collect();
        (done.len(), done.iter().filter(|m| m.fuzzy).count())
    }

    pub fn rule(&self) -> Rule {
        self.rule
    }
}

/// The compiled-in catalogue of `lang`, read the first time it is asked for.
pub fn catalogue(lang: Lang) -> Option<&'static Catalogue> {
    static READ: [OnceLock<Option<Catalogue>>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let slot = READ.get(Lang::ALL.iter().position(|l| *l == lang)?)?;
    slot.get_or_init(|| files::FILES.iter().find(|(code, _)| *code == lang.code()).map(|(_, text)| Catalogue::parse(text))).as_ref()
}

/// The pseudo-language form of `s`, kept so that it can be handed out as `&'static str`.
fn pseudo(s: &str) -> &'static str {
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
                'I' => 'Í',
                'O' => 'Ó',
                'U' => 'Ú',
                _ => c,
            }
        });
    }
    // Interface text is a fixed set (a test language), so keeping each form once is bounded.
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

/// A command's location (`Home › Respond`) in `lang`, part by part.
pub fn location(lang: Lang, loc: &str) -> String {
    loc.split(" › ").map(|part| lang.t(part)).collect::<Vec<_>>().join(" › ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_and_pseudo() {
        assert_eq!(fmt("{n} of {total} items", &[("n", "3"), ("total", "10")]), "3 of 10 items");
        assert_eq!(pseudo("Reply {name}"), "Réply {name}");
        assert_eq!(Lang::Pseudo.t("Inbox"), "Ínbóx");
        assert_eq!(location(Lang::English, "Home › Respond"), "Home › Respond");
        assert_eq!(location(Lang::Pseudo, "Home › Respond"), "Hómé › Réspónd");
    }

    #[test]
    fn settings_and_locales() {
        assert_eq!(Lang::from_pref("PSEUDO", None), Lang::Pseudo);
        assert_eq!(Lang::from_pref("klingon", None), Lang::English);
        assert_eq!(Lang::from_pref("auto", Some("en-GB")), Lang::English);
        // Automatic never picks the test language, nor one without a catalogue.
        assert_eq!(Lang::from_pref("auto", Some("pseudo")), Lang::English);
        assert_eq!(Lang::from_code("fr_CA.UTF-8"), Some(Lang::French));
        assert_eq!(Lang::from_code("de-DE"), None);
        let fr = if Lang::French.has_catalogue() { Lang::French } else { Lang::English };
        assert_eq!(Lang::from_pref("auto", Some("fr-BE")), fr);
        assert_eq!(Lang::from_pref("fr", None), fr);
        assert_eq!(Lang::available().contains(&Lang::French), Lang::French.has_catalogue());
        assert!(Lang::available().contains(&Lang::English) && !Lang::available().contains(&Lang::Pseudo));
    }

    #[test]
    fn english_plurals() {
        assert_eq!(Lang::English.tn(1, "{n} message", "{n} messages"), "{n} message");
        assert_eq!(Lang::English.tn(0, "{n} message", "{n} messages"), "{n} messages");
        assert_eq!(Lang::Pseudo.tn(2, "{n} message", "{n} messages"), "{n} mésságés");
    }

    #[test]
    fn a_catalogue_translates_and_falls_back_to_english() {
        let c = Catalogue::parse(
            "msgid \"\"\nmsgstr \"Plural-Forms: nplurals=2; plural=(n > 1);\\n\"\n\n#, fuzzy\nmsgid \"Reply\"\nmsgstr \"Répondre\"\n\nmsgid \"Empty\"\nmsgstr \"\"\n\nmsgid \"{n} message\"\nmsgid_plural \"{n} messages\"\nmsgstr[0] \"{n} message\"\nmsgstr[1] \"{n} messages\"\n",
        );
        assert_eq!(c.rule(), Rule::AboveOne);
        assert_eq!(c.get("Reply"), Some("Répondre"));
        assert_eq!(c.get("Empty"), None);
        assert_eq!(c.get("Missing"), None);
        assert_eq!(c.get_plural("{n} message", 0), Some("{n} message"));
        assert_eq!(c.get_plural("{n} message", 2), Some("{n} messages"));
        assert_eq!(c.counts(), (2, 1));
    }
}

//! E-mail addresses as people type and read them.

use serde::{Deserialize, Serialize};

/// A mailbox: an optional display name and an address.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Address {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    pub email: String,
}

impl Address {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Address {
        Address { name: name.into().trim().to_string(), email: email.into().trim().to_string() }
    }

    /// From `Name <a@b>`, `"Last, First" <a@b>`, `<a@b>` or `a@b`. `None` for an empty string.
    pub fn parse(s: &str) -> Option<Address> {
        let s = s.trim();
        if s.is_empty() {
            return None;
        }
        if let (Some(open), Some(close)) = (s.rfind('<'), s.rfind('>'))
            && open < close
        {
            let email = s.get(open + 1..close)?.trim();
            let name = s.get(..open)?.trim().trim_matches('"').replace("\\\"", "\"");
            return Some(Address::new(name.trim(), email));
        }
        Some(Address::new("", s.trim_matches('"')))
    }

    /// Split a recipient line on `,` and `;` (outside quotes and angle brackets) and parse each part.
    pub fn parse_list(s: &str) -> Vec<Address> {
        let mut out = Vec::new();
        let mut part = String::new();
        let (mut quoted, mut angle) = (false, false);
        let mut prev = '\0';
        for c in s.chars() {
            match c {
                '"' if prev != '\\' => quoted = !quoted,
                '<' if !quoted => angle = true,
                '>' if !quoted => angle = false,
                ',' | ';' if !quoted && !angle => {
                    out.extend(Address::parse(&part));
                    part.clear();
                    prev = c;
                    continue;
                }
                _ => {}
            }
            part.push(c);
            prev = c;
        }
        out.extend(Address::parse(&part));
        out
    }

    /// What a list shows: the name, or the address when there is none.
    pub fn display(&self) -> &str {
        if self.name.is_empty() { &self.email } else { &self.name }
    }

    /// `Name <a@b>` (the name quoted when it needs it), or just the address.
    pub fn full(&self) -> String {
        if self.name.is_empty() {
            return self.email.clone();
        }
        let plain = self.name.chars().all(|c| c.is_alphanumeric() || " .'-_".contains(c));
        if plain { format!("{} <{}>", self.name, self.email) } else { format!("\"{}\" <{}>", self.name.replace('"', "\\\""), self.email) }
    }

    /// Up to two initials for an avatar.
    pub fn initials(&self) -> String {
        let source = if self.name.is_empty() { self.email.split('@').next().unwrap_or("") } else { self.name.as_str() };
        let letters: String = source
            .split(|c: char| c.is_whitespace() || c == '.' || c == '_' || c == '-')
            .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
            .take(2)
            .collect();
        letters.to_uppercase()
    }

    /// Whether the address looks deliverable: `local@domain`, no spaces.
    pub fn is_valid(&self) -> bool {
        match self.email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty()
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
                    && !self.email.contains(char::is_whitespace)
            }
            None => false,
        }
    }

    /// The same mailbox, whatever the display name or letter case.
    pub fn same_mailbox(&self, other: &Address) -> bool {
        self.email.eq_ignore_ascii_case(&other.email)
    }
}

/// A recipient line: `Ada <ada@example.org>; bob@example.org`.
pub fn format_list(list: &[Address]) -> String {
    list.iter().map(Address::full).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_usual_forms() {
        assert_eq!(Address::parse("Ada Lovelace <ada@example.org>"), Some(Address::new("Ada Lovelace", "ada@example.org")));
        assert_eq!(Address::parse("\"Lovelace, Ada\" <ada@example.org>"), Some(Address::new("Lovelace, Ada", "ada@example.org")));
        assert_eq!(Address::parse("<ada@example.org>"), Some(Address::new("", "ada@example.org")));
        assert_eq!(Address::parse(" ada@example.org "), Some(Address::new("", "ada@example.org")));
        assert_eq!(Address::parse("  "), None);
    }

    #[test]
    fn lists_split_outside_quotes() {
        let l = Address::parse_list("\"Lovelace, Ada\" <ada@example.org>; bob@example.org, Carol <carol@example.org>;;");
        assert_eq!(l.len(), 3);
        assert_eq!(l[0].name, "Lovelace, Ada");
        assert_eq!(l[1].email, "bob@example.org");
        assert_eq!(format_list(&l), "\"Lovelace, Ada\" <ada@example.org>; bob@example.org; Carol <carol@example.org>");
        assert_eq!(Address::parse_list(&format_list(&l)), l);
        assert!(Address::parse_list("").is_empty());
    }

    #[test]
    fn display_and_validity() {
        let a = Address::new("Ada Lovelace", "ada@example.org");
        assert_eq!((a.display(), a.initials().as_str()), ("Ada Lovelace", "AL"));
        assert_eq!(Address::new("", "grace.hopper@example.org").initials(), "GH");
        assert!(a.is_valid());
        for bad in ["", "ada", "ada@", "@example.org", "ada@example", "a da@example.org", "ada@.org"] {
            assert!(!Address::new("", bad).is_valid(), "{bad}");
        }
        assert!(a.same_mailbox(&Address::new("A.", "ADA@example.org")));
    }
}

//! MailCraft mail protocols without sockets.
//!
//! [`smtp::Smtp`] and [`imap::Imap`] speak SMTP (RFC 5321, 3207, 4954) and IMAP4rev1 (RFC 3501,
//! 4315, 4959, 6154, 6851) over any `Read + Write` stream: a TLS socket on the desktop (`mailcraft-net`), a scripted
//! transcript in the tests. Everything a server sends is treated as hostile: lines, literals and
//! response counts are capped, and nothing here panics on junk.
//!
//! Passwords and access tokens never reach a log or an error text, even when a server echoes
//! them back.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use base64::Engine as _;

pub mod imap;
mod line;
pub mod smtp;

#[cfg(test)]
mod script;

/// Longest line a server may send.
pub const MAX_LINE: usize = 64 * 1024;
/// Largest counted block (an IMAP literal, so: the largest message) a server may send.
pub const MAX_LITERAL: usize = 64 * 1024 * 1024;
/// Longest user name and access token, together, that one sign-in may carry.
const MAX_XOAUTH2: usize = 32 * 1024;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ProtoError {
    #[error("connection problem: {0}")]
    Io(String),
    #[error("unexpected server response: {0}")]
    Protocol(String),
    /// The server refused the command (IMAP NO/BAD, SMTP 4xx/5xx); the text is the server's.
    #[error("the server said: {0}")]
    Rejected(String),
    #[error("sign-in failed: {0}")]
    Auth(String),
}

/// What SASL XOAUTH2 sends, in base64: `user=<user>^Aauth=Bearer <token>^A^A`, where `^A` is the
/// byte 0x01. A name or a token that would break out of that, or of the line it goes on, is refused.
pub(crate) fn xoauth2(user: &str, token: &str) -> Result<String, ProtoError> {
    if token.is_empty() {
        return Err(ProtoError::Auth("there is no access token to sign in with".into()));
    }
    let unsendable = |text: &str| text.bytes().any(|b| matches!(b, 0 | 1 | b'\r' | b'\n'));
    if unsendable(user) || unsendable(token) {
        return Err(ProtoError::Auth("the user name or access token has a character that can't be sent".into()));
    }
    if user.len().saturating_add(token.len()) > MAX_XOAUTH2 {
        return Err(ProtoError::Auth("the user name or access token is too long to be sent".into()));
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(format!("user={user}\x01auth=Bearer {token}\x01\x01")))
}

/// What must never be shown: the password or access token and every form it was sent in.
#[derive(Clone, Default)]
pub(crate) struct Secrets(Vec<String>);

impl Secrets {
    pub(crate) fn add(&mut self, secret: &str) {
        if !secret.is_empty() && !self.0.iter().any(|s| s == secret) {
            self.0.push(secret.to_string());
            // Longest first, so a secret inside another one can't leave a piece behind.
            self.0.sort_by_key(|s| std::cmp::Reverse(s.len()));
        }
    }

    /// `text` with every secret replaced by `***`.
    pub(crate) fn hide(&self, text: &str) -> String {
        let mut out = text.to_string();
        for s in &self.0 {
            if out.contains(s.as_str()) {
                out = out.replace(s.as_str(), "***");
            }
        }
        out
    }
}

/// Server text made fit for an error message: one line, no control characters, not too long.
pub(crate) fn tidy(text: &str) -> String {
    const MAX_CHARS: usize = 300;
    let mut out = String::new();
    let mut space = false;
    for (n, c) in text.trim().chars().enumerate() {
        if n >= MAX_CHARS {
            out.push('…');
            break;
        }
        if c.is_control() || c.is_whitespace() {
            space = !out.is_empty();
        } else {
            if space {
                out.push(' ');
                space = false;
            }
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_are_hidden_longest_first() {
        let mut s = Secrets::default();
        s.add("hunter2");
        s.add("hunter2-and-more");
        s.add("");
        assert_eq!(s.hide("no hunter2-and-more, no hunter2, nothing"), "no ***, no ***, nothing");
        assert_eq!(Secrets::default().hide("plain"), "plain");
    }

    #[test]
    fn the_xoauth2_response_is_user_and_bearer_token_between_control_a() {
        let decoded = |payload: String| base64::engine::general_purpose::STANDARD.decode(payload).unwrap();
        assert_eq!(decoded(xoauth2("ada@example.org", "tok.en-123").unwrap()), b"user=ada@example.org\x01auth=Bearer tok.en-123\x01\x01");
        // The example of the mechanism's description.
        assert_eq!(
            xoauth2("someuser@example.com", "ya29.vF9dft4qmTc2Nvb3RlckBhdHRhdmlzdGEuY29tCg").unwrap(),
            "dXNlcj1zb21ldXNlckBleGFtcGxlLmNvbQFhdXRoPUJlYXJlciB5YTI5LnZGOWRmdDRxbVRjMk52YjNSbGNrQmhkSFJoZG1semRHRXVZMjl0Q2cBAQ=="
        );
        assert_eq!(decoded(xoauth2("ädä", "t").unwrap()), "user=ädä\x01auth=Bearer t\x01\x01".as_bytes());
        // Nothing that could end the name, the token or the line early.
        for (user, token) in [("ada\x01auth=Bearer other", "t"), ("ada", "t\x01"), ("ada\r\n", "t"), ("ada", "t\nQUIT"), ("a\0", "t"), ("ada", "\0")]
        {
            assert!(matches!(xoauth2(user, token), Err(ProtoError::Auth(_))), "{user:?} {token:?}");
        }
        assert!(matches!(xoauth2("ada", ""), Err(ProtoError::Auth(_))));
        assert!(xoauth2("ada", &"t".repeat(MAX_XOAUTH2 - 3)).is_ok());
        let err = xoauth2("ada", &"t".repeat(MAX_XOAUTH2)).unwrap_err();
        assert!(matches!(&err, ProtoError::Auth(why) if why.len() < 100), "{err:?}");
    }

    #[test]
    fn server_text_is_tidied() {
        assert_eq!(tidy("  5.7.8\tBad\r\n credentials \u{7}"), "5.7.8 Bad credentials");
        let long = tidy(&"é".repeat(1000));
        assert_eq!(long.chars().count(), 301);
        assert!(long.ends_with('…'));
        assert_eq!(tidy(""), "");
    }
}

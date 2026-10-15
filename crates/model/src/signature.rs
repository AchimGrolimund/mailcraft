//! Signatures (R4.5): rich text the owner ends their messages with, and the pictures (a logo) it
//! may hold.
//!
//! A signature is kept in the mailbox on this computer only; nothing keeps it in step with
//! Outlook, which offers no way to. Its pictures are kept with it, checked by their own bytes
//! (PNG, JPEG or GIF) and capped by size ([`MAX_PICTURE_BYTES`] each, [`MAX_PICTURES_BYTES`]
//! together, [`MAX_PICTURES`] in all), so every message it goes into stays light. A picture
//! stands in the text as a picture run ([`crate::rich::Run::picture`]); the bytes, the width it is
//! shown at and its description are here.
//!
//! Signatures written by earlier versions (`{name, text}`) open as rich text without formatting,
//! and every signature is written with its plain text as `text` too, so an earlier version still
//! reads its words.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::ModelError;
use crate::rich::{Paragraph, RichText, Run, Style};

/// Most bytes one picture may have.
pub const MAX_PICTURE_BYTES: usize = 256 * 1024;
/// Most bytes a signature's pictures may have together.
pub const MAX_PICTURES_BYTES: usize = 512 * 1024;
/// Most pictures one signature holds.
pub const MAX_PICTURES: usize = 10;
/// Pictures above this many bytes together make every message noticeably heavier: the editor
/// says so, and lets the owner go on.
pub const HEAVY_BYTES: usize = 100 * 1024;
/// The widest a picture is shown, in pixels.
pub const MAX_WIDTH: u16 = 600;
/// Longest description of a picture, in characters.
pub const MAX_ALT: usize = 200;
/// Most characters of a signature's text.
pub const MAX_TEXT: usize = 10_000;

/// How a signature lays out its first picture against its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Layout {
    /// Pictures stand in the text where they are.
    #[default]
    Inline,
    /// The first picture on the left, the text on its right (a two-cell table when sent).
    PictureLeft,
    /// The first picture above the text.
    PictureAbove,
}

impl Layout {
    pub fn parse(code: &str) -> Option<Layout> {
        match code.trim().to_ascii_lowercase().as_str() {
            "inline" => Some(Layout::Inline),
            "pictureleft" | "left" => Some(Layout::PictureLeft),
            "pictureabove" | "above" => Some(Layout::PictureAbove),
            _ => None,
        }
    }
}

/// A picture a signature holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Picture {
    /// Its id in the signature's text ([`Run::picture`]).
    pub id: u32,
    /// The file it came from, for the owner (`logo.png`).
    pub name: String,
    /// `image/png`, `image/jpeg` or `image/gif`, from its own bytes.
    pub mime: String,
    #[serde(with = "crate::b64")]
    pub data: Arc<Vec<u8>>,
    /// The width it is shown at, in pixels, at most [`MAX_WIDTH`]; `None` for its own (capped).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u16>,
    /// What it shows, in words: for screen readers, for recipients who don't load pictures, and
    /// in plain text.
    pub alt: String,
}

/// The kind of picture `bytes` are, by their own first bytes: PNG, JPEG or GIF only.
pub fn picture_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else {
        None
    }
}

/// The details a signature can be made from (R4.5.4): kept so that changing one lays it out
/// again, until the owner edits the signature by hand.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Details {
    pub name: String,
    pub title: String,
    pub organisation: String,
    pub phone: String,
    pub mobile: String,
    pub email: String,
    pub website: String,
    pub address: String,
    /// A hash of the text the details last made; the signature's text no longer having it means
    /// the owner edited it by hand.
    #[serde(skip_serializing_if = "is_zero")]
    pub made: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

/// Text, and at most a few pictures, the owner ends their messages with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Signature {
    pub name: String,
    body: RichText,
    pictures: Vec<Picture>,
    pub layout: Layout,
    pub details: Option<Details>,
}

/// A signature as a file holds it (any version), before it is checked.
#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Raw {
    name: String,
    /// The plain text of a signature from before R4.5 (or written beside `body` for them).
    text: String,
    #[serde(deserialize_with = "crate::sync::lenient")]
    body: Option<RichText>,
    #[serde(deserialize_with = "crate::sync::lenient_list")]
    pictures: Vec<Picture>,
    #[serde(deserialize_with = "crate::sync::lenient_or_default")]
    layout: Layout,
    #[serde(deserialize_with = "crate::sync::lenient")]
    details: Option<Details>,
}

/// A signature as it is written: its plain text as `text` too, for earlier versions.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Out<'a> {
    name: &'a str,
    text: String,
    body: &'a RichText,
    #[serde(skip_serializing_if = "<[Picture]>::is_empty")]
    pictures: &'a [Picture],
    #[serde(skip_serializing_if = "is_inline")]
    layout: Layout,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: &'a Option<Details>,
}

fn is_inline(l: &Layout) -> bool {
    *l == Layout::Inline
}

impl Serialize for Signature {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        Out { name: &self.name, text: self.plain(), body: &self.body, pictures: &self.pictures, layout: self.layout, details: &self.details }
            .serialize(s)
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Signature, D::Error> {
        let raw = Raw::deserialize(d)?;
        let body = raw.body.unwrap_or_else(|| RichText::from_plain(&raw.text));
        let mut sig = Signature { name: raw.name, body, pictures: Vec::new(), layout: raw.layout, details: raw.details };
        // Each picture is checked as if it were added now; one that isn't a picture, or is over
        // the caps, is left out, and so is its place in the text.
        let mut seen = std::collections::HashSet::new();
        for p in raw.pictures {
            if seen.insert(p.id) && sig.check(&p.data, None).is_ok() {
                let mime = picture_mime(&p.data).unwrap_or_default().to_string();
                sig.pictures.push(Picture { mime, width: p.width.map(|w| w.clamp(1, MAX_WIDTH)), alt: clean_alt(&p.alt), ..p });
            }
        }
        sig.tidy();
        Ok(sig)
    }
}

/// A description as kept: one line, no control characters, at most [`MAX_ALT`] characters.
fn clean_alt(alt: &str) -> String {
    alt.chars().filter(|c| !c.is_control()).take(MAX_ALT).collect::<String>().trim().to_string()
}

impl Signature {
    /// A signature of plain text (as earlier versions kept them).
    pub fn plain_text(name: impl Into<String>, text: &str) -> Signature {
        let text: String = text.chars().take(MAX_TEXT).collect();
        Signature { name: name.into(), body: RichText::from_plain(&text), ..Signature::default() }
    }

    pub fn body(&self) -> &RichText {
        &self.body
    }

    pub fn pictures(&self) -> &[Picture] {
        &self.pictures
    }

    pub fn picture(&self, id: u32) -> Option<&Picture> {
        self.pictures.iter().find(|p| p.id == id)
    }

    /// Replace the text. Pictures it no longer holds are let go; a picture run naming a picture
    /// this signature doesn't have is left out.
    pub fn set_body(&mut self, body: RichText) -> Result<(), ModelError> {
        if body.len() > MAX_TEXT {
            return Err(ModelError::Invalid(format!("a signature holds at most {MAX_TEXT} characters")));
        }
        self.body = body;
        self.tidy();
        Ok(())
    }

    /// The words alone, as a plain-text message carries them: a picture as its description in
    /// brackets, or nothing.
    pub fn plain(&self) -> String {
        self.body.to_plain_with(|id| self.picture(id).map(|p| p.alt.clone()))
    }

    /// Whether there is nothing in it.
    pub fn is_empty(&self) -> bool {
        self.body.is_empty() && self.pictures.is_empty()
    }

    /// Bytes of its pictures together.
    pub fn picture_bytes(&self) -> usize {
        self.pictures.iter().map(|p| p.data.len()).sum()
    }

    /// Whether `data` may be added (or kept, `replacing` one of its own): a picture of a kind
    /// that is kept, within every cap. Why not, in words, when it may not.
    fn check(&self, data: &[u8], replacing: Option<u32>) -> Result<&'static str, ModelError> {
        let mime = picture_mime(data)
            .ok_or_else(|| ModelError::Invalid("a signature's pictures are PNG, JPEG or GIF (many mail programs can't show other kinds)".into()))?;
        if data.len() > MAX_PICTURE_BYTES {
            return Err(ModelError::Invalid(format!("a picture in a signature is at most {} KB", MAX_PICTURE_BYTES / 1024)));
        }
        let others: Vec<&Picture> = self.pictures.iter().filter(|p| Some(p.id) != replacing).collect();
        if others.len() >= MAX_PICTURES {
            return Err(ModelError::Invalid(format!("a signature holds at most {MAX_PICTURES} pictures")));
        }
        let total = others.iter().map(|p| p.data.len()).sum::<usize>().saturating_add(data.len());
        if total > MAX_PICTURES_BYTES {
            return Err(ModelError::Invalid(format!("a signature's pictures are at most {} KB together", MAX_PICTURES_BYTES / 1024)));
        }
        Ok(mime)
    }

    /// Add a picture at text position `at` (the end when it is past it): its id.
    pub fn add_picture(&mut self, name: &str, data: Vec<u8>, alt: Option<&str>, at: Option<usize>) -> Result<u32, ModelError> {
        let mime = self.check(&data, None)?;
        let id = self.pictures.iter().map(|p| p.id).max().map_or(1, |n| n.saturating_add(1));
        let name: String = name.chars().filter(|c| !c.is_control()).take(255).collect();
        // By default the file's name without its extension describes it.
        let alt = alt.map_or_else(|| name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem).replace(['_', '-'], " "), str::to_string);
        let at = at.unwrap_or(usize::MAX).min(self.body.len());
        let mut text = self.body.clone();
        let fragment = RichText::from_paragraphs(vec![Paragraph::new(vec![Run::picture(id, Style::default())], Default::default())]);
        text.insert_rich(at, &fragment)?;
        self.pictures.push(Picture { id, name, mime: mime.to_string(), data: Arc::new(data), width: None, alt: clean_alt(&alt) });
        self.body = text;
        Ok(id)
    }

    /// Change the width a picture is shown at (`Some(None)`: its own) and its description.
    pub fn set_picture(&mut self, id: u32, width: Option<Option<u16>>, alt: Option<&str>) -> Result<(), ModelError> {
        let p = self.pictures.iter_mut().find(|p| p.id == id).ok_or_else(|| ModelError::Invalid(format!("no picture {id} in this signature")))?;
        if let Some(width) = width {
            p.width = width.map(|w| w.clamp(1, MAX_WIDTH));
        }
        if let Some(alt) = alt {
            p.alt = clean_alt(alt);
        }
        Ok(())
    }

    /// Take a picture out, and its place in the text.
    pub fn remove_picture(&mut self, id: u32) -> Result<(), ModelError> {
        if self.picture(id).is_none() {
            return Err(ModelError::Invalid(format!("no picture {id} in this signature")));
        }
        let paragraphs = self
            .body
            .paragraphs()
            .iter()
            .map(|p| Paragraph::new(p.runs.iter().filter(|r| r.picture != Some(id)).cloned().collect(), p.list).at_level(p.level))
            .collect();
        self.body = RichText::from_paragraphs(paragraphs);
        self.tidy();
        Ok(())
    }

    /// The text and the pictures agree: no run names a picture that isn't here, and no picture is
    /// kept that the text doesn't hold.
    fn tidy(&mut self) {
        let held: std::collections::HashSet<u32> = self.pictures.iter().map(|p| p.id).collect();
        if self.body.pictures().iter().any(|id| !held.contains(id)) {
            let paragraphs = self
                .body
                .paragraphs()
                .iter()
                .map(|p| {
                    let runs = p.runs.iter().filter(|r| r.picture.is_none_or(|id| held.contains(&id))).cloned().collect();
                    Paragraph::new(runs, p.list).at_level(p.level)
                })
                .collect();
            self.body = RichText::from_paragraphs(paragraphs);
        }
        let used: std::collections::HashSet<u32> = self.body.pictures().into_iter().collect();
        self.pictures.retain(|p| used.contains(&p.id));
    }
}

#[cfg(test)]
mod tests;

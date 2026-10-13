//! What Exchange Web Services answers, read into a small tree: elements by their local names,
//! their attributes, and the text directly inside them.
//!
//! An answer is hostile like anything a server sends, and it may be anybody's: a SOAP fault, a
//! page that isn't the service at all, megabytes of nesting. So the reader takes no DTD and
//! expands no entity (a document with a `<!DOCTYPE` is not read at all; a reference that isn't a
//! character or one of XML's own five spoils the text or the attribute it stands in, which then
//! reads as empty), and everything is capped: how deep elements nest, how many there are, how
//! long a name, an attribute and a text may be. What is over a cap is left out or reads as
//! empty; only nesting that is too deep, and more elements than any answer has, end the reading.
//!
//! Elements are told by their local names. The prefixes are the document's own choice (the
//! service writes `t:` and `m:`, `s:` for the envelope), and what is read is found by where it
//! is under the operation's own answer, never by a prefix.

use quick_xml::Reader;
use quick_xml::escape::resolve_xml_entity;
use quick_xml::events::{BytesRef, BytesStart, Event};

/// Deepest the elements may nest, and most of them in one answer.
const MAX_DEPTH: usize = 40;
pub(super) const MAX_NODES: usize = 400_000;
/// Longest name of an element or an attribute that is kept (a longer one is no name anybody
/// asks for), most attributes read of one element, and the longest value of one.
const MAX_NAME: usize = 64;
const MAX_ATTRIBUTES: usize = 12;
const MAX_ATTRIBUTE: usize = 2_048;
/// Longest text kept of one element (bytes): an id, a name, a property in base64.
pub(super) const MAX_TEXT: usize = 8_192;

/// The answer isn't a document that can be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Malformed;

/// One element.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Node {
    /// Its local name; empty for one whose name is longer than any that is asked for.
    pub(super) name: String,
    /// The prefix its name is written with (`t` of `t:Folder`); empty for none. Nothing here
    /// goes by it; whoever has to hold a document to its namespaces does (the tests' stand-in).
    pub(super) prefix: String,
    /// The namespaces it declares, by prefix (`xmlns:t="…"`; the empty prefix for `xmlns="…"`).
    pub(super) namespaces: Vec<(String, String)>,
    /// Its other attributes, by their local names.
    pub(super) attributes: Vec<(String, String)>,
    /// The text directly inside it; empty when there is none, or it is longer than is kept, or
    /// it holds a reference to who knows what.
    pub(super) text: String,
    pub(super) children: Vec<Node>,
}

impl Node {
    /// The first element called `name` directly inside this one.
    pub(super) fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|child| child.name == name)
    }

    /// The element at the end of `path`, each name directly inside the one before it.
    pub(super) fn find(&self, path: &[&str]) -> Option<&Node> {
        path.iter().try_fold(self, |at, name| at.child(name))
    }

    /// Every element called `name` directly inside this one.
    pub(super) fn all<'n>(&'n self, name: &'n str) -> impl Iterator<Item = &'n Node> {
        self.children.iter().filter(move |child| child.name == name)
    }

    /// What its attribute `name` says.
    pub(super) fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(have, _)| have == name).map(|(_, value)| value.as_str())
    }

    /// The text of the element called `name` directly inside this one; empty when it has none.
    pub(super) fn text_of(&self, name: &str) -> &str {
        self.child(name).map_or("", |child| child.text.trim())
    }
}

/// An element that is being read: the node so far, and whether its text is still good.
struct Open {
    node: Node,
    spoiled: bool,
}

fn name_of(local: &[u8]) -> String {
    match std::str::from_utf8(local) {
        Ok(name) if name.len() <= MAX_NAME => name.to_string(),
        _ => String::new(),
    }
}

/// The element a start tag opens, with the attributes it has.
fn opened(start: &BytesStart<'_>) -> Result<Open, Malformed> {
    let prefix = start.name().prefix().map(|prefix| name_of(prefix.as_ref())).unwrap_or_default();
    let mut node = Node { name: name_of(start.local_name().as_ref()), prefix, ..Default::default() };
    for attribute in start.attributes().take(MAX_ATTRIBUTES) {
        // A tag whose attributes can't be told apart is no tag.
        let attribute = attribute.map_err(|_| Malformed)?;
        let name = name_of(attribute.key.local_name().as_ref());
        // (A value with a reference to who knows what in it is no value: left out.)
        let Ok(value) = attribute.normalized_value(quick_xml::XmlVersion::Implicit1_0) else { continue };
        if value.len() > MAX_ATTRIBUTE {
            continue;
        }
        // `xmlns="…"` and `xmlns:t="…"` say what a prefix stands for; anything else is the
        // element's own.
        let declares = match attribute.key.as_ref() {
            b"xmlns" => Some(String::new()),
            _ if attribute.key.prefix().is_some_and(|prefix| prefix.as_ref() == b"xmlns") => Some(name.clone()),
            _ => None,
        };
        match declares {
            Some(prefix) => node.namespaces.push((prefix, value.into_owned())),
            None if !name.is_empty() && !node.attributes.iter().any(|(have, _)| *have == name) => node.attributes.push((name, value.into_owned())),
            None => {}
        }
    }
    Ok(Open { node, spoiled: false })
}

struct Walk {
    stack: Vec<Open>,
    root: Option<Node>,
    nodes: usize,
}

impl Walk {
    fn open(&mut self, start: &BytesStart<'_>) -> Result<(), Malformed> {
        self.nodes = self.nodes.saturating_add(1);
        // (One element at the top: a second one makes it no document.)
        if self.stack.len() >= MAX_DEPTH || self.nodes > MAX_NODES || (self.stack.is_empty() && self.root.is_some()) {
            return Err(Malformed);
        }
        self.stack.push(opened(start)?);
        Ok(())
    }

    fn close(&mut self) -> Result<(), Malformed> {
        let Some(mut done) = self.stack.pop() else { return Err(Malformed) };
        if done.spoiled {
            done.node.text.clear();
        }
        match self.stack.last_mut() {
            Some(parent) => parent.node.children.push(done.node),
            None => self.root = Some(done.node),
        }
        Ok(())
    }

    fn text(&mut self, text: &str) {
        let Some(open) = self.stack.last_mut() else { return };
        if open.node.text.len().saturating_add(text.len()) > MAX_TEXT {
            open.spoiled = true;
        } else if !open.spoiled {
            open.node.text.push_str(text);
        }
    }

    /// A reference in the text: a character by its number, or one of XML's own five. Any other
    /// is not looked up anywhere, and the text it stands in is not taken.
    fn reference(&mut self, reference: &BytesRef<'_>) {
        let mut one = [0u8; 4];
        match reference.resolve_char_ref() {
            Ok(Some(c)) => self.text(c.encode_utf8(&mut one)),
            Ok(None) => match reference.decode().ok().and_then(|name| resolve_xml_entity(&name)) {
                Some(text) => self.text(text),
                None => self.spoil(),
            },
            Err(_) => self.spoil(),
        }
    }

    fn spoil(&mut self) {
        if let Some(open) = self.stack.last_mut() {
            open.spoiled = true;
        }
    }
}

/// The document in `body` (UTF-8, with or without a byte order mark) as a tree: its one element
/// at the top.
pub(super) fn parse(body: &[u8]) -> Result<Node, Malformed> {
    let body = body.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(body);
    let xml = std::str::from_utf8(body).map_err(|_| Malformed)?;
    let mut reader = Reader::from_str(xml);
    let config = reader.config_mut();
    (config.check_end_names, config.allow_unmatched_ends, config.expand_empty_elements) = (true, false, false);
    let mut walk = Walk { stack: Vec::new(), root: None, nodes: 0 };
    // Every event takes at least a byte of the document, which bounds the loop.
    for _ in 0..=xml.len() {
        match reader.read_event().map_err(|_| Malformed)? {
            Event::Start(element) => walk.open(&element)?,
            Event::Empty(element) => {
                walk.open(&element)?;
                walk.close()?;
            }
            Event::End(_) => walk.close()?,
            Event::Text(text) => walk.text(&text.decode().map_err(|_| Malformed)?),
            Event::CData(text) => walk.text(&text.decode().map_err(|_| Malformed)?),
            Event::GeneralRef(reference) => walk.reference(&reference),
            // A document that brings a DTD along is not one this reads.
            Event::DocType(_) => return Err(Malformed),
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) => {}
            Event::Eof => break,
        }
    }
    // A document that ends in the middle of an element is not the answer.
    if !walk.stack.is_empty() {
        return Err(Malformed);
    }
    walk.root.ok_or(Malformed)
}

/// `text` as it is written inside an element or an attribute of a request: the five characters
/// XML has names for by those names, and nothing that XML has no place for (control characters
/// other than the tab and the line ends, which no value written here holds anyway).
pub(super) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

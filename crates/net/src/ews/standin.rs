//! A stand-in for Exchange Web Services that behaves like the documented service, for the
//! tests. Nothing here leaves this machine, and nothing here is in a build that isn't a test's.
//!
//! It keeps the folders of one mailbox and the navigation shortcuts in its Common Views folder,
//! and holds a client to what the public EWS reference and the schemas of [MS-OXWSSRCH],
//! [MS-OXWSFOLD], [MS-OXWSCDATA] and [MS-OXWSXPROP] document:
//!
//! - **The request.** `POST /EWS/Exchange.asmx` with `Authorization: Bearer` and the token it
//!   was set up with (401 otherwise) and a `text/xml` body (415 otherwise): a SOAP 1.1 envelope
//!   whose header names a version the schema has (`RequestServerVersion`) and whose body is one
//!   operation. Every element has to be in the namespace it belongs to (the messages' or the
//!   types'), with the attributes it has and no others, and its children in the order of the
//!   schema; what isn't so is a SOAP fault (`ErrorSchemaValidation`, HTTP 500), as the service
//!   answers a request that doesn't fit its schema.
//! - **The operations.** `FindFolder` (`Traversal` of `Shallow`, `Deep` or `SoftDeleted`; a
//!   `FolderShape`, an `IndexedPageFolderView`, a `Restriction` that is one `IsEqualTo` of a
//!   `FieldURI` and a `Constant`, `ParentFolderIds`), `FindItem` (`Traversal` of `Shallow`,
//!   `SoftDeleted` or `Associated`; an `ItemShape`, an `IndexedPageItemView`, the same kind of
//!   `Restriction`, `ParentFolderIds`) and `GetFolder` (a `FolderShape` and `FolderIds`). Any
//!   other operation of the service that would change something is noted ([`State::writes`])
//!   and refused: nothing a client of this crate does may ever be one.
//! - **Ids.** `DistinguishedFolderId` takes the ids the reference lists, of which the mailbox
//!   has `root`, `msgfolderroot`, `inbox`, `drafts`, `sentitems`, `deleteditems`, `junkemail`
//!   and `outbox` (there is no `archive`: 500, as for any value the schema hasn't).
//! - **Properties.** A `FieldURI` it knows; an `ExtendedFieldURI` with a `PropertyTag` (`0x`
//!   and one to four hex digits, or a decimal number) and a `PropertyType` of the schema's.
//!   A property comes back only when the item has it under the type it was asked for: a GUID
//!   that the mailbox keeps as binary is not there for whoever asks for a `CLSID`, and the
//!   other way round ([`State::guids`]). The tag is written back the way the service writes
//!   it, in lower case and without leading zeros (`0xfff`).
//! - **Answers.** One response message for each thing asked about, in order; a `RootFolder`
//!   with `IndexedPagingOffset`, `TotalItemsInView` and `IncludesLastItemInRange`; a folder that
//!   isn't there as `ErrorFolderNotFound`; untyped items as `Message`.
//! - **Refusals.** An organisation that doesn't allow the service answers 403 with an
//!   `X-EWS-Policy-Reason` header ([`State::blocked`]); a busy one a fault with
//!   `ErrorServerBusy` ([`State::busy`]).
//!
//! And it can misbehave on purpose ([`Hostile`]), for a client that has to take anything.
//!
//! [`route`] has the calls of this crate that go to Exchange Web Services go to a stand-in
//! instead, on the thread that asks for it and until the guard is dropped.

use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex, MutexGuard};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

use super::xml::{self, Node};

/// The token a stand-in takes unless it is told another.
pub const TOKEN: &str = "standin.ews-acc_ess.tok-9876543210";
/// The two reasons Microsoft documents for the `X-EWS-Policy-Reason` header.
pub const DISABLED: &str = "EWS is disabled for this user or tenant";
pub const BLOCKED_BY_POLICY: &str = "EWS is blocked by policy for this user or tenant";

const SOAP: &str = "http://schemas.xmlsoap.org/soap/envelope/";
const TYPES: &str = "http://schemas.microsoft.com/exchange/services/2006/types";
const MESSAGES: &str = "http://schemas.microsoft.com/exchange/services/2006/messages";
const ERRORS: &str = "http://schemas.microsoft.com/exchange/services/2006/errors";
/// The versions `RequestServerVersion` may name ([MS-OXWSCDATA], `ExchangeVersionType`).
const VERSIONS: &[&str] = &[
    "Exchange2007",
    "Exchange2007_SP1",
    "Exchange2010",
    "Exchange2010_SP1",
    "Exchange2010_SP2",
    "Exchange2013",
    "Exchange2013_SP1",
    "Exchange2015",
    "Exchange2016",
];
/// The ids a `DistinguishedFolderId` may have (the reference's list).
const DISTINGUISHED: &[&str] = &[
    "calendar",
    "contacts",
    "deleteditems",
    "drafts",
    "inbox",
    "journal",
    "notes",
    "outbox",
    "sentitems",
    "tasks",
    "msgfolderroot",
    "publicfoldersroot",
    "root",
    "junkemail",
    "searchfolders",
    "voicemail",
    "recoverableitemsroot",
    "recoverableitemsdeletions",
    "recoverableitemsversions",
    "recoverableitemspurges",
    "archiveroot",
    "archivemsgfolderroot",
    "archivedeleteditems",
    "archiveinbox",
    "archiverecoverableitemsroot",
    "archiverecoverableitemsdeletions",
    "archiverecoverableitemsversions",
    "archiverecoverableitemspurges",
    "syncissues",
    "conflicts",
    "localfailures",
    "serverfailures",
    "recipientcache",
    "quickcontacts",
    "conversationhistory",
    "todosearch",
    "mycontacts",
    "directory",
    "imcontactlist",
    "peopleconnect",
    "favorites",
];
const FOLDER_URIS: &[&str] = &[
    "folder:FolderId",
    "folder:ParentFolderId",
    "folder:DisplayName",
    "folder:UnreadCount",
    "folder:TotalCount",
    "folder:ChildFolderCount",
    "folder:FolderClass",
    "folder:SearchParameters",
    "folder:ManagedFolderInformation",
    "folder:PermissionSet",
    "folder:EffectiveRights",
    "folder:SharingEffectiveRights",
];
const ITEM_URIS: &[&str] = &["item:ItemId", "item:ParentFolderId", "item:ItemClass", "item:Subject", "item:IsAssociated", "item:DateTimeCreated"];
const PROPERTY_TYPES: &[&str] = &[
    "ApplicationTime",
    "ApplicationTimeArray",
    "Binary",
    "BinaryArray",
    "Boolean",
    "CLSID",
    "CLSIDArray",
    "Currency",
    "CurrencyArray",
    "Double",
    "DoubleArray",
    "Error",
    "Float",
    "FloatArray",
    "Integer",
    "IntegerArray",
    "Long",
    "LongArray",
    "Null",
    "Object",
    "ObjectArray",
    "Short",
    "ShortArray",
    "SystemTime",
    "SystemTimeArray",
    "String",
    "StringArray",
];
/// Operations of the service that change a mailbox: none of them is ever to be asked for.
const WRITES: &[&str] = &[
    "CreateItem",
    "UpdateItem",
    "DeleteItem",
    "MoveItem",
    "CopyItem",
    "SendItem",
    "CreateFolder",
    "UpdateFolder",
    "DeleteFolder",
    "MoveFolder",
    "CopyFolder",
    "EmptyFolder",
    "CreateAttachment",
    "DeleteAttachment",
    "MarkAllItemsAsRead",
    "CreateUserConfiguration",
    "UpdateUserConfiguration",
    "DeleteUserConfiguration",
];
/// Largest request it reads, and the most entries one page holds whatever is asked for.
const MAX_REQUEST: usize = 1024 * 1024;
const MAX_PAGE: usize = 1_000;

thread_local! {
    static ROUTE: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Where this thread's calls to Exchange Web Services go instead, when a test has said so.
pub(crate) fn routed() -> Option<String> {
    ROUTE.with(|route| route.borrow().clone())
}

/// A stand-in is being spoken to in the service's place on this thread, for as long as this lives.
pub struct Routed(Option<String>);

/// Have this thread's calls to Exchange Web Services go to `base` (`http://127.0.0.1:{port}`, a
/// stand-in on this machine: nothing else is spoken to unencrypted) until the guard is dropped.
pub fn route(base: &str) -> Routed {
    Routed(ROUTE.with(|route| route.borrow_mut().replace(base.to_string())))
}

impl Drop for Routed {
    fn drop(&mut self) {
        let before = self.0.take();
        ROUTE.with(|route| *route.borrow_mut() = before);
    }
}

/// A folder of the mailbox.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Folder {
    /// What the service calls it.
    pub id: String,
    pub name: String,
    /// The id of the folder it lies in; empty for the root of the mailbox.
    pub parent: String,
    /// Its EntryID (`PidTagEntryId`, 0x0FFF).
    pub entry_id: Vec<u8>,
    /// The distinguished id it has (`inbox`); empty for none.
    pub distinguished: String,
}

/// A navigation shortcut in the Common Views folder: the properties it has.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shortcut {
    /// `PidTagWlinkType`: 0 a folder, 1 a search folder, 2 another user's folder, 4 a heading.
    pub kind: Option<i32>,
    /// `PidTagWlinkSection`: 1 the Favorites of Mail.
    pub section: Option<i32>,
    /// `PidTagWlinkOrdinal`.
    pub ordinal: Option<Vec<u8>>,
    /// `PidTagWlinkEntryId`.
    pub entry_id: Option<Vec<u8>>,
    /// `PidTagWlinkGroupClsid`.
    pub group: Option<[u8; 16]>,
    /// `PidTagWlinkGroupHeaderID`.
    pub header: Option<[u8; 16]>,
    /// Its message class; a shortcut's unless a test says otherwise.
    pub class: String,
}

/// How the mailbox keeps the GUIDs of a shortcut (its group, a heading's id): the two types
/// the documents give them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Guids {
    /// As binary (the property pages, and [MS-OXPROPS]).
    #[default]
    Binary,
    /// As GUIDs (the examples of [MS-OXOCFG]).
    Clsid,
}

/// A way of answering that no service should have, for a client that has to take anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hostile {
    /// A document that brings a DTD and entities of its own.
    Dtd,
    /// An answer that stops in the middle.
    CutShort,
    /// Something that isn't XML at all, with a status that says all is well.
    NotXml,
    /// Elements nested without end.
    Deep,
    /// More than any client reads.
    Huge,
    /// A SOAP fault that names no reason.
    Fault,
    /// Pages that never say they are the last, each with the same entries again.
    Endless,
    /// Fewer answers than there were questions.
    TooFew,
}

/// A request that came in, and what it was answered with.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Asked {
    /// The operation (`FindItem`); empty when the request named none.
    pub operation: String,
    /// Its `Traversal`, for the two that have one.
    pub traversal: String,
    /// The status of the answer.
    pub status: u16,
    /// The response code of the first answer in it (`NoError`), or of the fault.
    pub code: String,
    /// The whole body, as it came.
    pub body: String,
    headers: Vec<(String, String)>,
}

impl Asked {
    /// What the header `name` (in lower case) said; empty when there was none.
    pub fn header(&self, name: &str) -> &str {
        self.headers.iter().find(|(have, _)| have == name).map_or("", |(_, value)| value.as_str())
    }
}

/// What a stand-in keeps and how it behaves.
#[derive(Default)]
pub struct State {
    /// The token it takes.
    pub token: String,
    pub folders: Vec<Folder>,
    pub shortcuts: Vec<Shortcut>,
    /// The organisation doesn't allow the service: every request is answered 403, with this as
    /// the `X-EWS-Policy-Reason` ([`DISABLED`], [`BLOCKED_BY_POLICY`]).
    pub blocked: Option<String>,
    /// Every request is answered 403 without saying whose no it is.
    pub forbidden: bool,
    /// The next so many requests are told that the server is busy.
    pub busy: u32,
    /// The most entries a page holds, whatever a request asks for (0: as many as it asks for).
    pub page: usize,
    /// How the GUIDs of a shortcut are kept.
    pub guids: Guids,
    /// A request that asks for a shortcut's group is refused as one for a property that can't
    /// be asked for (`ErrorInvalidPropertyRequest`).
    pub refuses_groups: bool,
    /// Every answer from now on is this instead of what it should be.
    pub hostile: Option<Hostile>,
    /// Every request so far, in order.
    pub requests: Vec<Asked>,
    next: u64,
}

/// A refusal: the HTTP status, the response code, and whether it is a fault of the whole request.
struct Refusal {
    status: u16,
    code: &'static str,
    why: String,
}

fn invalid(why: impl Into<String>) -> Refusal {
    Refusal { status: 500, code: "ErrorSchemaValidation", why: why.into() }
}

/// The mailbox's own provider UID and database GUID, and another mailbox's.
const PROVIDER: [u8; 16] = [0x5E, 0x3A, 0x11, 0x9C, 0x42, 0x7D, 0x4B, 0x10, 0x8E, 0x55, 0x01, 0xA2, 0xB3, 0xC4, 0xD5, 0xE6];
const DATABASE: [u8; 16] = [0x7A, 0x21, 0xC3, 0x04, 0x9B, 0x6E, 0x4F, 0x18, 0xA5, 0x37, 0xD0, 0x02, 0x14, 0x86, 0xF9, 0x3B];
const ELSEWHERE: [u8; 16] = [0x0C, 0xF1, 0x88, 0x3D, 0x27, 0x5A, 0x41, 0x99, 0xB6, 0x60, 0x7E, 0x15, 0xA8, 0x4C, 0x2B, 0xD7];

/// The EntryID of a folder ([MS-OXCDATA] 2.2.4.1): flags, the provider's UID, the folder's
/// type, the database's GUID, a counter and two bytes of padding.
fn entry_id(provider: &[u8; 16], database: &[u8; 16], counter: u64) -> Vec<u8> {
    let mut out = vec![0u8; 4];
    out.extend_from_slice(provider);
    out.extend_from_slice(&[0x01, 0x00]);
    out.extend_from_slice(database);
    out.extend_from_slice(counter.to_be_bytes().get(2..).unwrap_or(&[0; 6]));
    out.extend_from_slice(&[0, 0]);
    out
}

/// A GUID's sixteen bytes written out, as the service writes a `CLSID`.
fn guid_text(bytes: &[u8; 16]) -> String {
    let [a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, e0, e1, e2, e3, e4, e5] = *bytes;
    format!("{a3:02x}{a2:02x}{a1:02x}{a0:02x}-{b1:02x}{b0:02x}-{c1:02x}{c0:02x}-{d0:02x}{d1:02x}-{e0:02x}{e1:02x}{e2:02x}{e3:02x}{e4:02x}{e5:02x}")
}

fn escape(text: &str) -> String {
    xml::escape(text)
}

/// A property that was asked for: its tag, and the type it was asked under.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Wanted {
    tag: u16,
    kind: String,
}

/// What a shape asks for besides the id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Shape {
    fields: Vec<String>,
    properties: Vec<Wanted>,
}

impl State {
    fn new_id(&mut self) -> String {
        self.next += 1;
        // In the letters the service's ids have; nothing of it means anything.
        format!("AAMkAGV4YW1wbGUtb3JnL2V3cy1zdGFuZC1pbgAuAAAAAACq{:06}+Tk/Zm9sZGVyLWlkAAA=", self.next)
    }

    /// Add a folder called `name` under the folder `parent` (the top of the mailbox when none
    /// is given); its id.
    pub fn add_folder(&mut self, name: &str, parent: Option<&str>) -> String {
        let parent = parent.map(str::to_string).unwrap_or_else(|| self.distinguished("msgfolderroot"));
        self.add(name, &parent, "")
    }

    fn add(&mut self, name: &str, parent: &str, distinguished: &str) -> String {
        let id = self.new_id();
        let entry_id = entry_id(&PROVIDER, &DATABASE, self.next);
        self.folders.push(Folder { id: id.clone(), name: name.into(), parent: parent.into(), entry_id, distinguished: distinguished.into() });
        id
    }

    /// The id of the folder with the distinguished id `name` (`inbox`); empty for none.
    pub fn distinguished(&self, name: &str) -> String {
        self.folders.iter().find(|folder| folder.distinguished == name).map(|folder| folder.id.clone()).unwrap_or_default()
    }

    /// The EntryID of the folder `id`.
    pub fn entry_id_of(&self, id: &str) -> Vec<u8> {
        self.folders.iter().find(|folder| folder.id == id).map(|folder| folder.entry_id.clone()).unwrap_or_default()
    }

    /// Put the folder `id` under the Favorites of Mail, at `ordinal`.
    pub fn add_favorite(&mut self, id: &str, ordinal: &[u8]) {
        let entry_id = self.entry_id_of(id);
        self.add_shortcut_to(entry_id, ordinal);
    }

    /// Put a shortcut to whatever has the EntryID `entry_id` under the Favorites of Mail.
    pub fn add_shortcut_to(&mut self, entry_id: Vec<u8>, ordinal: &[u8]) {
        self.shortcuts.push(Shortcut {
            kind: Some(0),
            section: Some(1),
            ordinal: Some(ordinal.to_vec()),
            entry_id: Some(entry_id),
            ..Shortcut::link()
        });
    }

    /// A shortcut under the Favorites of Mail to a folder of another mailbox.
    pub fn add_favorite_elsewhere(&mut self, ordinal: &[u8]) {
        self.next += 1;
        let entry_id = entry_id(&ELSEWHERE, &ELSEWHERE, self.next);
        self.add_shortcut_to(entry_id, ordinal);
    }

    /// A shortcut under the Favorites of Mail to a folder this mailbox no longer has.
    pub fn add_stale_favorite(&mut self, ordinal: &[u8]) {
        self.next += 1;
        let entry_id = entry_id(&PROVIDER, &DATABASE, 0xFFFF_0000 + self.next);
        self.add_shortcut_to(entry_id, ordinal);
    }

    /// The operations that would have changed something, had they been taken: none, for a
    /// client that only reads.
    pub fn writes(&self) -> Vec<String> {
        self.requests.iter().filter(|asked| WRITES.contains(&asked.operation.as_str())).map(|asked| asked.operation.clone()).collect()
    }

    /// How often `operation` was asked for.
    pub fn asked(&self, operation: &str) -> usize {
        self.requests.iter().filter(|asked| asked.operation == operation).count()
    }

    fn folder(&self, id: &str) -> Option<&Folder> {
        self.folders.iter().find(|folder| folder.id == id)
    }

    /// The folders under `parent`: the ones directly in it, or (`deep`) all of them, parents
    /// before what lies in them. A ring of folders is walked once.
    fn under(&self, parent: &str, deep: bool) -> Vec<&Folder> {
        let mut out: Vec<&Folder> = Vec::new();
        let mut level: Vec<&str> = vec![parent];
        for _ in 0..64 {
            let mut next: Vec<&str> = Vec::new();
            for folder in self.folders.iter().filter(|folder| level.contains(&folder.parent.as_str())) {
                if !out.iter().any(|have| have.id == folder.id) {
                    out.push(folder);
                    next.push(folder.id.as_str());
                }
            }
            if !deep || next.is_empty() {
                break;
            }
            level = next;
        }
        out
    }
}

impl Shortcut {
    /// A shortcut with nothing said of it but its class.
    pub fn link() -> Shortcut {
        Shortcut { class: "IPM.Microsoft.WunderBar.Link".into(), ..Default::default() }
    }

    /// The heading of a group under the Favorites of Mail.
    pub fn heading(id: [u8; 16], ordinal: &[u8]) -> Shortcut {
        Shortcut { kind: Some(4), section: Some(1), ordinal: Some(ordinal.to_vec()), header: Some(id), ..Shortcut::link() }
    }
}

// ---- holding a request to the schema ----

/// The namespace the prefix of `node` stands for, as `root` or the element itself declares it.
fn namespace<'n>(root: &'n Node, node: &'n Node) -> &'n str {
    let declared = |at: &'n Node| at.namespaces.iter().find(|(prefix, _)| *prefix == node.prefix).map(|(_, uri)| uri.as_str());
    declared(node).or_else(|| declared(root)).unwrap_or("")
}

struct Checker<'n> {
    root: &'n Node,
}

impl<'n> Checker<'n> {
    /// `node` is the element `name` of the namespace `space`, with no attributes but `allowed`.
    fn is(&self, node: &Node, space: &str, name: &str, allowed: &[&str]) -> Result<(), Refusal> {
        if node.name != name || namespace(self.root, node) != space {
            return Err(invalid(format!("the element {name} of {space} was expected, not {}", node.name)));
        }
        match node.attributes.iter().find(|(have, _)| !allowed.contains(&have.as_str())) {
            Some((odd, _)) => Err(invalid(format!("{name} has no attribute {odd}"))),
            None => Ok(()),
        }
    }

    /// The children of `node`, which are all of the namespace `space` and come in the order of
    /// `order` (each at most once): the child of each name, or none.
    fn children<'c>(&self, node: &'c Node, space: &str, order: &[&str]) -> Result<Vec<Option<&'c Node>>, Refusal> {
        let mut found: Vec<Option<&'c Node>> = order.iter().map(|_| None).collect();
        let mut at = 0usize;
        for child in &node.children {
            let place = order.iter().position(|name| *name == child.name);
            match place {
                Some(place) if place >= at && found.get(place).is_some_and(Option::is_none) && namespace(self.root, child) == space => {
                    if let Some(slot) = found.get_mut(place) {
                        *slot = Some(child);
                    }
                    at = place;
                }
                _ => return Err(invalid(format!("{} has no place for {} there", node.name, child.name))),
            }
        }
        Ok(found)
    }

    fn required<'c>(&self, node: &Node, child: Option<&'c Node>, name: &str) -> Result<&'c Node, Refusal> {
        child.ok_or_else(|| invalid(format!("{} needs a {name}", node.name)))
    }

    /// An attribute that has to be there.
    fn attribute<'c>(&self, node: &'c Node, name: &str) -> Result<&'c str, Refusal> {
        node.attribute(name).ok_or_else(|| invalid(format!("{} needs the attribute {name}", node.name)))
    }

    /// A `FolderShape` or an `ItemShape`: a `BaseShape`, then perhaps `AdditionalProperties`.
    fn shape(&self, node: &Node, name: &str, uris: &[&str]) -> Result<Shape, Refusal> {
        self.is(node, MESSAGES, name, &[])?;
        let parts = self.children(node, TYPES, &["BaseShape", "IncludeMimeContent", "BodyType", "AdditionalProperties"])?;
        let base = self.required(node, parts.first().copied().flatten(), "BaseShape")?;
        if !["IdOnly", "Default", "AllProperties"].contains(&base.text.trim()) {
            return Err(invalid("BaseShape is IdOnly, Default or AllProperties"));
        }
        if base.text.trim() != "IdOnly" {
            return Err(Refusal { status: 500, code: "ErrorInvalidRequest", why: "this stand-in answers IdOnly shapes only".into() });
        }
        let mut shape = Shape::default();
        let Some(more) = parts.get(3).copied().flatten() else { return Ok(shape) };
        for one in &more.children {
            if namespace(self.root, one) != TYPES {
                return Err(invalid("AdditionalProperties holds paths to properties"));
            }
            match one.name.as_str() {
                "FieldURI" => {
                    self.is(one, TYPES, "FieldURI", &["FieldURI"])?;
                    let uri = self.attribute(one, "FieldURI")?;
                    if !uris.contains(&uri) {
                        return Err(invalid(format!("{uri} is no property of that")));
                    }
                    shape.fields.push(uri.to_string());
                }
                "ExtendedFieldURI" => shape.properties.push(self.extended(one)?),
                other => return Err(invalid(format!("AdditionalProperties has no place for {other}"))),
            }
        }
        Ok(shape)
    }

    /// An `ExtendedFieldURI` that names a property by its tag.
    fn extended(&self, node: &Node) -> Result<Wanted, Refusal> {
        self.is(
            node,
            TYPES,
            "ExtendedFieldURI",
            &["DistinguishedPropertySetId", "PropertySetId", "PropertyTag", "PropertyName", "PropertyId", "PropertyType"],
        )?;
        let kind = self.attribute(node, "PropertyType")?;
        if !PROPERTY_TYPES.contains(&kind) {
            return Err(invalid(format!("{kind} is no PropertyType")));
        }
        // With a tag, nothing else names the property.
        if node.attributes.len() != 2 {
            return Err(Refusal {
                status: 500,
                code: "ErrorInvalidExtendedProperty",
                why: "a PropertyTag goes with a PropertyType and nothing else".into(),
            });
        }
        let tag = self.attribute(node, "PropertyTag")?;
        let hex = tag.strip_prefix("0x").or_else(|| tag.strip_prefix("0X"));
        let number = match hex {
            Some(digits) if (1..=4).contains(&digits.len()) => u16::from_str_radix(digits, 16).ok(),
            Some(_) => None,
            None => tag.parse::<u16>().ok().filter(|_| tag.bytes().all(|b| b.is_ascii_digit())),
        };
        let tag = number.ok_or_else(|| invalid(format!("{tag} is no PropertyTag")))?;
        if (0x8000..=0xFFFE).contains(&tag) {
            return Err(Refusal { status: 500, code: "ErrorNoPropertyTagForCustomProperties", why: "a named property has no tag to ask by".into() });
        }
        Ok(Wanted { tag, kind: kind.to_string() })
    }

    /// An indexed page view: from which entry, and how many at most.
    fn view(&self, node: Option<&Node>, name: &str) -> Result<(usize, usize), Refusal> {
        let Some(node) = node else { return Ok((0, MAX_PAGE)) };
        self.is(node, MESSAGES, name, &["MaxEntriesReturned", "Offset", "BasePoint"])?;
        if !node.children.is_empty() {
            return Err(invalid(format!("{name} holds nothing")));
        }
        let offset = self.attribute(node, "Offset")?.parse::<usize>().map_err(|_| invalid("Offset is a number"))?;
        if !["Beginning", "End"].contains(&self.attribute(node, "BasePoint")?) {
            return Err(invalid("BasePoint is Beginning or End"));
        }
        if node.attribute("BasePoint") != Some("Beginning") {
            return Err(Refusal { status: 500, code: "ErrorInvalidRequest", why: "this stand-in pages from the beginning only".into() });
        }
        let most = match node.attribute("MaxEntriesReturned") {
            Some(most) => most.parse::<usize>().ok().filter(|most| *most > 0).ok_or_else(|| invalid("MaxEntriesReturned is a number above zero"))?,
            None => MAX_PAGE,
        };
        Ok((offset, most.min(MAX_PAGE)))
    }

    /// A `Restriction` that is one `IsEqualTo` of a `FieldURI` and a `Constant`: the two.
    fn restriction<'c>(&self, node: Option<&'c Node>, uris: &[&str]) -> Result<Option<(&'c str, &'c str)>, Refusal> {
        let Some(node) = node else { return Ok(None) };
        self.is(node, MESSAGES, "Restriction", &[])?;
        let [equal] = node.children.as_slice() else { return Err(invalid("a Restriction is one expression")) };
        if equal.name != "IsEqualTo" {
            return Err(Refusal { status: 500, code: "ErrorInvalidRequest", why: "this stand-in compares with IsEqualTo only".into() });
        }
        self.is(equal, TYPES, "IsEqualTo", &[])?;
        let [path, value] = equal.children.as_slice() else { return Err(invalid("IsEqualTo compares a property with a value")) };
        self.is(path, TYPES, "FieldURI", &["FieldURI"])?;
        let uri = self.attribute(path, "FieldURI")?;
        if !uris.contains(&uri) {
            return Err(invalid(format!("{uri} is no property of that")));
        }
        self.is(value, TYPES, "FieldURIOrConstant", &[])?;
        let [constant] = value.children.as_slice() else { return Err(invalid("FieldURIOrConstant holds one of the two")) };
        self.is(constant, TYPES, "Constant", &["Value"])?;
        Ok(Some((uri, self.attribute(constant, "Value")?)))
    }

    /// The folders a list of ids names: each as the id it has in the mailbox (`None` for one
    /// the mailbox doesn't have).
    fn folder_ids(&self, state: &State, node: &Node, name: &str) -> Result<Vec<Option<String>>, Refusal> {
        self.is(node, MESSAGES, name, &[])?;
        if node.children.is_empty() {
            return Err(invalid(format!("{name} names at least one folder")));
        }
        let mut out = Vec::new();
        for one in &node.children {
            match one.name.as_str() {
                "DistinguishedFolderId" => {
                    self.is(one, TYPES, "DistinguishedFolderId", &["Id", "ChangeKey"])?;
                    let id = self.attribute(one, "Id")?;
                    if !DISTINGUISHED.contains(&id) {
                        return Err(invalid(format!("{id} is no distinguished folder")));
                    }
                    out.push(Some(state.distinguished(id)).filter(|id| !id.is_empty()));
                }
                "FolderId" => {
                    self.is(one, TYPES, "FolderId", &["Id", "ChangeKey"])?;
                    let id = self.attribute(one, "Id")?;
                    out.push(state.folder(id).map(|folder| folder.id.clone()));
                }
                other => return Err(invalid(format!("{name} has no place for {other}"))),
            }
        }
        Ok(out)
    }
}

// ---- answering ----

/// An extended property as the service writes it back: the tag in lower case without leading
/// zeros, the type it was asked under, and the value.
fn property_xml(wanted: &Wanted, value: &str) -> String {
    format!(
        "<t:ExtendedProperty><t:ExtendedFieldURI PropertyTag=\"0x{:x}\" PropertyType=\"{}\"/><t:Value>{}</t:Value></t:ExtendedProperty>",
        wanted.tag,
        wanted.kind,
        escape(value)
    )
}

fn guid_value(state: &State, wanted: &Wanted, guid: Option<&[u8; 16]>) -> Option<String> {
    let guid = guid?;
    match (state.guids, wanted.kind.as_str()) {
        (Guids::Binary, "Binary") => Some(STANDARD.encode(guid)),
        (Guids::Clsid, "CLSID") => Some(guid_text(guid)),
        _ => None,
    }
}

/// A shortcut as a `Message` of an answer, with the properties that were asked for and that it
/// has under the type they were asked under.
fn shortcut_xml(state: &State, shortcut: &Shortcut, n: usize, shape: &Shape) -> String {
    let mut out = format!("<t:Message><t:ItemId Id=\"AAMkAGV4YW1wbGUtb3JnL2V3cy1saW5r{n:06}AAA=\" ChangeKey=\"CQAAAA==\"/>");
    if shape.fields.iter().any(|uri| uri == "item:ItemClass") {
        out.push_str(&format!("<t:ItemClass>{}</t:ItemClass>", escape(&shortcut.class)));
    }
    for wanted in &shape.properties {
        let number = |value: Option<i32>| value.filter(|_| wanted.kind == "Integer").map(|value| value.to_string());
        let bytes = |value: Option<&Vec<u8>>| value.filter(|_| wanted.kind == "Binary").map(|value| STANDARD.encode(value));
        let value = match wanted.tag {
            0x6849 => number(shortcut.kind),
            0x6852 => number(shortcut.section),
            0x684B => bytes(shortcut.ordinal.as_ref()),
            0x684C => bytes(shortcut.entry_id.as_ref()),
            0x6850 => guid_value(state, wanted, shortcut.group.as_ref()),
            0x6842 => guid_value(state, wanted, shortcut.header.as_ref()),
            _ => None,
        };
        if let Some(value) = value {
            out.push_str(&property_xml(wanted, &value));
        }
    }
    out.push_str("</t:Message>");
    out
}

/// A folder as a `Folder` of an answer, its parts in the order of the schema.
fn folder_xml(folder: &Folder, shape: &Shape) -> String {
    let mut out = format!("<t:Folder><t:FolderId Id=\"{}\" ChangeKey=\"AQAAAA==\"/>", escape(&folder.id));
    let asked = |uri: &str| shape.fields.iter().any(|have| have == uri);
    if asked("folder:ParentFolderId") {
        out.push_str(&format!("<t:ParentFolderId Id=\"{}\" ChangeKey=\"AQAAAA==\"/>", escape(&folder.parent)));
    }
    if asked("folder:FolderClass") {
        out.push_str("<t:FolderClass>IPF.Note</t:FolderClass>");
    }
    if asked("folder:DisplayName") {
        out.push_str(&format!("<t:DisplayName>{}</t:DisplayName>", escape(&folder.name)));
    }
    for wanted in shape.properties.iter().filter(|wanted| wanted.tag == 0x0FFF && wanted.kind == "Binary") {
        out.push_str(&property_xml(wanted, &STANDARD.encode(&folder.entry_id)));
    }
    out.push_str("</t:Folder>");
    out
}

/// A page of `entries`, and what the `RootFolder` of the answer says of it.
fn page<T>(state: &State, entries: Vec<T>, offset: usize, most: usize) -> (Vec<T>, String) {
    let total = entries.len();
    let most = if state.page == 0 { most } else { most.min(state.page) };
    let endless = state.hostile == Some(Hostile::Endless);
    // (Endless: the same first entries again, whatever was asked for, and never the last.)
    let from = if endless { 0 } else { offset.min(total) };
    let taken: Vec<T> = entries.into_iter().skip(from).take(most).collect();
    let next = from + taken.len();
    let last = next >= total && !endless;
    (taken, format!("IndexedPagingOffset=\"{next}\" TotalItemsInView=\"{total}\" IncludesLastItemInRange=\"{last}\""))
}

fn envelope(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><s:Envelope xmlns:s=\"{SOAP}\"><s:Header>\
         <h:ServerVersionInfo MajorVersion=\"15\" MinorVersion=\"20\" MajorBuildNumber=\"9000\" MinorBuildNumber=\"10\" Version=\"V2018_01_08\" xmlns:h=\"{TYPES}\"/>\
         </s:Header><s:Body>{body}</s:Body></s:Envelope>"
    )
}

/// One response message that says no.
fn error_message(operation: &str, code: &str, rest: &str) -> String {
    format!(
        "<m:{operation}ResponseMessage ResponseClass=\"Error\"><m:MessageText>The request was refused.</m:MessageText>\
         <m:ResponseCode>{code}</m:ResponseCode><m:DescriptiveLinkKey>0</m:DescriptiveLinkKey>{rest}</m:{operation}ResponseMessage>"
    )
}

fn response(operation: &str, messages: &str) -> String {
    envelope(&format!(
        "<m:{operation}Response xmlns:m=\"{MESSAGES}\" xmlns:t=\"{TYPES}\"><m:ResponseMessages>{messages}</m:ResponseMessages></m:{operation}Response>"
    ))
}

fn fault(code: &str, why: &str) -> String {
    envelope(&format!(
        "<s:Fault><faultcode xmlns:a=\"{TYPES}\">a:{code}</faultcode><faultstring xml:lang=\"en-US\">{}</faultstring>\
         <detail><e:ResponseCode xmlns:e=\"{ERRORS}\">{code}</e:ResponseCode><e:Message xmlns:e=\"{ERRORS}\">{}</e:Message></detail></s:Fault>",
        escape(why),
        escape(why)
    ))
}

impl State {
    /// The answer to a request: the status, the response code it carries, and the body.
    fn answer(&mut self, asked: &mut Asked) -> Result<(u16, &'static str, String), Refusal> {
        let root = xml::parse(asked.body.as_bytes()).map_err(|_| invalid("the request is not XML that can be read"))?;
        let check = Checker { root: &root };
        check.is(&root, SOAP, "Envelope", &[])?;
        let parts = check.children(&root, SOAP, &["Header", "Body"])?;
        let header = check.required(&root, parts.first().copied().flatten(), "Header")?;
        let version =
            header.children.iter().find(|child| child.name == "RequestServerVersion").ok_or_else(|| invalid("a RequestServerVersion is needed"))?;
        check.is(version, TYPES, "RequestServerVersion", &["Version"])?;
        if !VERSIONS.contains(&check.attribute(version, "Version")?) {
            return Err(Refusal { status: 500, code: "ErrorInvalidServerVersion", why: "no such version of the schema".into() });
        }
        let body = check.required(&root, parts.get(1).copied().flatten(), "Body")?;
        let [operation] = body.children.as_slice() else { return Err(invalid("the body holds one operation")) };
        asked.operation = operation.name.clone();
        asked.traversal = operation.attribute("Traversal").unwrap_or("").to_string();
        if namespace(&root, operation) != MESSAGES {
            return Err(Refusal { status: 500, code: "ErrorInvalidRequest", why: "the operation is not one of the service's".into() });
        }
        let said = match operation.name.as_str() {
            "FindFolder" => self.find_folder(&check, operation)?,
            "FindItem" => self.find_item(&check, operation)?,
            "GetFolder" => self.get_folder(&check, operation)?,
            // Never to be asked for: noted (by its name, above), and refused.
            name if WRITES.contains(&name) => ("ErrorAccessDenied", response(name, &error_message(name, "ErrorAccessDenied", ""))),
            _ => return Err(Refusal { status: 500, code: "ErrorInvalidRequest", why: "no such operation".into() }),
        };
        Ok((200, said.0, said.1))
    }

    fn find_folder(&mut self, check: &Checker<'_>, operation: &Node) -> Result<(&'static str, String), Refusal> {
        check.is(operation, MESSAGES, "FindFolder", &["Traversal"])?;
        let traversal = check.attribute(operation, "Traversal")?;
        if !["Shallow", "Deep", "SoftDeleted"].contains(&traversal) {
            return Err(invalid("Traversal is Shallow, Deep or SoftDeleted"));
        }
        let parts = check.children(
            operation,
            MESSAGES,
            &["FolderShape", "IndexedPageFolderView", "FractionalPageFolderView", "Restriction", "ParentFolderIds"],
        )?;
        let shape = check.shape(check.required(operation, parts.first().copied().flatten(), "FolderShape")?, "FolderShape", FOLDER_URIS)?;
        let (offset, most) = check.view(parts.get(1).copied().flatten(), "IndexedPageFolderView")?;
        let equal = check.restriction(parts.get(3).copied().flatten(), FOLDER_URIS)?;
        let parents = check.folder_ids(self, check.required(operation, parts.get(4).copied().flatten(), "ParentFolderIds")?, "ParentFolderIds")?;
        let mut messages = String::new();
        let mut first = "NoError";
        for parent in parents {
            let Some(parent) = parent else {
                first = "ErrorFolderNotFound";
                messages.push_str(&error_message("FindFolder", "ErrorFolderNotFound", ""));
                continue;
            };
            let mut found = if traversal == "SoftDeleted" { Vec::new() } else { self.under(&parent, traversal == "Deep") };
            if let Some((uri, value)) = equal {
                found.retain(|folder| match uri {
                    "folder:DisplayName" => folder.name == value,
                    "folder:FolderClass" => value == "IPF.Note",
                    _ => false,
                });
            }
            let (found, paging) = page(self, found, offset, most);
            let folders: String = found.iter().map(|folder| folder_xml(folder, &shape)).collect();
            messages.push_str(&format!(
                "<m:FindFolderResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
                 <m:RootFolder {paging}><t:Folders>{folders}</t:Folders></m:RootFolder></m:FindFolderResponseMessage>"
            ));
        }
        Ok((first, response("FindFolder", &messages)))
    }

    fn find_item(&mut self, check: &Checker<'_>, operation: &Node) -> Result<(&'static str, String), Refusal> {
        check.is(operation, MESSAGES, "FindItem", &["Traversal"])?;
        let traversal = check.attribute(operation, "Traversal")?;
        if !["Shallow", "SoftDeleted", "Associated"].contains(&traversal) {
            return Err(invalid("Traversal is Shallow, SoftDeleted or Associated"));
        }
        let order = [
            "ItemShape",
            "IndexedPageItemView",
            "FractionalPageItemView",
            "CalendarView",
            "ContactsView",
            "GroupBy",
            "DistinguishedGroupBy",
            "Restriction",
            "SortOrder",
            "ParentFolderIds",
            "QueryString",
        ];
        let parts = check.children(operation, MESSAGES, &order)?;
        let shape = check.shape(check.required(operation, parts.first().copied().flatten(), "ItemShape")?, "ItemShape", ITEM_URIS)?;
        let (offset, most) = check.view(parts.get(1).copied().flatten(), "IndexedPageItemView")?;
        if parts.iter().enumerate().any(|(n, part)| part.is_some() && [2, 3, 4, 5, 6, 8, 10].contains(&n)) {
            return Err(Refusal { status: 500, code: "ErrorInvalidRequest", why: "this stand-in answers plain pages only".into() });
        }
        let equal = check.restriction(parts.get(7).copied().flatten(), ITEM_URIS)?;
        let parents = check.folder_ids(self, check.required(operation, parts.get(9).copied().flatten(), "ParentFolderIds")?, "ParentFolderIds")?;
        let groups = shape.properties.iter().any(|wanted| matches!(wanted.tag, 0x6842 | 0x684F | 0x6850));
        let mut messages = String::new();
        let mut first = "NoError";
        for parent in parents {
            let Some(parent) = parent else {
                first = "ErrorItemNotFound";
                messages.push_str(&error_message("FindItem", "ErrorItemNotFound", ""));
                continue;
            };
            if groups && self.refuses_groups {
                first = "ErrorInvalidPropertyRequest";
                messages.push_str(&error_message("FindItem", "ErrorInvalidPropertyRequest", ""));
                continue;
            }
            // The shortcuts are hidden items of the Common Views folder, and of no other.
            let views = self.folder(&parent).is_some_and(|folder| folder.name == "Common Views" && folder.parent == self.distinguished("root"));
            let mut found: Vec<(usize, &Shortcut)> =
                if traversal == "Associated" && views { self.shortcuts.iter().enumerate().collect() } else { Vec::new() };
            if let Some((uri, value)) = equal {
                found.retain(|(_, shortcut)| uri == "item:ItemClass" && shortcut.class == value);
            }
            let (found, paging) = page(self, found, offset, most);
            let items: String = found.iter().map(|(n, shortcut)| shortcut_xml(self, shortcut, *n, &shape)).collect();
            messages.push_str(&format!(
                "<m:FindItemResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
                 <m:RootFolder {paging}><t:Items>{items}</t:Items></m:RootFolder></m:FindItemResponseMessage>"
            ));
        }
        Ok((first, response("FindItem", &messages)))
    }

    fn get_folder(&mut self, check: &Checker<'_>, operation: &Node) -> Result<(&'static str, String), Refusal> {
        check.is(operation, MESSAGES, "GetFolder", &[])?;
        let parts = check.children(operation, MESSAGES, &["FolderShape", "FolderIds"])?;
        let shape = check.shape(check.required(operation, parts.first().copied().flatten(), "FolderShape")?, "FolderShape", FOLDER_URIS)?;
        let ids = check.folder_ids(self, check.required(operation, parts.get(1).copied().flatten(), "FolderIds")?, "FolderIds")?;
        let few = self.hostile == Some(Hostile::TooFew);
        let mut messages = String::new();
        let mut first = "";
        for id in ids.into_iter().skip(usize::from(few)) {
            let code = match id.as_deref().and_then(|id| self.folder(id)) {
                Some(folder) => {
                    messages.push_str(&format!(
                        "<m:GetFolderResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
                         <m:Folders>{}</m:Folders></m:GetFolderResponseMessage>",
                        folder_xml(folder, &shape)
                    ));
                    "NoError"
                }
                None => {
                    messages.push_str(&error_message("GetFolder", "ErrorFolderNotFound", "<m:Folders/>"));
                    "ErrorFolderNotFound"
                }
            };
            if first.is_empty() {
                first = code;
            }
        }
        Ok((first, response("GetFolder", &messages)))
    }

    /// What a hostile stand-in answers instead: a status and a body.
    fn misbehave(&self, how: Hostile, honest: String) -> (u16, String) {
        match how {
            Hostile::Dtd => {
                let inner = honest.split_once("?>").map_or(honest.as_str(), |(_, rest)| rest);
                (200, format!("<?xml version=\"1.0\"?><!DOCTYPE s [<!ENTITY a \"aaaaaaaaaa\"><!ENTITY b \"&a;&a;&a;&a;&a;&a;&a;&a;\">]>{inner}"))
            }
            Hostile::CutShort => (200, honest.chars().take(honest.chars().count() / 2).collect()),
            Hostile::NotXml => (200, "<html><body><h1>Sign in</h1><p>Nothing of the service is here.</body></html>".into()),
            Hostile::Deep => (200, format!("{}{}", "<a>".repeat(20_000), "</a>".repeat(20_000))),
            Hostile::Huge => {
                (200, format!("<?xml version=\"1.0\"?><s:Envelope xmlns:s=\"{SOAP}\"><s:Body>{}</s:Body></s:Envelope>", "x".repeat(9 * 1024 * 1024)))
            }
            Hostile::Fault => {
                (500, envelope("<s:Fault><faultcode>s:Server</faultcode><faultstring>An internal server error occurred.</faultstring></s:Fault>"))
            }
            Hostile::Endless | Hostile::TooFew => (200, honest),
        }
    }
}

/// Read one request from `stream`: the request line, the headers, and as much body as it says.
fn read_request(stream: &TcpStream) -> Option<(String, String, Asked)> {
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok().filter(|n| *n > 0)?;
    let mut words = line.split_whitespace();
    let (method, target) = (words.next()?.to_string(), words.next()?.to_string());
    let (mut headers, mut length) = (Vec::new(), 0usize);
    for _ in 0..200 {
        let mut line = String::new();
        if reader.read_line(&mut line).ok()? == 0 || line.trim().is_empty() {
            break;
        }
        let (name, value) = line.split_once(':')?;
        let (name, value) = (name.trim().to_ascii_lowercase(), value.trim().to_string());
        if name == "content-length" {
            length = value.parse().ok().filter(|n| *n <= MAX_REQUEST)?;
        }
        headers.push((name, value));
    }
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).ok()?;
    Some((method, target, Asked { body: String::from_utf8_lossy(&body).into_owned(), headers, ..Default::default() }))
}

/// A stand-in for Exchange Web Services on a loopback port, with the mailbox it keeps.
pub struct StandIn {
    pub port: u16,
    state: Arc<Mutex<State>>,
}

impl StandIn {
    /// Start one: a mailbox with its root, the Common Views folder under it, the top of the
    /// folders people see and the special folders under that, and no shortcuts; taking [`TOKEN`].
    pub fn start() -> std::io::Result<StandIn> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let mut state = State { token: TOKEN.into(), ..Default::default() };
        let root = state.add("Root", "", "root");
        state.add("Common Views", &root, "");
        state.add("Finder", &root, "searchfolders");
        let top = state.add("Top of Information Store", &root, "msgfolderroot");
        let special = [
            ("Inbox", "inbox"),
            ("Drafts", "drafts"),
            ("Sent Items", "sentitems"),
            ("Deleted Items", "deleteditems"),
            ("Junk Email", "junkemail"),
            ("Outbox", "outbox"),
        ];
        for (name, distinguished) in special {
            state.add(name, &top, distinguished);
        }
        // (Archive has no distinguished id in this mailbox: it is a folder like any other.)
        state.add("Archive", &top, "");
        let state = Arc::new(Mutex::new(state));
        let shared = state.clone();
        std::thread::Builder::new().name("ews stand-in".into()).spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let Some((method, target, mut asked)) = read_request(&stream) else { continue };
                let mut state = shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
                let (status, code, body, policy) = state.respond(&method, &target, &mut asked);
                asked.status = status;
                asked.code = code;
                state.requests.push(asked);
                drop(state);
                let reason = match status {
                    200 => "OK",
                    401 => "Unauthorized",
                    403 => "Forbidden",
                    404 => "Not Found",
                    405 => "Method Not Allowed",
                    415 => "Unsupported Media Type",
                    500 => "Internal Server Error",
                    _ => "Other",
                };
                let policy = policy.map(|reason| format!("X-EWS-Policy-Reason: {reason}\r\n")).unwrap_or_default();
                let head = format!(
                    "HTTP/1.1 {status} {reason}\r\nContent-Type: text/xml; charset=utf-8\r\n{policy}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes()).and_then(|()| stream.write_all(body.as_bytes()));
            }
        })?;
        Ok(StandIn { port, state })
    }

    /// `http://127.0.0.1:{port}`.
    pub fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// What it keeps, to set up and to look at.
    pub fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Have this thread's calls to Exchange Web Services come here until the guard is dropped.
    pub fn route(&self) -> Routed {
        route(&self.url())
    }
}

impl State {
    /// The whole answer to a request: status, response code, body, and the policy header's
    /// reason when the organisation turned it away.
    fn respond(&mut self, method: &str, target: &str, asked: &mut Asked) -> (u16, String, String, Option<String>) {
        if target.split('?').next() != Some("/EWS/Exchange.asmx") {
            return (404, String::new(), String::new(), None);
        }
        if method != "POST" {
            return (405, String::new(), String::new(), None);
        }
        // The organisation's policy comes before anything of the request is looked at.
        if let Some(reason) = self.blocked.clone() {
            return (403, String::new(), "<html><body>Forbidden</body></html>".into(), Some(reason));
        }
        if self.forbidden {
            return (403, String::new(), "<html><body>Forbidden</body></html>".into(), None);
        }
        if asked.header("authorization") != format!("Bearer {}", self.token) {
            return (401, String::new(), String::new(), None);
        }
        if !asked.header("content-type").to_ascii_lowercase().starts_with("text/xml") {
            return (415, String::new(), String::new(), None);
        }
        if self.busy > 0 {
            self.busy -= 1;
            return (
                500,
                "ErrorServerBusy".into(),
                fault("ErrorServerBusy", "The server cannot service this request right now. Try again later."),
                None,
            );
        }
        let (status, code, body) = match self.answer(asked) {
            Ok((status, code, body)) => (status, code.to_string(), body),
            Err(refusal) => (refusal.status, refusal.code.to_string(), fault(refusal.code, &refusal.why)),
        };
        match self.hostile {
            Some(how) if status == 200 => {
                let (status, body) = self.misbehave(how, body);
                (status, code, body, None)
            }
            _ => (status, code, body, None),
        }
    }
}

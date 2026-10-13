//! The navigation shortcuts of classic Outlook ("WunderBar" links), of which the ones under
//! Favorites in the Mail pane are the list that is read: which of them count, and in what order.
//!
//! [MS-OXOCFG] (Configuration Information Protocol, sections 2.2.9 and 3.1.4.9–10) has them as
//! hidden messages of class `IPM.Microsoft.WunderBar.Link` in the Common Views folder, each
//! with these properties (the tag, and the type the property pages give):
//!
//! | Tag | Property | Type | What is read of it |
//! |---|---|---|---|
//! | 0x6849 | PidTagWlinkType | Integer32 | 0 a folder, 1 a search folder, 2 another user's folder, 4 the heading of a group |
//! | 0x6852 | PidTagWlinkSection | Integer32 | 1 the Favorites of Mail (3 Calendar, 4 Contacts, 5 Tasks, 6 Notes, 7 Journal) |
//! | 0x684B | PidTagWlinkOrdinal | Binary | where it comes among the others: compared byte by byte |
//! | 0x684C | PidTagWlinkEntryId | Binary | the EntryID of the folder it is a shortcut to |
//! | 0x6850 | PidTagWlinkGroupClsid | Binary, or GUID | the group it is listed in |
//! | 0x6842 | PidTagWlinkGroupHeaderID | Binary, or GUID | of a heading: the group it is the heading of |
//!
//! Nothing else of a shortcut is asked for: not the name it is shown by (a folder is called
//! what its mailbox calls it), not the store it points into, not which client made it.
//!
//! **The two group properties are of a type the documents disagree on.** Their property pages
//! (2.2.9.3, 2.2.9.12) and [MS-OXPROPS] say `PtypBinary`; the examples of the same document
//! (4.4.1, 4.4.2) show them as `PtypGuid`. So both are asked for, each under both types, and a
//! value is read whichever way it comes: sixteen bytes in base64, or a GUID written out. When
//! neither comes (or the server won't be asked that way at all), the shortcuts are simply one
//! group, which is what Outlook's Mail pane has anyway.
//!
//! **Which count.** A shortcut of the Mail Favorites (section 1) to a folder (type 0). One to a
//! search folder or to another user's folder is left out and counted
//! ([`Wanted::skipped`]); a heading is structure, and anything of another section is another
//! list. Whether the folder is one of this mailbox today is for whoever has the folders to say
//! ([`key_of`]): a shortcut to one that is gone, or into another store, names none of them.
//!
//! **In what order.** "Sorted lexicographically" by the ordinal (2.2.9.7): byte by byte, a
//! shorter one before a longer one that begins with it. Groups come in the order of their
//! headings' ordinals, when every shortcut says which group it is in and each has a heading;
//! otherwise all of them are one list. Shortcuts with the same ordinal, and those without one
//! (which come last), stay in the order the server gave them.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;

use super::xml::Node;

/// The properties of a shortcut that are asked for, by their tags.
pub(super) const TYPE: u16 = 0x6849;
pub(super) const SECTION: u16 = 0x6852;
pub(super) const ORDINAL: u16 = 0x684B;
pub(super) const ENTRY_ID: u16 = 0x684C;
pub(super) const GROUP: u16 = 0x6850;
pub(super) const GROUP_HEADER: u16 = 0x6842;

/// What a shortcut's type says it is a shortcut to (`PidTagWlinkType`).
const FOLDER: i64 = 0;
const SEARCH_FOLDER: i64 = 1;
const SHARED_FOLDER: i64 = 2;
const HEADER: i64 = 4;
/// The section of the Favorites of Mail (`wbsidMailFavorites`).
const MAIL_FAVORITES: i64 = 1;
/// Longest ordinal and longest EntryID that are taken (bytes): a folder's EntryID is 46.
const MAX_ORDINAL: usize = 256;
const MAX_ENTRY_ID: usize = 512;
/// Most shortcuts put in order.
pub(super) const MAX_LINKS: usize = 2_000;

/// One shortcut, as far as it is read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Link {
    /// `PidTagWlinkType`.
    pub(super) kind: Option<i64>,
    /// `PidTagWlinkSection`.
    pub(super) section: Option<i64>,
    /// `PidTagWlinkOrdinal`; `None` when it has none that can be read.
    pub(super) ordinal: Option<Vec<u8>>,
    /// `PidTagWlinkEntryId`; `None` when it has none that can be read.
    pub(super) entry_id: Option<Vec<u8>>,
    /// `PidTagWlinkGroupClsid`: the group it is in.
    pub(super) group: Option<[u8; 16]>,
    /// `PidTagWlinkGroupHeaderID`: for a heading, the group it is the heading of.
    pub(super) header: Option<[u8; 16]>,
}

/// The tag an `ExtendedFieldURI` names: `0x684B` (one to four hex digits, in either case) or
/// the same as a number in decimal, as the schema has it ([MS-OXWSXPROP] 2.1.5).
pub(super) fn tag_of(text: &str) -> Option<u16> {
    let text = text.trim();
    match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        Some(hex) if (1..=4).contains(&hex.len()) && hex.bytes().all(|b| b.is_ascii_hexdigit()) => u16::from_str_radix(hex, 16).ok(),
        Some(_) => None,
        None if !text.is_empty() && text.len() <= 5 && text.bytes().all(|b| b.is_ascii_digit()) => text.parse().ok(),
        None => None,
    }
}

/// The bytes of a `Binary` value: base64, padded or not, of no more than `max` bytes.
pub(super) fn binary(text: &str, max: usize) -> Option<Vec<u8>> {
    let packed: String = text.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    let packed = packed.trim_end_matches('=');
    // (Four characters carry three bytes: what is longer than that is not looked at.)
    if packed.is_empty() || packed.len() > max.saturating_mul(4).div_ceil(3).saturating_add(4) {
        return None;
    }
    STANDARD_NO_PAD.decode(packed).ok().filter(|bytes| bytes.len() <= max)
}

/// A whole number as the service writes an `Integer`.
fn integer(text: &str) -> Option<i64> {
    let text = text.trim();
    (text.len() <= 11).then(|| text.parse::<i32>().ok()).flatten().map(i64::from)
}

/// A GUID whichever way it is written: sixteen bytes in base64 (`Binary`), or written out
/// (`CLSID`: `00067800-0000-0000-c000-000000000046`, with or without braces), as the bytes it
/// is stored as (the first three fields with their low byte first). The same GUID reads the
/// same both ways.
pub(super) fn guid(text: &str) -> Option<[u8; 16]> {
    let text = text.trim();
    let written = text.strip_prefix('{').and_then(|rest| rest.strip_suffix('}')).unwrap_or(text);
    if written.len() == 36 && written.is_ascii() {
        let fields: Vec<&str> = written.split('-').collect();
        let [a, b, c, d, e] = fields.as_slice() else { return None };
        if [a.len(), b.len(), c.len(), d.len(), e.len()] != [8, 4, 4, 4, 12] {
            return None;
        }
        let hex = |digits: &str, at: usize| digits.get(at..at + 2).and_then(|pair| u8::from_str_radix(pair, 16).ok());
        let mut out = [0u8; 16];
        // (The first three as numbers, low byte first; the rest as the bytes they are.)
        let places: [(&str, usize, usize); 16] = [
            (a, 6, 0),
            (a, 4, 1),
            (a, 2, 2),
            (a, 0, 3),
            (b, 2, 4),
            (b, 0, 5),
            (c, 2, 6),
            (c, 0, 7),
            (d, 0, 8),
            (d, 2, 9),
            (e, 0, 10),
            (e, 2, 11),
            (e, 4, 12),
            (e, 6, 13),
            (e, 8, 14),
            (e, 10, 15),
        ];
        for (digits, at, to) in places {
            *out.get_mut(to)? = hex(digits, at)?;
        }
        return Some(out);
    }
    <[u8; 16]>::try_from(binary(text, 16)?.as_slice()).ok()
}

/// The shortcut in an item of the service's answer: its `ExtendedProperty` elements, each an
/// `ExtendedFieldURI` (which says the tag) and a `Value`. A property that is there twice (asked
/// for under two types, and answered under both) counts the first time it can be read.
pub(super) fn link_of(item: &Node) -> Link {
    let mut link = Link::default();
    for property in item.all("ExtendedProperty").take(32) {
        let Some(tag) = property.child("ExtendedFieldURI").and_then(|uri| uri.attribute("PropertyTag")).and_then(tag_of) else { continue };
        let value = property.text_of("Value");
        match tag {
            TYPE => link.kind = link.kind.or(integer(value)),
            SECTION => link.section = link.section.or(integer(value)),
            ORDINAL => link.ordinal = link.ordinal.take().or_else(|| binary(value, MAX_ORDINAL)),
            ENTRY_ID => link.entry_id = link.entry_id.take().or_else(|| binary(value, MAX_ENTRY_ID)),
            GROUP => link.group = link.group.or_else(|| guid(value)),
            GROUP_HEADER => link.header = link.header.or_else(|| guid(value)),
            _ => {}
        }
    }
    link
}

/// What of an EntryID names the folder, whoever wrote it down: for the EntryID of a folder
/// ([MS-OXCDATA] 2.2.4.1: four bytes of flags, the provider's sixteen, two of folder type, the
/// database's GUID, six of counter and two of padding, 46 in all) the database's GUID and the
/// counter, which are the folder's own for good; for anything else, all of it.
pub(super) fn key_of(entry_id: &[u8]) -> &[u8] {
    match entry_id.len() {
        46 => entry_id.get(22..44).unwrap_or(entry_id),
        _ => entry_id,
    }
}

/// The folders the Favorites of Mail name, in Outlook's order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Wanted {
    /// The EntryIDs of the folders, each folder once.
    pub(super) folders: Vec<Vec<u8>>,
    /// Shortcuts that are not to folders of a mailbox (search folders, other users' folders),
    /// and those that don't say what they are to.
    pub(super) skipped: u32,
    /// Shortcuts to a folder that name none that can be looked for.
    pub(super) unresolved: u32,
}

/// Put the shortcuts in order ([`self`] says which count and how they are ordered).
pub(super) fn wanted(links: &[Link]) -> Wanted {
    let mut out = Wanted::default();
    let mail: Vec<&Link> = links.iter().take(MAX_LINKS).filter(|link| link.section == Some(MAIL_FAVORITES)).collect();
    // The headings of the groups: which group, and where it comes.
    let headings: Vec<([u8; 16], &[u8])> =
        mail.iter().filter(|link| link.kind == Some(HEADER)).filter_map(|link| Some((link.header?, link.ordinal.as_deref()?))).collect();
    let heading = |link: &Link| link.group.and_then(|group| headings.iter().find(|(id, _)| *id == group)).map(|(_, ordinal)| *ordinal);
    let mut folders: Vec<&Link> = Vec::new();
    for link in &mail {
        match link.kind {
            Some(FOLDER) if link.entry_id.as_ref().is_some_and(|id| !id.is_empty()) => folders.push(link),
            Some(FOLDER) => out.unresolved = out.unresolved.saturating_add(1),
            Some(HEADER) => {}
            Some(SEARCH_FOLDER | SHARED_FOLDER) | Some(_) | None => out.skipped = out.skipped.saturating_add(1),
        }
    }
    // By group only when every one of them says its group and that group has a heading.
    let grouped = !folders.is_empty() && folders.iter().all(|link| heading(link).is_some());
    let mut ordered: Vec<(&[u8], bool, &[u8], usize, &Link)> = folders
        .iter()
        .enumerate()
        .map(|(n, link)| {
            let group = if grouped { heading(link).unwrap_or(&[]) } else { &[] };
            (group, link.ordinal.is_none(), link.ordinal.as_deref().unwrap_or(&[]), n, *link)
        })
        .collect();
    ordered.sort_by(|a, b| (a.0, a.1, a.2, a.3).cmp(&(b.0, b.1, b.2, b.3)));
    for (_, _, _, _, link) in ordered {
        let Some(entry_id) = &link.entry_id else { continue };
        // (The same folder named twice is one favourite.)
        if !out.folders.iter().any(|have| key_of(have) == key_of(entry_id)) {
            out.folders.push(entry_id.clone());
        }
    }
    out
}

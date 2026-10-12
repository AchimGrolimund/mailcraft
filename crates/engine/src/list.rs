//! The message list: which messages show for the folder, filter and search in view, in what
//! order, under which group headings.

use std::collections::HashMap;

use mailcraft_model::search::Query;
use mailcraft_model::{FlagStatus, FolderKind, Id, Importance, Message};
use mailcraft_time::{DateGroup, Timestamp};
use serde::Serialize;

use crate::{Filter, Scope, Session, SortBy};

/// One row of the message list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    /// The message the row shows (the newest of its conversation).
    pub id: Id,
    /// Every message the row stands for, newest first (just `id` outside conversation view).
    pub members: Vec<Id>,
    pub unread: usize,
    /// The group heading above this row, when it starts a group (`Today`, `Last Week`).
    pub group: Option<String>,
}

/// When a message counts as having arrived: its received time, or its date when it has none.
pub fn when(m: &Message) -> Timestamp {
    if m.received != 0 { m.received } else { m.date }
}

fn group_label(s: &Session, m: &Message) -> Option<String> {
    if m.pinned {
        return Some("Pinned".into());
    }
    match s.view.sort {
        SortBy::Date => Some(DateGroup::of(s.local(when(m)).date, s.today(), s.prefs.week_start).label().to_string()),
        SortBy::From => Some(m.from.display().to_string()),
        SortBy::Importance => Some(
            match m.importance {
                Importance::High => "High",
                Importance::Normal => "Normal",
                Importance::Low => "Low",
            }
            .to_string(),
        ),
        SortBy::Subject | SortBy::Size => None,
    }
}

pub fn rows(s: &Session) -> Vec<Row> {
    let mb = &s.mailbox;
    let query = Query::parse(&s.view.search);
    let everywhere = !query.is_empty() && s.view.scope == Scope::Everywhere;
    // Deleted and junk mail stays out of a search of everything.
    let hidden: Vec<Id> = if everywhere {
        mb.folders.iter().filter(|f| matches!(f.kind, FolderKind::Deleted | FolderKind::Junk)).flat_map(|f| mb.subtree(f.id)).collect()
    } else {
        Vec::new()
    };
    let me: Vec<&str> = mb.accounts.iter().map(|a| a.email.as_str()).filter(|e| !e.is_empty()).collect();
    let mut shown: Vec<&Message> = mb
        .messages
        .values()
        .map(|m| m.as_ref())
        .filter(|m| if everywhere { !hidden.contains(&m.folder) } else { m.folder == s.view.folder })
        .filter(|m| match s.view.filter {
            Filter::All => true,
            Filter::Unread => !m.read,
            Filter::Flagged => m.flag == FlagStatus::Flagged,
            Filter::Attachments => m.has_attachments(),
            Filter::ToMe => m.to.iter().any(|a| me.iter().any(|e| a.email.eq_ignore_ascii_case(e))),
        })
        .filter(|m| query.is_empty() || query.matches(m, s.offset_at(m.date)))
        .collect();

    let desc = s.view.sort_descending;
    shown.sort_by(|a, b| {
        let key = match s.view.sort {
            SortBy::Date => when(a).cmp(&when(b)),
            // Within one sender or subject, newest first when the names run A to Z.
            SortBy::From => a.from.display().to_lowercase().cmp(&b.from.display().to_lowercase()).then(when(b).cmp(&when(a))),
            SortBy::Subject => a.topic().to_lowercase().cmp(&b.topic().to_lowercase()).then(when(b).cmp(&when(a))),
            SortBy::Size => a.size().cmp(&b.size()),
            SortBy::Importance => a.importance.cmp(&b.importance).then(when(a).cmp(&when(b))),
        };
        // Pinned messages stay on top whichever way the list is sorted; ids break ties so the
        // order never depends on the map's iteration.
        b.pinned.cmp(&a.pinned).then(if desc { key.reverse() } else { key }).then(b.id.cmp(&a.id))
    });

    let mut out: Vec<Row> = Vec::new();
    if s.view.conversations && s.view.sort == SortBy::Date {
        // One row per conversation, where its newest (or, ascending, oldest) message sorts.
        let mut index: HashMap<(Id, String), usize> = HashMap::new();
        for m in &shown {
            match index.get(&(m.account, m.thread_key())) {
                Some(i) => {
                    if let Some(row) = out.get_mut(*i) {
                        row.members.push(m.id);
                        row.unread += usize::from(!m.read);
                    }
                }
                None => {
                    index.insert((m.account, m.thread_key()), out.len());
                    out.push(Row { id: m.id, members: vec![m.id], unread: usize::from(!m.read), group: None });
                }
            }
        }
        for row in &mut out {
            // Newest first inside a conversation, and the row shows the newest.
            row.members.sort_by_key(|id| std::cmp::Reverse(mb.message(*id).map(|m| (when(m), m.id))));
            if let Some(first) = row.members.first() {
                row.id = *first;
            }
        }
    } else {
        out.extend(shown.iter().map(|m| Row { id: m.id, members: vec![m.id], unread: usize::from(!m.read), group: None }));
    }

    let mut last: Option<String> = None;
    for row in &mut out {
        let label = mb.message(row.id).and_then(|m| group_label(s, m));
        if label != last {
            row.group = label.clone();
            last = label;
        }
    }
    out
}

/// The row a message is in.
pub fn position(rows: &[Row], id: Id) -> Option<usize> {
    rows.iter().position(|r| r.id == id || r.members.contains(&id))
}

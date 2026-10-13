//! Where a folder that a server lists among the Favorites lies, said the way the mailbox finds
//! its own folder for it: the names from the top of the mailbox down to it, and which of the
//! special folders it is. Whoever read the list (Microsoft Graph, Exchange Web Services) knows
//! folders by what the server calls them; nothing of that goes further than here.

use std::collections::HashMap;

use mailcraft_model::FolderKind;
use mailcraft_model::remote::{MAX_FAVORITE_DEPTH, RemoteFavorite};

/// The favourite that the folder `id` is, called `name` and lying in the folder `parent`.
/// `known` says of every folder above it that could be asked about what it is called and where
/// it lies in turn; `top` is the folder a path ends at (the top of the mailbox), and a folder
/// that lies in nothing is at the top too; `kind_of` says which special folder a folder is.
///
/// `None` for a folder whose way up can't be followed to the top (a folder above it couldn't be
/// read, or they lie in each other in a ring) unless it is a special folder, which the mailbox
/// finds by what it is.
pub(crate) fn favorite_of(
    id: &str,
    name: &str,
    parent: &str,
    known: &HashMap<String, (String, String)>,
    top: Option<&str>,
    kind_of: &dyn Fn(&str) -> FolderKind,
) -> Option<RemoteFavorite> {
    let mut path = vec![name.to_string()];
    let (mut first, mut next, mut whole) = (id, parent, false);
    for _ in 0..MAX_FAVORITE_DEPTH {
        if next.is_empty() || top == Some(next) {
            whole = true;
            break;
        }
        // (A ring of folders, or a folder whose parent can't be read, has no path.)
        let Some((name, parent)) = known.get(next) else { break };
        path.push(name.clone());
        (first, next) = (next, parent.as_str());
    }
    path.reverse();
    let kind = kind_of(id);
    match (whole, kind) {
        (true, _) => Some(RemoteFavorite { path, kind, root: kind_of(first) }),
        (false, FolderKind::Custom) => None,
        (false, _) => Some(RemoteFavorite { path: Vec::new(), kind, root: kind }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(folders: &[(&str, &str, &str)]) -> HashMap<String, (String, String)> {
        folders.iter().map(|(id, name, parent)| (id.to_string(), (name.to_string(), parent.to_string()))).collect()
    }

    #[test]
    fn a_path_runs_from_the_top_down_and_a_ring_has_none() {
        let folders = known(&[("in", "Inbox", "top"), ("p", "Projects", "in"), ("a", "Apollo", "p"), ("x", "X", "y"), ("y", "Y", "x")]);
        let kind = |id: &str| if id == "in" { FolderKind::Inbox } else { FolderKind::Custom };
        let launch = favorite_of("l", "Launch", "a", &folders, Some("top"), &kind).unwrap();
        assert_eq!((launch.path.join("/"), launch.kind, launch.root), ("Inbox/Projects/Apollo/Launch".into(), FolderKind::Custom, FolderKind::Inbox));
        // The Inbox itself, whatever it is called there.
        let inbox = favorite_of("in", "Posteingang", "top", &folders, Some("top"), &kind).unwrap();
        assert_eq!((inbox.path, inbox.kind, inbox.root), (vec!["Posteingang".to_string()], FolderKind::Inbox, FolderKind::Inbox));
        // A ring, and a folder whose parent nobody knows: no path, so no favourite.
        assert_eq!(favorite_of("z", "Z", "x", &folders, Some("top"), &kind), None);
        assert_eq!(favorite_of("z", "Z", "gone", &folders, Some("top"), &kind), None);
        // A special folder is found by what it is all the same.
        let lost = favorite_of("in", "Inbox", "gone", &folders, Some("top"), &kind).unwrap();
        assert_eq!((lost.path.len(), lost.kind, lost.root), (0, FolderKind::Inbox, FolderKind::Inbox));
        // A folder that lies in nothing is at the top.
        assert_eq!(favorite_of("r", "Receipts", "", &folders, None, &kind).unwrap().path, ["Receipts"]);
    }
}

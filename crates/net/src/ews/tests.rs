//! Reading the Favorites through Exchange Web Services, against a stand-in that takes only what
//! the documented service takes (`standin`), and against whatever a server could send instead.
//! Nothing here leaves this machine, and every name and id is invented.

use mailcraft_model::FolderKind;
use mailcraft_model::remote::{FavoritesOutcome, RemoteFavorite};
use proptest::prelude::*;

use super::links::tag_of;
use super::standin::{BLOCKED_BY_POLICY, DISABLED, Guids, Hostile, Shortcut, StandIn, TOKEN};
use super::*;

/// The folders of the tests, by what the stand-in calls them.
struct Mailbox {
    server: StandIn,
    inbox: String,
    sent: String,
    projects: String,
    launch: String,
    receipts: String,
    team: String,
}

/// A mailbox with Inbox/Projects/Apollo/Launch, Receipts and Team Lumen beside the special
/// folders, and no shortcuts yet.
fn mailbox() -> Mailbox {
    let server = StandIn::start().unwrap();
    let (inbox, sent, projects, launch, receipts, team) = {
        let mut state = server.state();
        let (inbox, sent) = (state.distinguished("inbox"), state.distinguished("sentitems"));
        let projects = state.add_folder("Projects", Some(&inbox));
        let apollo = state.add_folder("Apollo", Some(&projects));
        let launch = state.add_folder("Launch", Some(&apollo));
        let receipts = state.add_folder("Receipts", None);
        let team = state.add_folder("Team Lumen", None);
        (inbox, sent, projects, launch, receipts, team)
    };
    Mailbox { server, inbox, sent, projects, launch, receipts, team }
}

impl Mailbox {
    fn read(&self) -> Result<FavoritesDownload, NetError> {
        favorites_at(&format!("{}{PATH}", self.server.url()), TOKEN)
    }

    /// The Favorites as they are read: each folder's path, kind and root.
    fn listed(&self) -> Vec<(String, FolderKind, FolderKind)> {
        let download = self.read().unwrap();
        assert_eq!(download.outcome, FavoritesOutcome::Read);
        download.folders.iter().map(|folder: &RemoteFavorite| (folder.path.join("/"), folder.kind, folder.root)).collect()
    }
}

fn custom(path: &str) -> (String, FolderKind, FolderKind) {
    (path.to_string(), FolderKind::Custom, FolderKind::Custom)
}

#[test]
fn the_favorites_arrive_in_outlooks_order_and_what_is_no_folder_here_is_left_out() {
    let mailbox = mailbox();
    {
        let mut state = mailbox.server.state();
        // Put there in no order at all: the ordinals say where each comes.
        state.add_favorite(&mailbox.team, &[0xC0]);
        state.add_favorite(&mailbox.receipts, &[0x40]);
        state.add_favorite(&mailbox.launch, &[0x80, 0x80]);
        state.add_favorite(&mailbox.inbox, &[0x80]);
        state.add_favorite(&mailbox.sent, &[0x81]);
        // The same folder a second time; a folder of another mailbox; one that is long gone.
        state.add_favorite(&mailbox.receipts, &[0xD0]);
        state.add_favorite_elsewhere(&[0x50]);
        state.add_stale_favorite(&[0x60]);
        // A search folder, another user's folder, and a shortcut of the Calendar's list.
        let anything = state.entry_id_of(&mailbox.projects);
        let odd = |kind: i32, section: i32| Shortcut {
            kind: Some(kind),
            section: Some(section),
            ordinal: Some(vec![0x10]),
            entry_id: Some(anything.clone()),
            ..Shortcut::link()
        };
        state.shortcuts.push(odd(1, 1));
        state.shortcuts.push(odd(2, 1));
        state.shortcuts.push(odd(0, 3));
        // And a hidden item of the folder that is no shortcut at all.
        state.shortcuts.push(Shortcut { class: "IPM.Microsoft.FolderDesign.NamedView".into(), ..odd(0, 1) });
    }
    let download = mailbox.read().unwrap();
    let listed: Vec<(String, FolderKind, FolderKind)> = download.folders.iter().map(|f| (f.path.join("/"), f.kind, f.root)).collect();
    assert_eq!(
        listed,
        [
            custom("Receipts"),
            ("Inbox".into(), FolderKind::Inbox, FolderKind::Inbox),
            ("Inbox/Projects/Apollo/Launch".into(), FolderKind::Custom, FolderKind::Inbox),
            ("Sent Items".into(), FolderKind::Sent, FolderKind::Sent),
            custom("Team Lumen"),
        ]
    );
    assert_eq!((download.outcome, download.skipped, download.unresolved), (FavoritesOutcome::Read, 2, 2));

    // Four questions, all of them reads, each as the reference has it.
    let state = mailbox.server.state();
    let asked: Vec<(&str, &str, u16, &str)> =
        state.requests.iter().map(|asked| (asked.operation.as_str(), asked.traversal.as_str(), asked.status, asked.code.as_str())).collect();
    assert_eq!(
        asked,
        [
            ("FindFolder", "Shallow", 200, "NoError"),
            ("FindItem", "Associated", 200, "NoError"),
            ("GetFolder", "", 200, "NoError"),
            ("FindFolder", "Deep", 200, "NoError")
        ]
    );
    assert!(state.writes().is_empty());
    let first = &state.requests[0];
    assert_eq!(first.header("authorization"), format!("Bearer {TOKEN}"));
    assert_eq!((first.header("content-type"), first.header("accept"), first.header("soapaction")), ("text/xml; charset=utf-8", "text/xml", ""));
    assert!(
        first.body.contains(r#"<t:RequestServerVersion Version="Exchange2013_SP1"/>"#) && first.body.contains(r#"Constant Value="Common Views""#)
    );
    // Nothing is asked of a shortcut but what is read of it: not its name, not the store.
    let links = &state.requests[1].body;
    for tag in ["0x6849", "0x6852", "0x684B", "0x684C", "0x6850", "0x6842"] {
        assert!(links.contains(tag), "{tag}");
    }
    for never in ["item:Subject", "0x684E", "0x684D", "0x6851", "0x6890", "0x6847", "CreateItem", "DeleteItem", "UpdateItem"] {
        assert!(!state.requests.iter().any(|asked| asked.body.contains(never)), "{never}");
    }
}

#[test]
fn the_whole_list_is_read_however_small_the_pages_are() {
    let mailbox = mailbox();
    let mut folders = Vec::new();
    {
        let mut state = mailbox.server.state();
        // Pages of two, of the shortcuts and of the folders alike.
        state.page = 2;
        for n in 0..9u8 {
            let id = state.add_folder(&format!("Folder {n}"), Some(&mailbox.projects));
            // The last made comes first.
            state.add_favorite(&id, &[0xF0 - n]);
            folders.push(format!("Inbox/Projects/Folder {n}"));
        }
    }
    folders.reverse();
    let listed: Vec<String> = mailbox.listed().into_iter().map(|(path, _, _)| path).collect();
    assert_eq!(listed, folders);
    let state = mailbox.server.state();
    // Nine shortcuts in five pages; the twenty-one folders in eleven, after the one for the
    // folder the shortcuts are in.
    assert_eq!((state.asked("FindItem"), state.asked("FindFolder")), (5, 12));
    assert!(state.writes().is_empty());
}

#[test]
fn groups_come_in_the_order_of_their_headings_whichever_type_their_ids_have() {
    const WORK: [u8; 16] = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xF0, 0x01];
    const HOME: [u8; 16] = [0xA1; 16];
    let build = |guids: Guids, refuses: bool| {
        let mailbox = mailbox();
        {
            let mut state = mailbox.server.state();
            (state.guids, state.refuses_groups) = (guids, refuses);
            // "Home" is listed before "Work", although its shortcuts have the later ordinals.
            state.shortcuts.push(Shortcut::heading(HOME, &[0x20]));
            state.shortcuts.push(Shortcut::heading(WORK, &[0x90]));
            for (folder, ordinal, group) in
                [(&mailbox.receipts, 0x70u8, HOME), (&mailbox.team, 0x30, WORK), (&mailbox.inbox, 0x60, HOME), (&mailbox.sent, 0x10, WORK)]
            {
                let entry_id = Some(state.entry_id_of(folder));
                state.shortcuts.push(Shortcut {
                    kind: Some(0),
                    section: Some(1),
                    ordinal: Some(vec![ordinal]),
                    entry_id,
                    group: Some(group),
                    ..Shortcut::link()
                });
            }
        }
        mailbox
    };
    let by_group = ["Inbox", "Receipts", "Sent Items", "Team Lumen"];
    let flat = ["Sent Items", "Team Lumen", "Inbox", "Receipts"];
    let paths = |mailbox: &Mailbox| mailbox.listed().into_iter().map(|(path, _, _)| path).collect::<Vec<_>>();
    // Kept as binary (the property pages) or as GUIDs (the examples): read either way.
    for guids in [Guids::Binary, Guids::Clsid] {
        let mailbox = build(guids, false);
        assert_eq!(paths(&mailbox), by_group, "{guids:?}");
        assert_eq!(mailbox.server.state().asked("FindItem"), 1);
    }
    // A service that won't be asked for them at all is asked without, once: one list.
    let mailbox = build(Guids::Binary, true);
    assert_eq!(paths(&mailbox), flat);
    let state = mailbox.server.state();
    let codes: Vec<&str> = state.requests.iter().filter(|asked| asked.operation == "FindItem").map(|asked| asked.code.as_str()).collect();
    assert_eq!(codes, ["ErrorInvalidPropertyRequest", "NoError"]);
    let again = state.requests.iter().filter(|asked| asked.operation == "FindItem").nth(1).unwrap();
    assert!(!again.body.contains("CLSID") && !again.body.contains("0x6850") && again.body.contains("0x684B"));
    // A shortcut whose group has no heading: nobody can say where the group comes. One list.
    let mailbox = build(Guids::Clsid, false);
    mailbox.server.state().shortcuts.remove(0);
    assert_eq!(paths(&mailbox), flat);
}

#[test]
fn an_organisation_that_does_not_allow_it_is_told_by_the_header_and_a_token_that_is_not_taken_by_the_status() {
    let mailbox = mailbox();
    mailbox.server.state().add_favorite(&mailbox.inbox, &[0x80]);
    for reason in [DISABLED, BLOCKED_BY_POLICY, "Some reason nobody has documented yet"] {
        mailbox.server.state().blocked = Some(reason.into());
        let error = mailbox.read().unwrap_err();
        assert!(matches!(&error, NetError::Blocked(_)), "{error:?}");
        // Our own words, never the server's.
        assert!(!error.to_string().contains(reason) && error.to_string().contains("organisation"), "{error}");
    }
    // One question was enough each time: nothing is tried a second way.
    assert_eq!(mailbox.server.state().requests.len(), 3);
    // A 403 that doesn't say whose no it is.
    let mut state = mailbox.server.state();
    (state.blocked, state.forbidden) = (None, true);
    drop(state);
    assert!(matches!(mailbox.read().unwrap_err(), NetError::Forbidden(_)));
    // A token the service doesn't take.
    let mut state = mailbox.server.state();
    (state.forbidden, state.token) = (false, "another".into());
    drop(state);
    let error = mailbox.read().unwrap_err();
    assert!(error.is_refused_sign_in() && !error.to_string().contains(TOKEN), "{error}");
    // A busy service: said so, for whoever asks to come back.
    let mut state = mailbox.server.state();
    (state.token, state.busy) = (TOKEN.into(), 1);
    drop(state);
    assert!(matches!(mailbox.read().unwrap_err(), NetError::Busy { .. }));
    assert_eq!(mailbox.listed(), [("Inbox".to_string(), FolderKind::Inbox, FolderKind::Inbox)]);
    // A token that can't be one goes nowhere.
    let asked = mailbox.server.state().requests.len();
    for token in ["", "two words", "line\nbreak"] {
        assert!(favorites_at(&format!("{}{PATH}", mailbox.server.url()), token).unwrap_err().is_refused_sign_in());
    }
    assert_eq!(mailbox.server.state().requests.len(), asked);
    assert!(mailbox.server.state().writes().is_empty());
}

#[test]
fn a_mailbox_without_such_a_list_has_none_and_a_renamed_inbox_is_still_the_inbox() {
    // No shortcut of the Favorites of Mail: classic Outlook never kept a list here.
    let mailbox = mailbox();
    assert_eq!(mailbox.read().unwrap().outcome, FavoritesOutcome::Missing);
    let calendar = Shortcut { kind: Some(0), section: Some(3), ordinal: Some(vec![0x80]), ..Shortcut::link() };
    mailbox.server.state().shortcuts.push(calendar);
    assert_eq!(mailbox.read().unwrap().outcome, FavoritesOutcome::Missing);
    // The group is there and holds nothing: a list, and an empty one.
    mailbox.server.state().shortcuts.push(Shortcut::heading([7; 16], &[0x80]));
    let empty = mailbox.read().unwrap();
    assert_eq!((empty.outcome, empty.folders.len()), (FavoritesOutcome::Read, 0));
    // Only two questions were needed for that: the folders are not asked about for nothing.
    assert_eq!(mailbox.server.state().requests.len(), 2 + 2 + 2);

    // A mailbox in another language: the special folders are known by what they are.
    {
        let mut state = mailbox.server.state();
        state.add_favorite(&mailbox.inbox, &[0x70]);
        state.add_favorite(&mailbox.projects, &[0x71]);
        let archive = state.folders.iter().find(|folder| folder.name == "Archive").unwrap().id.clone();
        state.add_favorite(&archive, &[0x72]);
        state.folders.iter_mut().find(|folder| folder.id == mailbox.inbox).unwrap().name = "Posteingang".into();
    }
    assert_eq!(
        mailbox.listed(),
        [
            ("Posteingang".to_string(), FolderKind::Inbox, FolderKind::Inbox),
            ("Posteingang/Projects".to_string(), FolderKind::Custom, FolderKind::Inbox),
            // (There is no distinguished id for it: found by its name, like any other.)
            custom("Archive"),
        ]
    );

    // No Common Views folder at all.
    mailbox.server.state().folders.retain(|folder| folder.name != "Common Views");
    assert_eq!(mailbox.read().unwrap().outcome, FavoritesOutcome::Missing);
    assert!(mailbox.server.state().writes().is_empty());
}

#[test]
fn folders_that_lie_in_each_other_have_no_path_and_nothing_hangs() {
    let mailbox = mailbox();
    {
        let mut state = mailbox.server.state();
        let ring = state.add_folder("Ring", Some(&mailbox.projects));
        let inner = state.add_folder("Inner", Some(&ring));
        // Projects now lies in Inner, which lies in Ring, which lies in Projects.
        state.folders.iter_mut().find(|folder| folder.id == mailbox.projects).unwrap().parent = inner.clone();
        state.add_favorite(&inner, &[0x10]);
        state.add_favorite(&mailbox.launch, &[0x20]);
        state.add_favorite(&mailbox.receipts, &[0x30]);
    }
    let download = mailbox.read().unwrap();
    // (The ring is not under the top of the mailbox any more: the service doesn't list it.)
    assert_eq!(download.folders.iter().map(|f| f.path.join("/")).collect::<Vec<_>>(), ["Receipts"]);
    assert_eq!(download.unresolved, 2);
}

#[test]
fn whatever_the_service_answers_instead_is_an_error_and_never_a_list() {
    let mailbox = mailbox();
    mailbox.server.state().add_favorite(&mailbox.inbox, &[0x80]);
    for how in [Hostile::Dtd, Hostile::CutShort, Hostile::NotXml, Hostile::Deep, Hostile::Huge, Hostile::Fault, Hostile::Endless, Hostile::TooFew] {
        mailbox.server.state().hostile = Some(how);
        let outcome = mailbox.read();
        let error = outcome.expect_err(&format!("{how:?}"));
        // None of them is a sign-in that is over or a no of the organisation's: just not now.
        assert!(!error.is_refused_sign_in() && !matches!(error, NetError::Blocked(_) | NetError::Forbidden(_)), "{how:?}: {error:?}");
        assert!(!error.to_string().contains(TOKEN), "{how:?}");
    }
    mailbox.server.state().hostile = None;
    assert_eq!(mailbox.listed().len(), 1);
    // A list without end is given up after so many pages, not followed for ever.
    let endless = mailbox.server.state().requests.iter().filter(|asked| asked.operation == "FindItem").count();
    assert!(endless < 3 * MAX_LINK_PAGES, "{endless}");
    assert!(mailbox.server.state().writes().is_empty());
}

#[test]
fn the_stand_in_holds_a_request_to_the_schema() {
    let server = StandIn::start().unwrap();
    let ews = Ews::at(&format!("{}{PATH}", server.url()), TOKEN).unwrap();
    let refused = |body: &str, operation: &str| match ews.ask(operation, body) {
        Err(said) => said.error.to_string(),
        Ok(_) => panic!("taken: {body}"),
    };
    // An element in the wrong namespace, an attribute the element doesn't have, parts out of
    // order, a value the schema doesn't know, a tag that isn't one.
    let good = find_common_views();
    for (from, to) in [
        ("<m:FolderShape>", "<t:FolderShape>"),
        ("Traversal=\"Shallow\"", "Traversal=\"Shallow\" Depth=\"1\""),
        ("Traversal=\"Shallow\"", "Traversal=\"Sideways\""),
        ("<t:BaseShape>IdOnly</t:BaseShape>", "<t:BaseShape>Everything</t:BaseShape>"),
        ("Id=\"root\"", "Id=\"archive\""),
        ("BasePoint=\"Beginning\"", "BasePoint=\"Middle\""),
        ("folder:DisplayName", "folder:Nickname"),
    ] {
        assert!(good.contains(from), "{from}");
        let why = refused(&good.replace(from, to), "FindFolder");
        assert!(why.contains("ErrorSchemaValidation"), "{to}: {why}");
    }
    let swapped = good.replace("<m:ParentFolderIds><t:DistinguishedFolderId Id=\"root\"/></m:ParentFolderIds>", "");
    let swapped = swapped.replace("<m:FolderShape>", "<m:ParentFolderIds><t:DistinguishedFolderId Id=\"root\"/></m:ParentFolderIds><m:FolderShape>");
    assert!(refused(&swapped, "FindFolder").contains("ErrorSchemaValidation"));
    let links = find_links(&server.state().distinguished("root"), 0, true);
    for (from, to) in
        [("PropertyTag=\"0x6849\"", "PropertyTag=\"6849h\""), ("PropertyType=\"Integer\"", "PropertyType=\"Number\""), ("Associated", "Hidden")]
    {
        assert!(refused(&links.replacen(from, to, 1), "FindItem").contains("ErrorSchemaValidation"), "{to}");
    }
    // An operation that would change something is noted, and refused.
    let create = "<m:CreateItem MessageDisposition=\"SaveOnly\"><m:Items/></m:CreateItem>";
    let answers = ews.ask("CreateItem", create).unwrap();
    assert!(matches!(accepted(&answers[0], TOKEN), Err(said) if matches!(said.error, NetError::Forbidden(_))));
    assert_eq!(server.state().writes(), ["CreateItem"]);
    // What this crate asks is taken as it is.
    for (operation, body) in [("FindFolder", good), ("FindItem", links), ("GetFolder", get_special()), ("FindFolder", find_tree(0))] {
        assert!(ews.ask(operation, &body).is_ok(), "{operation}");
    }
}

#[test]
fn shortcuts_are_read_whichever_way_their_values_are_written() {
    // A tag as the schema allows it: hex with its prefix, in either case, or decimal.
    assert_eq!(
        [tag_of("0x684B"), tag_of("0X684b"), tag_of("0xfff"), tag_of("26699"), tag_of(" 0x1 ")],
        [Some(0x684B), Some(0x684B), Some(0xFFF), Some(0x684B), Some(1)]
    );
    assert_eq!([tag_of("684B"), tag_of("0x"), tag_of("0x12345"), tag_of("-1"), tag_of("70000"), tag_of("")], [None; 6]);
    // A GUID as sixteen bytes in base64, and the same written out, with or without braces.
    let bytes = [0x00, 0x78, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46];
    assert_eq!(links::guid("AHgGAAAAAADAAAAAAAAARg=="), Some(bytes));
    assert_eq!(links::guid("00067800-0000-0000-C000-000000000046"), Some(bytes));
    assert_eq!(links::guid(" {00067800-0000-0000-c000-000000000046} "), Some(bytes));
    for odd in [
        "",
        "00067800-0000-0000-C000-00000000004",
        "0006780-00000-0000-C000-000000000046",
        "AHgG",
        "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz",
        "é0067800-0000-0000-C000-00000000004",
    ] {
        assert_eq!(links::guid(odd), None, "{odd}");
    }
    // An EntryID of a folder is known by its database and its counter, whoever wrote the rest.
    let mut one = vec![0u8; 46];
    one[22..44].copy_from_slice(&[9; 22]);
    let mut other = one.clone();
    (other[0], other[5], other[45]) = (1, 0xEE, 3);
    assert_eq!(links::key_of(&one), links::key_of(&other));
    other[30] = 8;
    assert_ne!(links::key_of(&one), links::key_of(&other));
    assert_eq!(links::key_of(&[1, 2, 3]), [1, 2, 3]);
}

fn link(kind: i64, ordinal: Option<&[u8]>, entry: u8) -> Link {
    let mut entry_id = vec![0u8; 46];
    entry_id[43] = entry;
    Link { kind: Some(kind), section: Some(1), ordinal: ordinal.map(<[u8]>::to_vec), entry_id: Some(entry_id), ..Default::default() }
}

#[test]
fn ordinals_sort_byte_by_byte_and_what_has_none_comes_last() {
    // As [MS-OXOCFG] has it: between 128 and 129 goes 128, 128.
    let listed = [
        link(0, None, 1),
        link(0, Some(&[129]), 2),
        link(0, Some(&[128, 128]), 3),
        link(0, Some(&[128]), 4),
        link(0, Some(&[]), 5),
        link(0, None, 6),
    ];
    let order: Vec<u8> = links::wanted(&listed).folders.iter().map(|entry| entry[43]).collect();
    assert_eq!(order, [5, 4, 3, 2, 1, 6]);
    // Not by length, and not as numbers: 0x02 0xFF comes before 0x10.
    let listed = [link(0, Some(&[0x10]), 1), link(0, Some(&[0x02, 0xFF]), 2), link(0, Some(&[0x02]), 3)];
    assert_eq!(links::wanted(&listed).folders.iter().map(|entry| entry[43]).collect::<Vec<_>>(), [3, 2, 1]);
    // What is no folder is counted, a heading is not, and a folder that names nothing is unresolved.
    let nameless = Link { entry_id: None, ..link(0, Some(&[1]), 0) };
    let elsewhere = Link { section: Some(4), ..link(0, Some(&[1]), 9) };
    let untyped = Link { kind: None, ..link(0, Some(&[1]), 9) };
    let all = links::wanted(&[link(1, None, 9), link(2, None, 9), link(4, None, 9), link(7, None, 9), nameless, elsewhere, untyped]);
    assert_eq!((all.folders.len(), all.skipped, all.unresolved), (0, 4, 1));
}

/// An answer of the service's shape with whatever in it.
fn answer_like(parts: &[String]) -> String {
    format!(
        "<s:Envelope xmlns:s=\"{SOAP}\"><s:Body><m:FindItemResponse xmlns:m=\"{MESSAGES}\" xmlns:t=\"{TYPES}\"><m:ResponseMessages>\
         <m:FindItemResponseMessage ResponseClass=\"Success\"><m:ResponseCode>NoError</m:ResponseCode>\
         <m:RootFolder IncludesLastItemInRange=\"true\"><t:Items>{}</t:Items></m:RootFolder></m:FindItemResponseMessage>\
         </m:ResponseMessages></m:FindItemResponse></s:Body></s:Envelope>",
        parts.concat()
    )
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    /// Any bytes at all: an error or a tree, never a panic, and never a tree deeper or larger
    /// than is read.
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2_000)) {
        let _ = xml::parse(&bytes);
        let ews = Ews::at("https://ews.example/EWS/Exchange.asmx", TOKEN).unwrap();
        for status in [200u16, 403, 500] {
            let reply = Reply { status, body: bytes.clone(), retry_after: None, policy: None };
            prop_assert!(ews.read("FindItem", &reply).is_err() || status == 200);
        }
    }

    /// Markup that is nearly an answer: read or refused, never a panic; and what is read of a
    /// shortcut is within its limits.
    #[test]
    fn answer_shaped_noise_never_panics(
        pieces in proptest::collection::vec(
            prop_oneof![
                Just("<t:Message>".to_string()), Just("</t:Message>".to_string()), Just("<t:ExtendedProperty>".to_string()),
                Just("</t:ExtendedProperty>".to_string()), Just("<t:Value>".to_string()), Just("</t:Value>".to_string()),
                Just("<t:ExtendedFieldURI PropertyTag=\"0x684b\" PropertyType=\"Binary\"/>".to_string()),
                Just("<t:ExtendedFieldURI PropertyTag=\"0x6849\" PropertyType=\"Integer\"/>".to_string()),
                Just("<t:ExtendedFieldURI PropertyTag=\"0x6850\" PropertyType=\"CLSID\"/>".to_string()),
                Just("&amp;".to_string()), Just("&bogus;".to_string()), Just("&#x41;".to_string()), Just("<![CDATA[x]]>".to_string()),
                Just("<!DOCTYPE x>".to_string()), Just("<!-- c -->".to_string()),
                "[A-Za-z0-9+/=]{0,40}", "\\PC{0,12}", "-?[0-9]{1,12}", "[0-9a-f-]{30,40}",
            ],
            0..40,
        )
    ) {
        let ews = Ews::at("https://ews.example/EWS/Exchange.asmx", TOKEN).unwrap();
        let reply = Reply { status: 200, body: answer_like(&pieces).into_bytes(), retry_after: None, policy: None };
        if let Ok(answers) = ews.read("FindItem", &reply) {
            for answer in &answers {
                let items = answer.find(&["RootFolder", "Items"]).map(|items| items.children.as_slice()).unwrap_or_default();
                let read: Vec<Link> = items.iter().map(links::link_of).collect();
                for link in &read {
                    prop_assert!(link.ordinal.as_ref().is_none_or(|ordinal| ordinal.len() <= 256));
                    prop_assert!(link.entry_id.as_ref().is_none_or(|id| id.len() <= 512));
                }
                let _ = links::wanted(&read);
            }
        }
    }

    /// Whatever the ordinals are, the folders come out sorted by them byte by byte, those with
    /// the same one in the order they came, each folder once.
    #[test]
    fn any_ordinals_come_out_in_bytewise_order(ordinals in proptest::collection::vec(proptest::option::of(proptest::collection::vec(any::<u8>(), 0..5)), 0..40)) {
        let listed: Vec<Link> = ordinals.iter().enumerate().map(|(n, ordinal)| link(0, ordinal.as_deref(), n as u8)).collect();
        let wanted = links::wanted(&listed);
        prop_assert_eq!(wanted.folders.len(), listed.len());
        let order: Vec<usize> = wanted.folders.iter().map(|entry| usize::from(entry[43])).collect();
        let mut expected: Vec<usize> = (0..listed.len()).collect();
        expected.sort_by(|a, b| match (&ordinals[*a], &ordinals[*b]) {
            (Some(x), Some(y)) => x.as_slice().cmp(y.as_slice()).then(a.cmp(b)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.cmp(b),
        });
        prop_assert_eq!(order, expected);
    }

    /// A GUID reads the same as binary and written out, and nothing else reads as one.
    #[test]
    fn a_guid_reads_the_same_both_ways(bytes in any::<[u8; 16]>(), noise in "\\PC{0,50}") {
        use base64::Engine as _;
        let packed = base64::engine::general_purpose::STANDARD.encode(bytes);
        let [a0, a1, a2, a3, b0, b1, c0, c1, d0, d1, e0, e1, e2, e3, e4, e5] = bytes;
        let written = format!("{a3:02X}{a2:02x}{a1:02x}{a0:02x}-{b1:02x}{b0:02x}-{c1:02x}{c0:02x}-{d0:02x}{d1:02x}-{e0:02x}{e1:02x}{e2:02x}{e3:02x}{e4:02x}{e5:02x}");
        prop_assert_eq!(links::guid(&packed), Some(bytes));
        prop_assert_eq!(links::guid(&written), Some(bytes));
        let _ = (links::guid(&noise), links::tag_of(&noise), links::binary(&noise, 64));
    }

    /// Text survives being written into a request and read back.
    #[test]
    fn text_survives_a_request(text in "[ -~\u{a0}-\u{2ff}]{0,60}") {
        let document = format!("<a b=\"{}\">{}</a>", xml::escape(&text), xml::escape(&text));
        let root = xml::parse(document.as_bytes()).unwrap();
        prop_assert_eq!(root.attribute("b"), Some(text.as_str()));
        prop_assert_eq!(root.text.as_str(), text.as_str());
    }
}

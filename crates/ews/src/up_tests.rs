// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Changes going up against the strict stand-in (#252, E6): each change of a message and of a
//! folder, drafts made, found by their marker and changed, messages sent and held when the answer
//! is lost, what a server lacks, and a busy server.

use std::cell::Cell;
use std::future::Future;
use std::sync::atomic::AtomicBool;

use mailcraft_model::sync::{
    AttachmentUp, AttachmentsUp, CategoriesUp, ChangeOutcome, DraftBody, DraftUp, EwsMailRequest, EwsMailResult, FlagUp, FolderChange, MessageChange,
    MessageOutcome, SendOutcome, SendUp, SyncedAttachment,
};
use mailcraft_model::{Address, FlagStatus, Importance, Message};

use crate::standin::{AUTHORIZATION, StandIn, block_on};
use crate::*;

const URL: &str = "https://mail.example.org/EWS/Exchange.asmx";

fn client() -> Client {
    Client::new(URL, Secret::new(AUTHORIZATION)).unwrap()
}

fn base() -> EwsMailRequest {
    EwsMailRequest { email: "ada@example.org".into(), url: URL.into(), inbox_pass: false, ..Default::default() }
}

fn run_with<E: HttpExchange>(exchange: &E, request: &EwsMailRequest) -> (Vec<EwsMailResult>, Vec<u32>) {
    let (mut pieces, mut waits) = (Vec::new(), Vec::new());
    let cancel = AtomicBool::new(false);
    block_on(sync_mail(
        &client(),
        exchange,
        request,
        &cancel,
        &mut |ms| {
            waits.push(ms);
            true
        },
        &mut |piece| pieces.push(piece),
    ))
    .unwrap();
    (pieces, waits)
}

fn run(standin: &StandIn, request: &EwsMailRequest) -> EwsMailResult {
    let (pieces, _) = run_with(standin, request);
    let mut all = EwsMailResult::default();
    for piece in pieces {
        all.absorb(piece);
    }
    all
}

/// Loses the answer to the first request of an operation that is never repeated: it is done, and
/// the program is told the answer went missing.
struct Losing<'a> {
    inner: &'a StandIn,
    operation: &'static str,
    lost: Cell<u32>,
}

impl HttpExchange for Losing<'_> {
    fn exchange(&self, request: &HttpRequest) -> impl Future<Output = Result<HttpResponse, TransportError>> {
        let done = block_on(self.inner.exchange(request)).unwrap();
        let body = String::from_utf8_lossy(request.body()).into_owned();
        let hit = body.contains(&format!("<m:{} ", self.operation)) || body.contains(&format!("<m:{}>", self.operation));
        let lose = hit && !request.is_repeatable() && self.lost.get() > 0;
        if lose {
            self.lost.set(self.lost.get() - 1);
        }
        std::future::ready(if lose { Err(TransportError::Lost) } else { done })
    }
}

fn mail(standin: &StandIn, subject: &str) -> String {
    standin.state().deliver("inbox", subject, false)
}


fn done(result: &EwsMailResult, at: usize) -> &MessageOutcome {
    &result.messages.get(at).unwrap().outcome
}

#[test]
fn read_flag_categories_and_importance_go_up_in_one_change() {
    let standin = StandIn::new();
    let id = mail(&standin, "Budget");
    standin.state().items[0].categories = vec!["Blue".into(), "Red".into()];
    let flag = FlagUp { status: FlagStatus::Flagged, start: Some(1_791_900_000), due: Some(1_792_000_000), completed: None };
    let change = MessageChange::Update {
        message: 1,
        id: id.clone(),
        read: Some(true),
        flag: Some(flag.clone()),
        // Taken away here: Red. Added here: Green. Added on the server meanwhile: Blue is kept (it was synced).
        categories: Some(CategoriesUp { synced: vec!["Red".into(), "Blue".into()], here: vec!["Blue".into(), "Green".into()] }),
        importance: Some(Importance::High),
    };
    let result = run(&standin, &EwsMailRequest { messages: vec![change], ..base() });
    let MessageOutcome::Done { categories, flag: went, .. } = done(&result, 0) else { panic!("{:?}", done(&result, 0)) };
    assert_eq!(categories.as_deref(), Some(&["Blue".to_string(), "Green".to_string()][..]));
    assert_eq!(went.as_ref(), Some(&flag));
    let state = standin.state();
    let item = state.item(&id).unwrap();
    assert!(item.read && item.flagged && item.importance == "High");
    assert_eq!(item.categories, ["Blue", "Green"]);
    assert!(state.deletes.is_empty());
}

#[test]
fn a_flag_cleared_and_a_message_marked_unread_go_up() {
    let standin = StandIn::new();
    let id = mail(&standin, "Budget");
    {
        let mut s = standin.state();
        s.items[0].read = true;
        s.items[0].flagged = true;
        s.items[0].flag = "Flagged".into();
    }
    let change = MessageChange::Update {
        message: 1,
        id: id.clone(),
        read: Some(false),
        flag: Some(FlagUp { status: FlagStatus::None, ..Default::default() }),
        categories: Some(CategoriesUp { synced: vec![], here: vec![] }),
        importance: None,
    };
    let result = run(&standin, &EwsMailRequest { messages: vec![change], ..base() });
    assert!(matches!(done(&result, 0), MessageOutcome::Done { .. }));
    let state = standin.state();
    assert!(!state.item(&id).unwrap().read && !state.item(&id).unwrap().flagged);
}

#[test]
fn a_server_without_a_flag_leaves_it_here_and_the_rest_goes_up() {
    let standin = StandIn::new();
    standin.state().lacks_flag = true;
    let (a, b) = (mail(&standin, "One"), mail(&standin, "Two"));
    let flag = FlagUp { status: FlagStatus::Flagged, start: Some(1), due: Some(2), completed: None };
    let changes = vec![
        MessageChange::Update { message: 1, id: a.clone(), read: Some(true), flag: Some(flag.clone()), categories: None, importance: None },
        MessageChange::Update { message: 2, id: b.clone(), read: None, flag: Some(flag.clone()), categories: None, importance: None },
    ];
    let result = run(&standin, &EwsMailRequest { messages: changes, ..base() });
    // Both are done as far as the server goes: the flag stays here, nothing is refused.
    assert!(matches!(done(&result, 0), MessageOutcome::Done { flag: Some(_), .. }));
    assert!(matches!(done(&result, 1), MessageOutcome::Done { flag: Some(_), .. }));
    let state = standin.state();
    assert!(state.item(&a).unwrap().read && !state.item(&a).unwrap().flagged);
    // (The second never asked: the first had told the run.)
    let updates = state.requests.iter().filter(|r| r.operation == "UpdateItem").count();
    assert_eq!(updates, 2, "one that failed and was made again without the flag, and nothing for the second");
}

#[test]
fn a_move_gives_the_message_its_new_id_and_a_copy_makes_one() {
    let standin = StandIn::new();
    let id = mail(&standin, "Budget");
    let archive = standin.state().add_folder("Archive");
    let changes = vec![
        MessageChange::Copy { copy: 7, id: id.clone(), to: archive.clone(), to_folder: None, marker: "m".into() },
        MessageChange::Move { message: 1, id: id.clone(), to: archive.clone(), to_folder: None },
    ];
    let result = run(&standin, &EwsMailRequest { messages: changes, ..base() });
    let MessageOutcome::Done { folder, id: copy, .. } = done(&result, 0) else { panic!() };
    assert_eq!((folder.as_str(), copy.is_empty()), (archive.as_str(), false));
    let MessageOutcome::Done { folder, id: moved, .. } = done(&result, 1) else { panic!() };
    assert!(*folder == archive && !moved.is_empty() && *moved != id);
    let state = standin.state();
    assert!(state.item(&id).is_none());
    assert_eq!(state.items.iter().filter(|i| i.folder == archive).count(), 2);
}

#[test]
fn a_delete_for_good_is_a_soft_delete_and_never_a_hard_one() {
    let standin = StandIn::new();
    let id = mail(&standin, "Budget");
    let result = run(&standin, &EwsMailRequest { messages: vec![MessageChange::Delete { id: id.clone() }], ..base() });
    assert!(matches!(done(&result, 0), MessageOutcome::Done { .. }));
    assert_eq!(standin.state().deletes, ["SoftDelete"]);
    assert!(standin.state().item(&id).is_none());
    // Gone already: nothing to do.
    let again = run(&standin, &EwsMailRequest { messages: vec![MessageChange::Delete { id }], ..base() });
    assert!(matches!(done(&again, 0), MessageOutcome::Gone));
}

#[test]
fn delete_item_to_the_deleted_items_folder_moves_it_there() {
    let standin = StandIn::new();
    let id = mail(&standin, "Budget");
    let op = changes::DeleteItem::new(&id, DeleteMode::MoveToDeletedItems);
    block_on(client().call(&standin, &op.element())).unwrap().unwrap();
    let state = standin.state();
    let deleted = state.folder_id("deleteditems").unwrap();
    assert!(state.items.iter().all(|i| i.folder == deleted) && state.items.len() == 1);
    assert_eq!(state.deletes, ["MoveToDeletedItems"]);
}

#[test]
fn a_change_to_a_message_the_server_lost_says_so() {
    let standin = StandIn::new();
    let result = run(&standin, &EwsMailRequest { messages: vec![MessageChange::Update { message: 1, id: "AAMkNoSuchItem".into(), read: Some(true), flag: None, categories: None, importance: None }, MessageChange::Move { message: 2, id: "x".into(), to: "y".into(), to_folder: None }], ..base() });
    // (An update with nothing set is an update with nothing to set: asked, and the item isn't there.)
    assert!(matches!(done(&result, 0), MessageOutcome::Gone | MessageOutcome::Refused { .. }));
}

#[test]
fn folders_are_made_renamed_moved_and_deleted_softly() {
    let standin = StandIn::new();
    let changes = vec![
        FolderChange::Create { folder: 10, name: "Projects".into(), parent: String::new(), parent_folder: None },
        FolderChange::Create { folder: 11, name: "Apollo".into(), parent: String::new(), parent_folder: Some(10) },
    ];
    let result = run(&standin, &EwsMailRequest { changes, ..base() });
    let ChangeOutcome::Done { folder: projects } = &result.changes[0].outcome else { panic!() };
    let ChangeOutcome::Done { folder: apollo } = &result.changes[1].outcome else { panic!() };
    assert_eq!((projects.name.as_str(), apollo.parent.as_str()), ("Projects", projects.id.as_str()));
    // A name taken is refused, in words.
    let again = run(&standin, &EwsMailRequest { changes: vec![FolderChange::Create { folder: 12, name: "projects".into(), parent: String::new(), parent_folder: None }], ..base() });
    assert!(matches!(&again.changes[0].outcome, ChangeOutcome::Refused { why } if why.contains("already")));
    let changes = vec![
        FolderChange::Rename { folder: 10, id: projects.id.clone(), name: "Work".into() },
        FolderChange::Move { folder: 11, id: apollo.id.clone(), parent: String::new() },
        FolderChange::Delete { id: apollo.id.clone() },
        FolderChange::Delete { id: apollo.id.clone() },
    ];
    let result = run(&standin, &EwsMailRequest { changes, ..base() });
    assert!(matches!(&result.changes[0].outcome, ChangeOutcome::Done { folder } if folder.name == "Work"));
    assert!(matches!(&result.changes[1].outcome, ChangeOutcome::Done { folder } if folder.parent.is_empty()));
    assert!(matches!(&result.changes[2].outcome, ChangeOutcome::Done { .. }));
    assert!(matches!(&result.changes[3].outcome, ChangeOutcome::Gone));
    let state = standin.state();
    assert_eq!(state.deletes, ["SoftDelete", "SoftDelete"]);
    assert!(state.folders.iter().any(|f| f.name == "Work") && state.folders.iter().all(|f| f.name != "Apollo"));
}

#[test]
fn a_special_folder_cannot_be_deleted() {
    let standin = StandIn::new();
    let inbox = standin.state().folder_id("inbox").unwrap();
    let result = run(&standin, &EwsMailRequest { changes: vec![FolderChange::Delete { id: inbox }], ..base() });
    assert!(matches!(&result.changes[0].outcome, ChangeOutcome::Refused { .. }));
}

fn draft(marker: &str, subject: &str) -> DraftUp {
    let file = AttachmentUp { name: "notes.txt".into(), mime: "text/plain".into(), hash: 11, size: 5, data: std::sync::Arc::new(b"hello".to_vec()), ..Default::default() };
    DraftUp {
        message: 5,
        marker: marker.into(),
        saves: 1,
        subject: Some(subject.into()),
        body: Some(DraftBody { html: false, content: "Dear Zoe".into(), hash: 3 }),
        to: Some(vec![Address::new("Zoe", "zoe@example.org")]),
        cc: Some(vec![]),
        bcc: Some(vec![Address::new("", "kit@example.org")]),
        importance: Some(Importance::Normal),
        attachments: Some(AttachmentsUp { here: vec![file], synced: vec![] }),
        ..Default::default()
    }
}

#[test]
fn a_draft_is_made_in_drafts_with_its_file_and_then_changed_in_place() {
    let standin = StandIn::new();
    let result = run(&standin, &EwsMailRequest { drafts: vec![draft("mark-1", "Hello")], ..base() });
    let made = &result.drafts[0];
    assert!(!made.id.is_empty() && made.fields && !made.change_key.is_empty());
    let files = made.attachments.as_ref().unwrap();
    assert_eq!((files.len(), files[0].name.as_str(), files[0].id.is_empty()), (1, "notes.txt", false));
    {
        let state = standin.state();
        let item = state.item(&made.id).unwrap();
        assert!(item.draft && item.folder == state.folder_id("drafts").unwrap());
        assert_eq!((item.subject.as_str(), item.marker.as_str(), item.bcc.len(), item.files.len()), ("Hello", "mark-1", 1, 1));
    }
    // Saved again: the subject changes, the file is taken off and another is added.
    let other = AttachmentUp { name: "plan.pdf".into(), mime: "application/pdf".into(), hash: 22, size: 3, data: std::sync::Arc::new(b"pdf".to_vec()), ..Default::default() };
    let again = DraftUp {
        id: made.id.clone(),
        change_key: made.change_key.clone(),
        subject: Some("Hello again".into()),
        body: None,
        to: None,
        cc: None,
        bcc: Some(vec![]),
        importance: Some(Importance::High),
        attachments: Some(AttachmentsUp { here: vec![other], synced: files.clone() }),
        ..draft("mark-1", "")
    };
    let result = run(&standin, &EwsMailRequest { drafts: vec![again], ..base() });
    let changed = &result.drafts[0];
    assert!(changed.fields && !changed.conflict && changed.refused.is_none());
    let files = changed.attachments.as_ref().unwrap();
    assert_eq!(files.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["plan.pdf"]);
    let state = standin.state();
    let item = state.item(&made.id).unwrap();
    assert_eq!((item.subject.as_str(), item.importance.as_str(), item.bcc.len()), ("Hello again", "High", 0));
    assert_eq!(item.files.iter().map(|f| f.1.as_str()).collect::<Vec<_>>(), ["plan.pdf"]);
    assert_eq!(state.items.iter().filter(|i| i.draft).count(), 1, "changed in place, not made again");
}

#[test]
fn a_draft_changed_elsewhere_is_not_written_over() {
    let standin = StandIn::new();
    let result = run(&standin, &EwsMailRequest { drafts: vec![draft("mark-2", "Hello")], ..base() });
    let made = &result.drafts[0];
    standin.state().edit_subject(&made.id, "Edited on the server");
    let again = DraftUp { id: made.id.clone(), change_key: made.change_key.clone(), subject: Some("Mine".into()), attachments: None, ..draft("mark-2", "") };
    let result = run(&standin, &EwsMailRequest { drafts: vec![again], ..base() });
    assert!(result.drafts[0].conflict);
    assert_eq!(standin.state().item(&made.id).unwrap().subject, "Edited on the server");
}

#[test]
fn a_draft_whose_make_was_lost_is_found_by_its_marker_and_never_made_twice() {
    let standin = StandIn::new();
    let losing = Losing { inner: &standin, operation: "CreateItem", lost: Cell::new(1) };
    let request = EwsMailRequest { drafts: vec![draft("mark-3", "Hello")], ..base() };
    let (pieces, _) = run_with(&losing, &request);
    // The server made it; the run has no answer for it.
    assert!(pieces.iter().all(|p| p.drafts.is_empty()));
    assert_eq!(standin.state().items.iter().filter(|i| i.draft).count(), 1);
    // The next run finds it by its marker and brings it up to date.
    let result = run(&standin, &request);
    assert!(!result.drafts[0].id.is_empty() && result.drafts[0].fields);
    assert_eq!(standin.state().items.iter().filter(|i| i.draft).count(), 1, "found, not made again");
}

fn message(to: &str, bcc: &str) -> Box<Message> {
    Box::new(Message {
        subject: "Quarterly".into(),
        from: Address::new("Ada", "ada@example.org"),
        to: vec![Address::new("Zoe", to)],
        bcc: if bcc.is_empty() { vec![] } else { vec![Address::new("", bcc)] },
        date: 1_791_993_600,
        message_id: "m1@example.org".into(),
        body: mailcraft_model::Body { text: "Numbers attached.".into(), ..Default::default() },
        ..Default::default()
    })
}

#[test]
fn a_message_never_saved_goes_as_mime_with_its_bcc_and_a_copy_is_filed_in_sent_items() {
    let standin = StandIn::new();
    let send = SendUp { message: 3, draft: None, mime: Some(message("zoe@example.org", "kit@example.org")) };
    let result = run(&standin, &EwsMailRequest { sends: vec![send], ..base() });
    assert_eq!(result.sent[0].outcome, SendOutcome::Sent);
    let state = standin.state();
    let out = String::from_utf8_lossy(&state.outgoing[0]).into_owned();
    assert!(out.contains("Subject: Quarterly") && out.to_lowercase().contains("bcc: kit@example.org"), "{out}");
    let sent = state.folder_id("sentitems").unwrap();
    assert!(state.items.iter().any(|i| i.folder == sent && i.subject == "Quarterly"));
}

#[test]
fn a_message_that_cannot_go_is_held_with_why() {
    let standin = StandIn::new();
    let sends = vec![
        SendUp { message: 1, draft: None, mime: Some(message("not an address", "")) },
        SendUp { message: 2, draft: None, mime: Some(Box::new(Message { to: vec![], ..*message("a@example.org", "") })) },
    ];
    let result = run(&standin, &EwsMailRequest { sends, ..base() });
    assert!(matches!(&result.sent[0].outcome, SendOutcome::Held { lost: false, why } if !why.is_empty()));
    assert!(matches!(&result.sent[1].outcome, SendOutcome::Held { lost: false, why } if why.contains("no recipients")));
    assert!(standin.state().outgoing.is_empty());
}

#[test]
fn a_draft_is_brought_up_to_date_and_sent_as_the_draft() {
    let standin = StandIn::new();
    let send = SendUp { message: 5, draft: Some(DraftUp { change_key: String::new(), ..draft("mark-4", "Hello") }), mime: None };
    let result = run(&standin, &EwsMailRequest { sends: vec![send], ..base() });
    assert_eq!(result.sent[0].outcome, SendOutcome::Sent);
    let made = result.sent[0].draft.as_ref().unwrap();
    assert!(!made.id.is_empty() && made.fields);
    let state = standin.state();
    assert!(state.outgoing[0].windows(5).any(|w| w == b"Hello"));
    assert!(state.outgoing.len() == 1 && state.items.iter().all(|i| !i.draft));
    // Filed in Sent Items.
    let sent = state.folder_id("sentitems").unwrap();
    assert!(state.items.iter().any(|i| i.folder == sent && i.subject == "Hello"));
}

#[test]
fn a_send_whose_answer_was_lost_is_held_and_never_sent_twice() {
    for (operation, send) in [
        ("CreateItem", SendUp { message: 3, draft: None, mime: Some(message("zoe@example.org", "")) }),
        ("SendItem", SendUp { message: 5, draft: Some(draft("mark-5", "Hello")), mime: None }),
    ] {
        let standin = StandIn::new();
        let losing = Losing { inner: &standin, operation, lost: Cell::new(1) };
        let (pieces, _) = run_with(&losing, &EwsMailRequest { sends: vec![send], ..base() });
        let held: Vec<_> = pieces.iter().flat_map(|p| &p.sent).collect();
        assert_eq!(held.len(), 1, "{operation}");
        assert_eq!(held[0].outcome, SendOutcome::Held { lost: true, why: String::new() }, "{operation}");
        // It went once; the run did not send it again.
        assert_eq!(standin.state().outgoing.len(), 1, "{operation}");
    }
}

#[test]
fn a_busy_server_is_waited_for_before_a_write_and_the_write_goes_once() {
    let standin = StandIn::new();
    let id = mail(&standin, "Budget");
    standin.state().busy = (1, 1000);
    let change = MessageChange::Update { message: 1, id: id.clone(), read: Some(true), flag: None, categories: None, importance: None };
    let (pieces, waits) = run_with(&standin, &EwsMailRequest { messages: vec![change], ..base() });
    assert_eq!(waits, [1000]);
    assert!(pieces.iter().any(|p| !p.messages.is_empty()));
    assert!(standin.state().item(&id).unwrap().read);
}

#[test]
fn the_inbox_pass_sends_and_changes_nothing() {
    let standin = StandIn::new();
    let id = mail(&standin, "Budget");
    let change = MessageChange::Update { message: 1, id: id.clone(), read: Some(true), flag: None, categories: None, importance: None };
    let result = run(&standin, &EwsMailRequest { messages: vec![change], inbox_pass: true, ..base() });
    assert!(result.messages.is_empty() && !standin.state().item(&id).unwrap().read);
}

#[test]
fn an_answer_that_changes_nothing_is_not_in_the_piece() {
    let standin = StandIn::new();
    // A change of a folder under a parent made in this run that was not made: it waits.
    let changes = vec![FolderChange::Create { folder: 3, name: "Orphan".into(), parent: String::new(), parent_folder: Some(99) }];
    let result = run(&standin, &EwsMailRequest { changes, ..base() });
    assert!(result.changes.is_empty());
    let _ = SyncedAttachment::default();
}

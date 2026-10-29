// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The request writer, the reader, the judgement of answers, and the client against the strict
//! stand-in.

use crate::standin::{AUTHORIZATION, BLOCKED_BY_POLICY, DISABLED, Hostile, StandIn, block_on};
use crate::*;

const URL: &str = "http://127.0.0.1:1/EWS/Exchange.asmx";

fn secret() -> Secret {
    Secret::new(AUTHORIZATION)
}

fn client() -> Client {
    Client::new(URL, secret()).unwrap()
}

fn check(standin: &StandIn) -> Result<FolderInfo, Error> {
    block_on(client().check(standin)).unwrap()
}

/// What the stand-in answers to a body written by hand.
fn raw(standin: &StandIn, body: &str) -> (u16, String) {
    let request = HttpRequest::soap(URL, &secret(), body.to_owned()).unwrap();
    let response = block_on(standin.exchange(&request)).unwrap().unwrap();
    (response.status(), String::from_utf8_lossy(response.body()).into_owned())
}

const HEAD: &str = "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\" xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:t=\"http://schemas.microsoft.com/exchange/services/2006/types\"><s:Header><t:RequestServerVersion Version=\"Exchange2013\"/></s:Header><s:Body>";
const TAIL: &str = "</s:Body></s:Envelope>";

fn hand(operation: &str) -> String {
    format!("{HEAD}{operation}{TAIL}")
}

const GOOD: &str = "<m:GetFolder><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape><m:FolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:FolderIds></m:GetFolder>";

// ---- the client against the stand-in ----

#[test]
fn the_inbox_can_be_had() {
    let standin = StandIn::new();
    let inbox = check(&standin).unwrap();
    assert_eq!(inbox.display_name, "Inbox");
    assert_eq!((inbox.total_count, inbox.unread_count, inbox.child_folder_count), (Some(12), Some(3), Some(0)));
    assert_eq!(inbox.change_key.as_deref(), Some("AQAAAA=="));
    let state = standin.state();
    assert_eq!(state.requests.len(), 1);
    assert_eq!((state.requests[0].operation.as_str(), state.requests[0].status), ("GetFolder", 200));
    assert!(state.writes.is_empty());
}

#[test]
fn several_folders_are_answered_in_order() {
    let standin = StandIn::new();
    let get = GetFolder::new(&[DistinguishedFolder::Drafts, DistinguishedFolder::SentItems, DistinguishedFolder::Inbox]);
    let all = block_on(client().get_folders(&standin, &get)).unwrap().unwrap();
    let names: Vec<_> = all.into_iter().map(|one| one.unwrap().display_name).collect();
    assert_eq!(names, ["Drafts", "Sent Items", "Inbox"]);
}

#[test]
fn a_folder_that_is_not_there_is_its_own_error() {
    let standin = StandIn::new();
    standin.state().folders.retain(|folder| folder.distinguished != "junkemail");
    let get = GetFolder::new(&[DistinguishedFolder::JunkEmail, DistinguishedFolder::Inbox]);
    let all = block_on(client().get_folders(&standin, &get)).unwrap().unwrap();
    assert_eq!(all[0], Err(Error::Server(ResponseCode::FolderNotFound)));
    assert!(all[1].is_ok());
}

#[test]
fn a_wrong_credential_is_unauthorized() {
    let standin = StandIn::new();
    let client = Client::new(URL, Secret::new("Basic bm9wZTpub3Bl")).unwrap();
    assert_eq!(block_on(client.check(&standin)).unwrap(), Err(Error::Unauthorized));
}

#[test]
fn an_organisation_that_shuts_the_service_is_told_apart() {
    let standin = StandIn::new();
    standin.state().blocked = Some(DISABLED.into());
    assert_eq!(check(&standin), Err(Error::Blocked(PolicyReason::Disabled)));
    standin.state().blocked = Some(BLOCKED_BY_POLICY.into());
    assert_eq!(check(&standin), Err(Error::Blocked(PolicyReason::Blocked)));
    standin.state().blocked = Some("something else".into());
    assert_eq!(check(&standin), Err(Error::Blocked(PolicyReason::Other)));
}

#[test]
fn a_busy_server_says_how_long_and_is_believed_only_so_far() {
    let standin = StandIn::new();
    standin.state().busy = (1, 2_500);
    assert_eq!(check(&standin), Err(Error::Busy { back_off_ms: 2_500 }));
    assert!(check(&standin).is_ok());
    standin.state().busy = (1, u64::MAX);
    assert_eq!(check(&standin), Err(Error::Busy { back_off_ms: error::MAX_BACK_OFF_MS }));
}

#[test]
fn a_busy_server_that_names_no_wait_gets_the_default() {
    let body = reply_with("ErrorServerBusy", "");
    assert_eq!(parse_folder_error(&body), Error::Busy { back_off_ms: error::DEFAULT_BACK_OFF_MS });
    let body = reply_with("ErrorServerBusy", "<m:BackOffMilliseconds>soon</m:BackOffMilliseconds>");
    assert_eq!(parse_folder_error(&body), Error::Busy { back_off_ms: error::DEFAULT_BACK_OFF_MS });
    let body = reply_with("ErrorServerBusy", "<m:BackOffMilliseconds>7000</m:BackOffMilliseconds>");
    assert_eq!(parse_folder_error(&body), Error::Busy { back_off_ms: 7_000 });
}

fn reply_with(code: &str, extra: &str) -> String {
    format!(
        "<s:Envelope xmlns:s=\"http://schemas.xmlsoap.org/soap/envelope/\"><s:Body><m:GetFolderResponse xmlns:m=\"http://schemas.microsoft.com/exchange/services/2006/messages\">\
         <m:ResponseMessages><m:GetFolderResponseMessage ResponseClass=\"Error\"><m:ResponseCode>{code}</m:ResponseCode>{extra}</m:GetFolderResponseMessage></m:ResponseMessages>\
         </m:GetFolderResponse></s:Body></s:Envelope>"
    )
}

fn parse_folder_error(body: &str) -> Error {
    let reply = Reply::parse(body.as_bytes()).unwrap();
    GetFolder::new(&[DistinguishedFolder::Inbox]).answer(&reply).unwrap().remove(0).unwrap_err()
}

#[test]
fn http_statuses_are_judged() {
    let judge = |response: HttpResponse| Reply::judge(&response, 1_000).unwrap_err();
    assert_eq!(judge(HttpResponse::new(401, vec![])), Error::Unauthorized);
    assert_eq!(judge(HttpResponse::new(403, vec![])), Error::Forbidden);
    assert_eq!(judge(HttpResponse::new(503, vec![]).with_field("retry-after", "3")), Error::Busy { back_off_ms: 3_000 });
    assert_eq!(judge(HttpResponse::new(429, vec![]).with_field("Retry-After", "99999")), Error::Busy { back_off_ms: error::MAX_BACK_OFF_MS });
    assert_eq!(
        judge(HttpResponse::new(429, vec![]).with_field("Retry-After", "Wed, 21 Oct 2026 07:28:00 GMT")),
        Error::Busy { back_off_ms: error::DEFAULT_BACK_OFF_MS }
    );
    assert_eq!(judge(HttpResponse::new(404, vec![])), Error::Status(404));
    assert_eq!(judge(HttpResponse::new(500, b"<html/>".to_vec())), Error::Status(500));
    assert_eq!(judge(HttpResponse::new(200, vec![b'x'; 2_000])), Error::TooLarge);
    assert_eq!(judge(HttpResponse::new(200, b"<html><body>Sign in</body></html>".to_vec())), Error::Unexpected);
    assert_eq!(judge(HttpResponse::new(200, b"not xml at all".to_vec())), Error::Malformed);
}

#[test]
fn a_fault_is_the_error_it_names() {
    let standin = StandIn::new();
    let (status, body) = raw(&standin, &hand("<m:GetFolder/>"));
    assert_eq!(status, 500);
    assert_eq!(Reply::parse(body.as_bytes()).unwrap_err(), Error::Server(ResponseCode::SchemaValidation));
    let (_, body) = raw(&standin, &hand("<m:NoSuchThing/>"));
    assert_eq!(Reply::parse(body.as_bytes()).unwrap_err(), Error::Server(ResponseCode::InvalidRequest));
}

#[test]
fn prefixes_are_the_documents_own_choice() {
    let body = "<a:Envelope xmlns:a=\"http://schemas.xmlsoap.org/soap/envelope/\"><a:Body>\
        <x:GetFolderResponse xmlns:x=\"http://schemas.microsoft.com/exchange/services/2006/messages\" xmlns:y=\"http://schemas.microsoft.com/exchange/services/2006/types\"><x:ResponseMessages>\
        <x:GetFolderResponseMessage ResponseClass=\"Success\"><x:ResponseCode>NoError</x:ResponseCode><x:Folders><y:Folder><y:FolderId Id=\"AbC+/=-_\"/><y:DisplayName>Inbox</y:DisplayName></y:Folder></x:Folders>\
        </x:GetFolderResponseMessage></x:ResponseMessages></x:GetFolderResponse></a:Body></a:Envelope>";
    let reply = Reply::parse(body.as_bytes()).unwrap();
    let got = GetFolder::new(&[DistinguishedFolder::Inbox]).answer(&reply).unwrap().remove(0).unwrap();
    assert_eq!(got.display_name, "Inbox");
    assert_eq!(got.total_count, None);
}

#[test]
fn an_element_in_the_wrong_namespace_is_not_the_one_asked_for() {
    let body = reply_with("NoError", "").replace("services/2006/messages", "services/2006/elsewhere");
    let reply = Reply::parse(body.as_bytes()).unwrap();
    assert_eq!(GetFolder::new(&[DistinguishedFolder::Inbox]).answer(&reply).unwrap_err(), Error::Unexpected);
}

#[test]
fn unknown_elements_are_skipped() {
    let extra = "<m:Surprise><m:Deeper>1</m:Deeper></m:Surprise><x:Other xmlns:x=\"urn:other\"/>";
    let body = reply_with("ErrorAccessDenied", extra);
    assert_eq!(parse_folder_error(&body), Error::Server(ResponseCode::AccessDenied));
}

#[test]
fn response_codes_are_kept_only_as_short_identifiers() {
    assert_eq!(ResponseCode::from_name(" ErrorServerBusy "), ResponseCode::ServerBusy);
    assert_eq!(ResponseCode::from_name("ErrorSomethingNew"), ResponseCode::Other("ErrorSomethingNew".into()));
    assert_eq!(ResponseCode::from_name("bad code\r\nX: y"), ResponseCode::Other(String::new()));
    assert_eq!(ResponseCode::from_name(&"A".repeat(500)), ResponseCode::Other(String::new()));
    assert_eq!(ResponseCode::from_name("bad\r\nX: y").to_string(), "an unnamed error");
}

#[test]
fn a_folder_id_that_is_not_an_id_is_not_taken() {
    let standin = StandIn::new();
    standin.state().hostile = Some(Hostile::BadId);
    assert_eq!(check(&standin), Err(Error::Unexpected));
    assert!(FolderId::new("AAMk+Tk/Zm9=").is_some());
    assert!(FolderId::new("").is_none());
    assert!(FolderId::new("a b").is_none());
    assert!(FolderId::new(&"a".repeat(2_000)).is_none());
}

#[test]
fn hostile_answers_are_errors_never_panics() {
    for (how, expected) in [
        (Hostile::Dtd, Error::Malformed),
        (Hostile::CutShort, Error::Malformed),
        (Hostile::NotXml, Error::Malformed),
        (Hostile::Deep, Error::Malformed),
        (Hostile::Huge, Error::TooLarge),
        (Hostile::Fault, Error::Server(ResponseCode::Other(String::new()))),
        (Hostile::TooFew, Error::Unexpected),
    ] {
        let standin = StandIn::new();
        standin.state().hostile = Some(how);
        assert_eq!(check(&standin), Err(expected), "{how:?}");
    }
}

// ---- the request ----

#[test]
fn the_request_is_a_soap_envelope_with_the_version_and_one_operation() {
    let get = GetFolder::new(&[DistinguishedFolder::Inbox]);
    let text = soap::envelope(Version::Exchange2016, &get.element());
    let tree = xml::parse(text.as_bytes()).unwrap();
    assert!(tree.is(soap::SOAP, "Envelope"));
    let version = tree.child_in(soap::SOAP, "Header").unwrap().child_in(soap::TYPES, "RequestServerVersion").unwrap();
    assert_eq!(version.attribute("Version"), Some("Exchange2016"));
    let body = tree.child_in(soap::SOAP, "Body").unwrap();
    assert_eq!(body.children.len(), 1);
    assert!(body.children[0].is(soap::MESSAGES, "GetFolder"));
    let request = client().with_version(Version::Exchange2010Sp2).request(&get.element()).unwrap();
    assert_eq!(request.method(), "POST");
    assert!(String::from_utf8_lossy(request.body()).contains("Exchange2010_SP2"));
}

#[test]
fn nothing_written_can_break_out_of_its_place() {
    let nasty = "a\r\nX-Injected: 1\0</t:BaseShape><m:Evil/>\"'&";
    let element = Element::new("m:Op\r\n<x").attr("Na\nme", nasty).text(nasty).child(Element::new("t:Child").text(nasty));
    let text = element.write();
    assert!(!text.contains(['\r', '\n', '\0']), "{text:?}");
    assert!(!text.contains("<m:Evil/>") && !text.contains("<x"));
    // What is written reads back as the one element, holding the text as text.
    let tree = xml::parse(soap::envelope(Version::default(), &element).as_bytes()).unwrap();
    let op = &tree.child_in(soap::SOAP, "Body").unwrap().children[0];
    assert_eq!(op.name, "Opx");
    assert!(op.text.contains("</t:BaseShape><m:Evil/>"));
    assert_eq!(op.children.len(), 1);
    let secret = Secret::new("Basic abc\r\nX-Injected: 1");
    assert_eq!(secret.expose(), "Basic abcX-Injected: 1");
    assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
}

#[test]
fn credentials_and_ids_are_not_in_debug_output() {
    let request = client().request(&GetFolder::new(&[DistinguishedFolder::Inbox]).element()).unwrap();
    let shown = format!("{request:?} {:?} {:?}", client(), FolderId::new("AAMkSecretId").unwrap());
    assert!(!shown.contains("standin.ews") && !shown.contains("SecretId") && !shown.contains("127.0.0.1"), "{shown}");
}

#[test]
fn the_transport_rule() {
    for ok in [
        "https://mail.example.org/EWS/Exchange.asmx",
        "HTTPS://mail.example.org",
        "http://127.0.0.1:8080/x",
        "http://localhost/x",
        "http://[::1]:80/x",
        "http://127.9.9.9/",
    ] {
        assert_eq!(http::check_endpoint(ok), Ok(()), "{ok}");
    }
    for (bad, why) in [
        ("http://mail.example.org/EWS", EndpointError::NotEncrypted),
        ("http://127.0.0.1.example.org/", EndpointError::NotEncrypted),
        ("http://localhost.example.org/", EndpointError::NotEncrypted),
        ("https://user:pw@mail.example.org/", EndpointError::Credentials),
        ("http://localhost@evil.example/", EndpointError::Credentials),
        ("ftp://mail.example.org/", EndpointError::Malformed),
        ("https://", EndpointError::Malformed),
        ("https://mail.example.org/a b", EndpointError::Malformed),
        ("https://mail.example.org/\r\nX: y", EndpointError::Malformed),
        ("mail.example.org", EndpointError::Malformed),
    ] {
        assert_eq!(http::check_endpoint(bad), Err(why), "{bad}");
    }
    assert_eq!(http::check_endpoint(&format!("https://a.example/{}", "a".repeat(3_000))), Err(EndpointError::Malformed));
}

// ---- the stand-in holds a request to the schema ----

#[test]
fn the_stand_in_takes_a_request_that_fits() {
    let (status, body) = raw(&StandIn::new(), &hand(GOOD));
    assert_eq!(status, 200, "{body}");
}

#[test]
fn the_stand_in_refuses_what_does_not_fit() {
    let cases = [
        ("an unknown element", GOOD.replace("<m:FolderShape>", "<m:Extra/><m:FolderShape>")),
        ("a wrong namespace", GOOD.replace("<t:BaseShape>Default</t:BaseShape>", "<m:BaseShape>Default</m:BaseShape>")),
        ("a missing FolderIds", "<m:GetFolder><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape></m:GetFolder>".into()),
        ("a missing BaseShape", "<m:GetFolder><m:FolderShape/><m:FolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:FolderIds></m:GetFolder>".into()),
        ("a wrong order", "<m:GetFolder><m:FolderIds><t:DistinguishedFolderId Id=\"inbox\"/></m:FolderIds><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape></m:GetFolder>".into()),
        ("an unknown attribute", GOOD.replace("Id=\"inbox\"", "Id=\"inbox\" Odd=\"1\"")),
        ("a missing Id", GOOD.replace("Id=\"inbox\"", "")),
        ("an unknown folder name", GOOD.replace("inbox", "archive")),
        ("an unknown shape", GOOD.replace("Default", "Everything")),
        ("no folders", "<m:GetFolder><m:FolderShape><t:BaseShape>Default</t:BaseShape></m:FolderShape><m:FolderIds/></m:GetFolder>".into()),
        ("two operations", format!("{GOOD}{GOOD}")),
    ];
    for (what, operation) in cases {
        let (status, body) = raw(&StandIn::new(), &hand(&operation));
        assert_eq!(status, 500, "{what}: {body}");
        assert!(body.contains("ErrorSchemaValidation") || body.contains("ErrorInvalidRequest"), "{what}: {body}");
    }
}

#[test]
fn the_stand_in_refuses_a_bad_envelope() {
    let standin = StandIn::new();
    for (what, text) in [
        ("no version", hand(GOOD).replace("<t:RequestServerVersion Version=\"Exchange2013\"/>", "")),
        ("a version the schema lacks", hand(GOOD).replace("Exchange2013", "Exchange2099")),
        ("the version in the wrong namespace", hand(GOOD).replace("<t:RequestServerVersion", "<m:RequestServerVersion")),
        ("a wrong envelope namespace", hand(GOOD).replace("http://schemas.xmlsoap.org/soap/envelope/", "urn:other")),
        ("not xml", "hello".to_owned()),
        ("a DTD", format!("<!DOCTYPE s [<!ENTITY a \"b\">]>{}", hand(GOOD))),
    ] {
        let (status, body) = raw(&standin, &text);
        assert_eq!(status, 500, "{what}: {body}");
    }
}

#[test]
fn the_stand_in_notes_and_refuses_anything_that_would_change_the_mailbox() {
    let standin = StandIn::new();
    let (status, body) = raw(&standin, &hand("<m:DeleteItem DeleteType=\"HardDelete\"/>"));
    assert_eq!(status, 200);
    assert!(body.contains("ErrorAccessDenied"));
    assert_eq!(standin.state().writes, ["DeleteItem"]);
}

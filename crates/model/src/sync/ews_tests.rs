// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;
use crate::FolderKind;
use crate::sync::{FolderChange, GraphFolder};

const ME: &str = "ada@example.org";
const URL: &str = "https://secret-host.example.org/EWS/Exchange.asmx";

fn round_trip<T: Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug>(value: &T) -> String {
    let json = serde_json::to_string(value).unwrap();
    assert_eq!(&serde_json::from_str::<T>(&json).unwrap(), value, "{json}");
    json
}

fn request() -> EwsMailRequest {
    EwsMailRequest {
        account: 3,
        email: ME.into(),
        url: URL.into(),
        username: "CORP\\ada".into(),
        trusted: "ab".repeat(32),
        list_folders: true,
        hierarchy: "H-STATE".into(),
        scope: EwsScope::Light { also: vec!["F-ONE".into()] },
        states: vec![EwsFolderState { folder: "F-ONE".into(), state: "S-ONE".into() }],
        changes: vec![FolderChange::Delete { id: "F-GONE".into() }],
        inbox_pass: true,
        full: true,
        background: true,
        ..Default::default()
    }
}

#[test]
fn a_request_round_trips_and_an_empty_one_is_small() {
    round_trip(&request());
    round_trip(&EwsFetchRequest { account: 3, email: ME.into(), url: URL.into(), ids: vec!["A".into(), "B".into()], ..Default::default() });
    let json = round_trip(&EwsMailRequest::default());
    assert!(!json.contains("trusted") && !json.contains("states") && !json.contains("sends"), "{json}");
}

#[test]
fn a_result_round_trips_and_pieces_are_absorbed_in_order() {
    let inbox = GraphFolder { id: "F1".into(), name: "Inbox".into(), kind: FolderKind::Inbox, ..Default::default() };
    let first = EwsMailResult {
        account: 3,
        email: ME.into(),
        url: URL.into(),
        listing: Some(FolderListing { complete: true, folders: vec![inbox] }),
        hierarchy: "H1".into(),
        states: vec![EwsFolderState { folder: "F1".into(), state: "S1".into() }],
        errors: vec!["one".into()],
        partial: true,
        ..Default::default()
    };
    let last =
        EwsMailResult { account: 3, states: vec![EwsFolderState { folder: "F1".into(), state: "S2".into() }], whole: true, ..Default::default() };
    round_trip(&first);
    let mut all = EwsMailResult::default();
    all.absorb(first);
    all.absorb(last);
    assert_eq!((all.account, all.email.as_str(), all.hierarchy.as_str()), (3, ME, "H1"));
    assert_eq!(all.states.iter().map(|s| s.state.as_str()).collect::<Vec<_>>(), ["S1", "S2"]);
    assert_eq!(all.errors.len(), 1);
    assert!(all.whole && !all.partial && all.listing.is_some());
}

#[test]
fn a_missing_or_unknown_field_still_reads() {
    let result: EwsMailResult = serde_json::from_str("{}").unwrap();
    assert_eq!(result, EwsMailResult::default());
    let result: EwsMailResult = serde_json::from_str(r#"{"account": 2, "future": [1, 2]}"#).unwrap();
    assert_eq!(result.account, 2);
}

#[test]
fn debug_output_is_counts_only() {
    let fetch = EwsFetchRequest { email: ME.into(), url: URL.into(), ids: vec!["SECRET-ID".into()], ..Default::default() };
    let text = format!("{:?} {fetch:?}", request());
    for secret in [ME, "secret-host", "CORP", "ababab", "H-STATE", "S-ONE", "F-ONE", "SECRET-ID"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    let result = EwsMailResult {
        email: ME.into(),
        url: URL.into(),
        states: vec![EwsFolderState { folder: "F-ONE".into(), state: "S-ONE".into() }],
        hierarchy: "H-STATE".into(),
        untrusted: "ab".repeat(32),
        ..Default::default()
    };
    let text = format!("{result:?}");
    for secret in [ME, "secret-host", "S-ONE", "F-ONE", "H-STATE", "ababab"] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
}

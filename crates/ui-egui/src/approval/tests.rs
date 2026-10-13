//! The words of the approval help: what a host's answer is taken for, and what the message for
//! an administrator says. (The pages and the commands are tried in the window, with Add
//! Account: `account_setup`'s tests.)

use super::*;

#[test]
fn a_hosts_answer_is_taken_for_what_it_says_exactly() {
    for (error, end, code) in [
        (NEEDS_APPROVAL, SignInEnd::Approval, "approval"),
        (SIGN_IN_DECLINED, SignInEnd::Declined, "declined"),
        (SIGN_IN_TIMED_OUT, SignInEnd::TimedOut, "timedOut"),
        (NO_BROWSER, SignInEnd::NoBrowser, "noBrowser"),
        ("AADSTS53003: Access has been blocked by Conditional Access policies.", SignInEnd::Other, "failed"),
        ("", SignInEnd::Other, "failed"),
    ] {
        assert_eq!((SignInEnd::of(error), SignInEnd::of(error).code()), (end, code), "{error}");
    }
    // Nothing that only resembles one of them, and nothing that only contains one.
    let near = format!("{NEEDS_APPROVAL}.");
    for other in [near.as_str(), "your organisation's administrator has to approve this once", " Nothing came back from Microsoft's sign-in page"] {
        assert_eq!(SignInEnd::of(other), SignInEnd::Other, "{other}");
    }
    // The same words the Favorites know an approval by: one refusal, said once.
    assert_eq!(NEEDS_APPROVAL, crate::favorites_sync::NEEDS_APPROVAL);
}

#[test]
fn the_message_for_an_administrator_says_what_it_is_for_and_nothing_of_the_owner() {
    let link = "https://login.example.org/lumen.example/v2.0/adminconsent?client_id=11111111-2222-3333-4444-555555555555";
    let text = message(link);
    // What the program is, in a sentence; that the approval is asked once, for the organisation;
    // the address; and what the administrator sees afterwards, with why that is as it should be.
    for said in [
        "MailCraft is an open-source mail and calendar program",
        "only an administrator",
        "approval once, for the whole organisation",
        link,
        "a page that can't be reached",
        "That is expected",
        "nothing is listening",
    ] {
        assert!(text.contains(said), "{said}: {text}");
    }
    // The address stands on a line of its own, so that whatever it is pasted into makes a link of it.
    assert!(text.lines().any(|line| line == link));
    // Plain text with the line ends of every system's clipboard, and short enough to read.
    assert!(!text.contains('\r') && !text.contains('{') && text.chars().count() < 900, "{text}");
    assert!(!text.contains('@'));
    // Whatever stands where the address goes is put there as it is, and nothing else changes.
    assert_eq!(message("").replace("\n\n\n\n", "\n\n{link}\n\n"), MESSAGE);
    assert!(message(&"x".repeat(5_000)).len() > 5_000);
}

#[test]
fn an_approval_that_was_not_had_says_why_in_the_windows_words_and_microsofts() {
    assert_eq!(AdminConsentAnswer::Approved.said(), (String::new(), String::new()));
    assert_eq!(AdminConsentAnswer::Cancelled.said(), (String::new(), String::new()));
    let theirs = "AADSTS90094: Administrator consent is required.";
    let (ours, words) = AdminConsentAnswer::NotAdministrator(theirs.into()).said();
    assert!(ours.contains("not an administrator") && ours.contains("Copy the message") && words == theirs);
    let (ours, words) = AdminConsentAnswer::Declined("AADSTS65004: Declined.".into()).said();
    assert!(ours.contains("without the approval") && words == "AADSTS65004: Declined.");
    for (answer, said) in [(AdminConsentAnswer::TimedOut, "five minutes"), (AdminConsentAnswer::NoBrowser, "couldn't open your browser")] {
        let (ours, words) = answer.said();
        assert!(ours.contains(said) && words.is_empty(), "{ours}");
    }
    // What comes from outside is one line of a length a page can show.
    let (_, words) = AdminConsentAnswer::Failed(format!("line one\r\nline two\u{0}{}", "x".repeat(10_000))).said();
    assert!(!words.contains('\n') && !words.contains('\u{0}') && words.chars().count() <= 301, "{}", words.chars().count());
    let codes: Vec<&str> = [
        AdminConsentAnswer::Approved,
        AdminConsentAnswer::NotAdministrator(String::new()),
        AdminConsentAnswer::Declined(String::new()),
        AdminConsentAnswer::TimedOut,
        AdminConsentAnswer::NoBrowser,
        AdminConsentAnswer::Cancelled,
        AdminConsentAnswer::Failed(String::new()),
    ]
    .iter()
    .map(AdminConsentAnswer::code)
    .collect();
    assert_eq!(codes, ["approved", "notAdministrator", "declined", "timedOut", "noBrowser", "cancelled", "failed"]);
}

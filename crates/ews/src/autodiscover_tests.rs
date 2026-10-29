// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Autodiscover state machine against the strict stand-in: each step, the redirects and the
//! owner's say, Exchange Online, and answers that are hostile.

use proptest::prelude::*;

use super::*;
use crate::standin::block_on;
use crate::standin_autodiscover::{AutodiscoverStandIn, BASIC, Route, error_answer, redirect_addr, redirect_url, settings};

const INTERNAL: &str = "https://exch01.corp.example.org/EWS/Exchange.asmx";
const EXTERNAL: &str = "https://mail.example.org/EWS/Exchange.asmx";

fn good() -> Route {
    Route::Document(settings(INTERNAL, EXTERNAL, "exch01.corp.example.org"))
}

fn search(email: &str) -> Autodiscover {
    Autodiscover::new(email, Secret::new(BASIC)).unwrap()
}

/// A whole search: no SRV record, and the owner's answers given in order (then no).
struct Run {
    outcome: Outcome,
    asked: Vec<(String, ConsentReason)>,
    srv: Vec<String>,
}

fn run_with(standin: &AutodiscoverStandIn, mut search: Autodiscover, srv: Option<Vec<&str>>, yes: &[bool]) -> Run {
    let (mut asked, mut looked, mut at) = (Vec::new(), Vec::new(), 0usize);
    let srv = srv.map(|hosts| hosts.into_iter().map(str::to_owned).collect::<Vec<_>>());
    let outcome = block_on(search.run(
        standin,
        |name| {
            looked.push(name.to_owned());
            srv.clone()
        },
        |consent| {
            asked.push((consent.host().to_owned(), consent.reason()));
            at += 1;
            yes.get(at - 1).copied().unwrap_or(false)
        },
    ))
    .unwrap();
    Run { outcome, asked, srv: looked }
}

fn go(standin: &AutodiscoverStandIn, srv: Option<Vec<&str>>, yes: &[bool]) -> Run {
    run_with(standin, search("ada@example.org"), srv, yes)
}

fn found(run: &Run) -> &Discovered {
    let Outcome::Found(found) = &run.outcome else { panic!("not found: {:?}", run.outcome) };
    found
}

/// Every sign-in went to the owner's domain or to a host in `also`.
fn only_to(standin: &AutodiscoverStandIn, also: &[&str]) {
    for seen in &standin.state().seen {
        assert!(seen.authorized, "{}", seen.host);
        assert!(is_within(&seen.host, "example.org") || also.contains(&seen.host.as_str()), "the sign-in went to {}", seen.host);
    }
}

// ---- the three places, in order ----

#[test]
fn the_domain_itself_is_asked_first() {
    let standin = AutodiscoverStandIn::new().route("example.org", good());
    let run = go(&standin, None, &[]);
    let found = found(&run);
    assert_eq!(found.urls(), [EXTERNAL, INTERNAL]);
    assert_eq!((found.external_url(), found.internal_url(), found.server()), (Some(EXTERNAL), Some(INTERNAL), Some("exch01.corp.example.org")));
    assert_eq!(standin.hosts_asked(), ["example.org"]);
    let state = standin.state();
    assert_eq!((state.seen[0].url.as_str(), state.seen[0].email.as_str()), ("https://example.org/autodiscover/autodiscover.xml", "ada@example.org"));
    assert!(run.asked.is_empty() && run.srv.is_empty());
}

#[test]
fn the_autodiscover_host_is_asked_second() {
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Status(404)).route("autodiscover.example.org", good());
    let run = go(&standin, None, &[]);
    assert_eq!(found(&run).external_url(), Some(EXTERNAL));
    assert_eq!(standin.hosts_asked(), ["example.org", "autodiscover.example.org"]);
    assert!(run.srv.is_empty());
    only_to(&standin, &[]);
}

#[test]
fn a_host_that_is_down_or_has_no_certificate_goes_on_to_the_next() {
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Unreachable).route("autodiscover.example.org", Route::TlsFails);
    let run = go(&standin, Some(vec!["ad.example.org"]), &[]);
    // (Nothing at ad.example.org either: the stand-in has no route for it.)
    assert_eq!(run.outcome, Outcome::Failed(Failure::SecureConnection));
    assert_eq!(standin.hosts_asked(), ["example.org", "autodiscover.example.org", "ad.example.org"]);
}

#[test]
fn the_srv_record_is_third_and_a_host_inside_the_domain_needs_no_question() {
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Status(404)).route("autodiscover.example.org", Route::Unreachable).route("ad.example.org", good());
    let run = go(&standin, Some(vec!["AD.example.org."]), &[]);
    found(&run);
    assert_eq!(run.srv, ["_autodiscover._tcp.example.org"]);
    assert!(run.asked.is_empty());
    assert_eq!(standin.hosts_asked(), ["example.org", "autodiscover.example.org", "ad.example.org"]);
}

#[test]
fn the_host_an_srv_record_names_outside_the_domain_is_asked_about_first() {
    let make = || AutodiscoverStandIn::new().route("hosted.example.net", good());
    let standin = make();
    let run = go(&standin, Some(vec!["hosted.example.net"]), &[true]);
    found(&run);
    assert_eq!(run.asked, [("hosted.example.net".to_owned(), ConsentReason::Redirect)]);
    only_to(&standin, &["hosted.example.net"]);

    // Declined: no sign-in goes there, and the search ends with that said.
    let standin = make();
    let run = run_with(&standin, search("ada@example.org"), Some(vec!["hosted.example.net"]), &[false]);
    assert_eq!(run.outcome, Outcome::Failed(Failure::Declined));
    assert!(standin.hosts_asked().iter().all(|host| host != "hosted.example.net"), "{:?}", standin.hosts_asked());
}

#[test]
fn what_an_srv_lookup_gives_is_checked() {
    let standin = AutodiscoverStandIn::new();
    let hostile = vec!["", ".", "localhost", "a b.example.org", "ex@mple.org", "evil.example.org/path", "https://evil.example.org", "-x.example.org", "x_y.example.org", "e\r\nvil.example.org"];
    let run = go(&standin, Some(hostile), &[]);
    assert_eq!(run.outcome, Outcome::Failed(Failure::Unreachable));
    assert_eq!(standin.hosts_asked(), ["example.org", "autodiscover.example.org"]);

    // At most three, each once.
    let standin = AutodiscoverStandIn::new();
    let many = vec!["a.example.org", "A.example.org.", "b.example.org", "c.example.org", "d.example.org", "e.example.org"];
    run_with(&standin, search("ada@example.org"), Some(many), &[]);
    assert_eq!(standin.hosts_asked(), ["example.org", "autodiscover.example.org", "a.example.org", "b.example.org", "c.example.org"]);
    // A lookup that failed is the same as one that found nothing.
    let standin = AutodiscoverStandIn::new();
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::Failed(Failure::Unreachable));
}

// ---- redirects ----

fn hop(from: &str, to: &str) -> (String, Route) {
    (from.to_owned(), Route::Redirect(302, format!("https://{to}/autodiscover/autodiscover.xml")))
}

fn chain(hosts: &[&str], settled: Route) -> AutodiscoverStandIn {
    let mut standin = AutodiscoverStandIn::new();
    for pair in hosts.windows(2) {
        let (from, route) = hop(pair[0], pair[1]);
        standin = standin.route(&from, route);
    }
    standin.route(hosts[hosts.len() - 1], settled)
}

#[test]
fn a_redirect_inside_the_domain_is_followed_without_asking() {
    let standin = chain(&["example.org", "mail.example.org"], good());
    let run = go(&standin, None, &[]);
    found(&run);
    assert!(run.asked.is_empty());
    assert_eq!(standin.hosts_asked(), ["example.org", "mail.example.org"]);
    only_to(&standin, &[]);
    for status in [301, 302, 303, 307, 308] {
        let standin = AutodiscoverStandIn::new()
            .route("example.org", Route::Redirect(status, "/autodiscover/autodiscover.xml".into()))
            .route("autodiscover.example.org", good());
        // (A path on the same host is the same host again: a loop, ended by the cap.)
        let run = run_with(&standin, search("ada@example.org"), None, &[]);
        assert_eq!(run.outcome, Outcome::Failed(Failure::TooManyRedirects), "{status}");
    }
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Redirect(301, "https://example.org:8443/autodiscover/autodiscover.xml".into()));
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::Failed(Failure::TooManyRedirects));
}

#[test]
fn a_redirect_outside_the_domain_stops_and_asks() {
    let standin = chain(&["example.org", "login.example.net"], good());
    let mut machine = search("ada@example.org");
    let Step::Request(request) = machine.step() else { panic!("no request") };
    machine.response(block_on(standin.exchange(&request)).unwrap());
    // Stopped, naming the host; the question is the same however often it is asked; nothing was sent there.
    for _ in 0..3 {
        let Step::NeedsYes(consent) = machine.step() else { panic!("not asked") };
        assert_eq!((consent.host(), consent.reason()), ("login.example.net", ConsentReason::Redirect));
    }
    assert_eq!(standin.hosts_asked(), ["example.org"]);
    machine.agree();
    let Step::Request(request) = machine.step() else { panic!("not sent on") };
    assert_eq!(request.url(), "https://login.example.net/autodiscover/autodiscover.xml");
    machine.response(block_on(standin.exchange(&request)).unwrap());
    assert!(matches!(machine.step(), Step::Done(Outcome::Found(_))));
    assert_eq!(standin.hosts_asked(), ["example.org", "login.example.net"]);

    // Said no: it is never sent there, and the next place in the domain is tried.
    let standin = chain(&["example.org", "login.example.net"], good()).route("autodiscover.example.org", good());
    let run = go(&standin, None, &[false]);
    found(&run);
    assert_eq!(standin.hosts_asked(), ["example.org", "autodiscover.example.org"]);
}

#[test]
fn an_agreement_is_for_that_host_only() {
    // Two hosts outside the domain in a row: the owner is asked about each.
    let standin = chain(&["example.org", "a.example.net", "b.example.net"], good());
    let run = go(&standin, None, &[true, true]);
    found(&run);
    assert_eq!(run.asked.iter().map(|(host, _)| host.as_str()).collect::<Vec<_>>(), ["a.example.net", "b.example.net"]);
    // A look-alike of the domain is not in it.
    let standin = chain(&["example.org", "evil-example.org"], good());
    let run = go(&standin, None, &[false]);
    assert_eq!(run.asked, [("evil-example.org".to_owned(), ConsentReason::Redirect)]);
    assert!(!standin.hosts_asked().contains(&"evil-example.org".to_owned()));
}

#[test]
fn five_redirects_are_followed_and_a_sixth_ends_the_search() {
    let five = ["example.org", "a1.example.org", "a2.example.org", "a3.example.org", "a4.example.org", "a5.example.org"];
    let standin = chain(&five, good());
    found(&go(&standin, None, &[]));
    assert_eq!(standin.hosts_asked().len(), 6);

    let six = ["example.org", "a1.example.org", "a2.example.org", "a3.example.org", "a4.example.org", "a5.example.org", "a6.example.org"];
    let standin = chain(&six, good());
    let run = go(&standin, None, &[]);
    assert_eq!(run.outcome, Outcome::Failed(Failure::TooManyRedirects));
    // The sixth is not even asked, and the other places are not tried: this is a loop, not an absence.
    assert_eq!(standin.hosts_asked().len(), 6);
    assert!(!standin.hosts_asked().contains(&"a6.example.org".to_owned()));

    // A loop between two hosts ends the same way.
    let standin = AutodiscoverStandIn::new()
        .route("example.org", hop("x", "autodiscover.example.org").1)
        .route("autodiscover.example.org", hop("x", "example.org").1);
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::Failed(Failure::TooManyRedirects));
    assert_eq!(standin.hosts_asked().len(), 6);
}

#[test]
fn a_redirect_that_is_not_https_is_refused() {
    for location in [
        "http://mail.example.org/autodiscover/autodiscover.xml",
        "HTTP://mail.example.org/autodiscover/autodiscover.xml",
        "http://127.0.0.1/autodiscover/autodiscover.xml",
        "ftp://mail.example.org/",
        "//mail.example.org/autodiscover/autodiscover.xml",
        "autodiscover/autodiscover.xml",
        "https://user:pw@mail.example.org/autodiscover/autodiscover.xml",
        "https://mail.example.org/a b",
        "javascript:alert(1)",
        "",
    ] {
        let standin = AutodiscoverStandIn::new().route("example.org", Route::Redirect(302, location.into())).route("mail.example.org", good());
        let run = run_with(&standin, search("ada@example.org"), None, &[]);
        assert_eq!(run.outcome, Outcome::Failed(Failure::InsecureRedirect), "{location:?}");
        // (The stand-in fails the test by itself if anything is asked over plain HTTP.)
        assert_eq!(standin.hosts_asked(), ["example.org", "autodiscover.example.org"], "{location:?}");
    }
    // The same in an answer's redirectUrl.
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Document(redirect_url("http://mail.example.org/autodiscover/autodiscover.xml")));
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::Failed(Failure::InsecureRedirect));
}

#[test]
fn redirect_url_in_an_answer_is_a_redirect_like_any() {
    let standin = AutodiscoverStandIn::new()
        .route("example.org", Route::Document(redirect_url("https://exch01.example.org/Autodiscover/Autodiscover.xml")))
        .route("exch01.example.org", good());
    let run = go(&standin, None, &[]);
    found(&run);
    assert!(run.asked.is_empty());
    let standin = AutodiscoverStandIn::new()
        .route("example.org", Route::Document(redirect_url("https://exch01.example.net/autodiscover/autodiscover.xml")))
        .route("exch01.example.net", good());
    let run = go(&standin, None, &[true]);
    found(&run);
    assert_eq!(run.asked, [("exch01.example.net".to_owned(), ConsentReason::Redirect)]);
}

#[test]
fn redirect_addr_starts_again_with_the_new_address_and_may_ask_about_its_domain() {
    // Inside the domain: no question.
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Document(redirect_addr("Ada@Mail.Example.org"))).route("autodiscover.mail.example.org", good());
    let run = go(&standin, None, &[]);
    found(&run);
    assert!(run.asked.is_empty());
    assert_eq!(standin.state().seen.last().map(|seen| seen.email.clone()), Some("Ada@mail.example.org".to_owned()));

    // Another domain: asked once for the domain, then its places are asked without more questions.
    let make = || {
        AutodiscoverStandIn::new()
            .route("example.org", Route::Document(redirect_addr("ada@example.net")))
            .route("example.net", Route::Status(404))
            .route("autodiscover.example.net", good())
    };
    let standin = make();
    let run = go(&standin, None, &[true]);
    found(&run);
    assert_eq!(run.asked, [("example.net".to_owned(), ConsentReason::OtherDomain)]);
    assert_eq!(standin.hosts_asked(), ["example.org", "example.net", "autodiscover.example.net"]);

    // Declined: nothing goes to that domain, and nothing else is tried in it.
    let standin = make();
    let run = run_with(&standin, search("ada@example.org"), Some(vec!["srv.example.net"]), &[false]);
    assert_eq!(run.outcome, Outcome::Failed(Failure::Declined));
    assert_eq!(standin.hosts_asked(), ["example.org"]);

    // Garbage in place of an address.
    for bad in ["", "nobody", "a@b", "a b@example.net", "<ada@example.net>", "a@@example.net"] {
        let standin = AutodiscoverStandIn::new().route("example.org", Route::Document(redirect_addr(bad)));
        assert!(matches!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::Failed(_)), "{bad:?}");
        assert_eq!(standin.hosts_asked().len(), 2, "{bad:?}");
    }
}

// ---- Exchange Online ----

#[test]
fn a_mailbox_in_exchange_online_is_said_so_and_nothing_is_sent_there() {
    // The settings name a Microsoft 365 host.
    let online = Route::Document(settings("https://outlook.office365.com/EWS/Exchange.asmx", "https://outlook.office365.com/EWS/Exchange.asmx", "outlook.office365.com"));
    let standin = AutodiscoverStandIn::new().route("example.org", online);
    let run = go(&standin, None, &[]);
    assert_eq!(run.outcome, Outcome::ExchangeOnline);
    assert!(run.asked.is_empty());

    // Only the server says so.
    let route = Route::Document(settings(INTERNAL, EXTERNAL, "mail.eur01.outlook.com"));
    let standin = AutodiscoverStandIn::new().route("example.org", route);
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::ExchangeOnline);

    // A redirect to Microsoft's Autodiscover: never asked, never sent the sign-in.
    for target in ["https://autodiscover-s.outlook.com/autodiscover/autodiscover.xml", "https://outlook.office365.com/autodiscover/autodiscover.xml"] {
        let standin = AutodiscoverStandIn::new()
            .route("example.org", Route::Redirect(302, target.into()))
            .route("autodiscover-s.outlook.com", good())
            .route("outlook.office365.com", good());
        let run = run_with(&standin, search("ada@example.org"), None, &[true, true]);
        assert_eq!(run.outcome, Outcome::ExchangeOnline, "{target}");
        assert!(run.asked.is_empty(), "{target}");
        assert_eq!(standin.hosts_asked(), ["example.org"], "{target}");
    }
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Document(redirect_url("https://autodiscover-s.outlook.com/autodiscover/autodiscover.xml")));
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[true]).outcome, Outcome::ExchangeOnline);

    // A redirect to an address in a tenant.
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Document(redirect_addr("ada@example.onmicrosoft.com")));
    let run = run_with(&standin, search("ada@example.org"), None, &[true]);
    assert_eq!((run.outcome, run.asked.len(), standin.hosts_asked().len()), (Outcome::ExchangeOnline, 0, 1));

    // The owner's own address is a Microsoft one: no request at all.
    for email in ["ada@outlook.com", "ada@Hotmail.OUTLOOK.com", "ada@contoso.onmicrosoft.com"] {
        let standin = AutodiscoverStandIn::new();
        assert_eq!(run_with(&standin, search(email), None, &[]).outcome, Outcome::ExchangeOnline, "{email}");
        assert!(standin.hosts_asked().is_empty(), "{email}");
    }
}

#[test]
fn look_alikes_of_microsoft_hosts_are_not_exchange_online() {
    for yes in ["outlook.com", "mail.outlook.com", "outlook.office365.com", "x.y.onmicrosoft.com", "partner.outlook.cn"] {
        assert!(is_exchange_online(yes), "{yes}");
    }
    for no in ["notoutlook.com", "outlook.com.example.org", "office365.com.evil.example", "example.org", "outlook", "", "xoffice.com"] {
        assert!(!is_exchange_online(no), "{no}");
    }
}

// ---- the server's own address ----

#[test]
fn an_address_on_another_host_asks_before_the_password_goes_there() {
    let route = || Route::Document(settings("https://exch.hoster.example.net/EWS/Exchange.asmx", "https://exch.hoster.example.net/EWS/Exchange.asmx", "exch.hoster.example.net"));
    let standin = AutodiscoverStandIn::new().route("example.org", route());
    let run = go(&standin, None, &[true]);
    assert_eq!(found(&run).urls(), ["https://exch.hoster.example.net/EWS/Exchange.asmx"]);
    assert_eq!(run.asked, [("exch.hoster.example.net".to_owned(), ConsentReason::MailServer)]);

    let standin = AutodiscoverStandIn::new().route("example.org", route());
    let run = run_with(&standin, search("ada@example.org"), None, &[false]);
    assert_eq!(run.outcome, Outcome::Failed(Failure::Declined));

    // A host agreed to for a redirect is not asked about again for the address.
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Redirect(302, "https://exch.hoster.example.net/autodiscover/autodiscover.xml".into())).route("exch.hoster.example.net", route());
    let run = run_with(&standin, search("ada@example.org"), None, &[true]);
    found(&run);
    assert_eq!(run.asked.len(), 1);
}

// ---- failures ----

#[test]
fn a_refused_sign_in_ends_the_search_at_the_first_host() {
    let standin = AutodiscoverStandIn::new().route("example.org", good()).route("autodiscover.example.org", good());
    let machine = Autodiscover::new("ada@example.org", Secret::new("Basic d3Jvbmc6d3Jvbmc=")).unwrap();
    let run = run_with(&standin, machine, Some(vec!["ad.example.org"]), &[]);
    assert_eq!(run.outcome, Outcome::Failed(Failure::SignInRefused));
    assert_eq!(standin.hosts_asked(), ["example.org"]);
    assert!(run.srv.is_empty());
}

#[test]
fn each_failure_has_its_reason() {
    let case = |route: Route| {
        let standin = AutodiscoverStandIn::new().route("example.org", route.clone()).route("autodiscover.example.org", route);
        run_with(&standin, search("ada@example.org"), None, &[]).outcome
    };
    assert_eq!(case(Route::Status(404)), Outcome::Failed(Failure::NoAutodiscover));
    assert_eq!(case(Route::Status(500)), Outcome::Failed(Failure::NoAutodiscover));
    assert_eq!(case(Route::Status(403)), Outcome::Failed(Failure::ServerRefused));
    assert_eq!(case(Route::Document(error_answer())), Outcome::Failed(Failure::ServerRefused));
    assert_eq!(case(Route::TlsFails), Outcome::Failed(Failure::SecureConnection));
    assert_eq!(case(Route::Unreachable), Outcome::Failed(Failure::Unreachable));
    assert_eq!(case(Route::Bytes(b"<html><body>Sign in</body></html>".to_vec())), Outcome::Failed(Failure::Unreadable));
    assert_eq!(case(Route::Document(settings("", "", "mail.example.org"))), Outcome::Failed(Failure::NoExchangeAddress));
    assert_eq!(case(Route::Status(204)), Outcome::Failed(Failure::NoAutodiscover));
    // The most telling reason is kept when the places differ.
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Unreachable).route("autodiscover.example.org", Route::Status(403));
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::Failed(Failure::ServerRefused));
}

#[test]
fn every_failure_offers_server_settings_and_holds_nothing_a_server_wrote() {
    for failure in [
        Failure::Unreachable,
        Failure::NoAutodiscover,
        Failure::SecureConnection,
        Failure::Unreadable,
        Failure::InsecureRedirect,
        Failure::ServerRefused,
        Failure::NoExchangeAddress,
        Failure::Declined,
        Failure::SignInRefused,
        Failure::TooManyRedirects,
    ] {
        let text = failure.to_string();
        assert!(text.contains("Server settings"), "{text}");
        assert!(!text.contains("ada") && !text.contains("exch") && !text.contains("https:"), "{text}");
    }
    assert!(AddressError.to_string().contains("Server settings"));
}

// ---- hostile answers ----

fn answer_of(account: &str) -> Vec<u8> {
    format!("<Autodiscover xmlns=\"{ANSWER_SCHEMA}\"><Response xmlns=\"{RESPONSE_SCHEMA}\"><Account>{account}</Account></Response></Autodiscover>").into_bytes()
}

fn protocol(kind: &str, extra: &str) -> String {
    format!("<Protocol><Type>{kind}</Type>{extra}</Protocol>")
}

#[test]
fn only_the_address_and_the_server_are_taken() {
    let account = format!(
        "<Action>settings</Action>{}{}{}",
        protocol("EXCH", "<Server>Mail.Example.org.</Server><EwsUrl>https://mail.example.org/EWS/A.asmx</EwsUrl><ASUrl>https://evil.example.net/AS</ASUrl><OOFUrl>https://evil.example.net/OOF</OOFUrl><EmwsUrl>https://evil.example.net/EMWS</EmwsUrl><EwsPartnerUrl>https://evil.example.net/P</EwsPartnerUrl>"),
        protocol("EXCH", "<EwsUrl>https://evil.example.net/second</EwsUrl>"),
        protocol("WEB", "<EwsUrl>https://evil.example.net/web</EwsUrl>")
    );
    let Answer::Settings(found) = read_answer(&answer_of(&account)) else { panic!() };
    assert_eq!(found.urls(), ["https://mail.example.org/EWS/A.asmx"]);
    assert_eq!(found.server(), Some("mail.example.org"));
    assert!(found.internal_url().is_some() && found.external_url().is_none());
    // Debug shows only what is there.
    assert!(!format!("{found:?}").contains("mail"), "{found:?}");
}

#[test]
fn addresses_that_are_not_https_are_not_taken() {
    for bad in [
        "http://mail.example.org/EWS/Exchange.asmx",
        "HTTP://mail.example.org/EWS/Exchange.asmx",
        "http://127.0.0.1/EWS/Exchange.asmx",
        "ftp://mail.example.org/",
        "file:///etc/passwd",
        "//mail.example.org/EWS",
        "mail.example.org/EWS",
        "https://user:pw@mail.example.org/EWS",
        "https://",
        "https:///EWS",
        "https://mail example.org/EWS",
        "https://mail.example.org/\u{202e}EWS",
        "",
    ] {
        let account = format!("<Action>settings</Action>{}", protocol("EXCH", &format!("<EwsUrl>{}</EwsUrl>", xml::escape(bad))));
        assert_eq!(read_answer(&answer_of(&account)), Answer::Settings(Discovered { external: None, internal: None, server: None }), "{bad:?}");
    }
    let long = format!("https://mail.example.org/{}", "a".repeat(3_000));
    let account = format!("<Action>settings</Action>{}", protocol("EXCH", &format!("<EwsUrl>{long}</EwsUrl>")));
    assert!(matches!(read_answer(&answer_of(&account)), Answer::Settings(found) if found.urls().is_empty()));
}

#[test]
fn hostile_documents_are_unreadable_not_trusted() {
    let honest = String::from_utf8(answer_of(&format!("<Action>settings</Action>{}", protocol("EXCH", "<EwsUrl>https://mail.example.org/EWS/Exchange.asmx</EwsUrl>")))).unwrap();
    assert!(matches!(read_answer(honest.as_bytes()), Answer::Settings(_)));
    let bomb = "<!DOCTYPE a [<!ENTITY x \"aaaaaaaaaa\"><!ENTITY y \"&x;&x;&x;&x;&x;&x;&x;&x;\"><!ENTITY z \"&y;&y;&y;&y;&y;&y;&y;&y;\">]>";
    let external = "<!DOCTYPE a [<!ENTITY x SYSTEM \"file:///etc/passwd\">]>";
    for hostile in [
        format!("{bomb}{honest}"),
        format!("{external}{honest}"),
        honest.replace("<Action>settings</Action>", "<Action>settings</Action><!ENTITY x>"),
        format!("{}{honest}{}", "<a>".repeat(5_000), "</a>".repeat(5_000)),
        honest.replace(ANSWER_SCHEMA, "http://example.org/other"),
        honest.replace(RESPONSE_SCHEMA, "http://example.org/other"),
        honest.replace("Autodiscover", "Autodiscovery"),
        honest.replace("<Account>", "<Acount>").replace("</Account>", "</Acount>"),
        honest.replace("settings", "somethingElse"),
        honest.replace("</Autodiscover>", ""),
        String::new(),
        "\u{0}\u{1}\u{2}".to_owned(),
        "not xml".to_owned(),
    ] {
        let read = read_answer(hostile.as_bytes());
        assert!(!matches!(&read, Answer::Settings(found) if found.urls().iter().any(|url| url.contains("passwd"))), "{read:?}");
        assert!(matches!(read, Answer::Unreadable), "{hostile:.120}");
    }
    // An entity that is not one of XML's five spoils the text it is in, and so the address.
    let spoiled = honest.replace("mail.example.org/EWS", "mail.example.org/&bogus;EWS");
    assert!(matches!(read_answer(spoiled.as_bytes()), Answer::Settings(found) if found.urls().is_empty()));
    // Megabytes of address are never read.
    let huge = honest.replace("mail.example.org", &"a".repeat(MAX_ANSWER + 10));
    let standin = AutodiscoverStandIn::new().route("example.org", Route::Bytes(huge.into_bytes()));
    assert_eq!(run_with(&standin, search("ada@example.org"), None, &[]).outcome, Outcome::Failed(Failure::Unreadable));
    // And the request asks the transport to stop reading at the same limit.
    let Step::Request(request) = search("ada@example.org").step() else { panic!() };
    assert_eq!(request.max_response_bytes(), MAX_ANSWER);
}

// ---- the request, the secrets, misuse ----

#[test]
fn the_request_is_the_documented_one_and_the_address_is_escaped() {
    let mut machine = search("ada@example.org");
    let Step::Request(request) = machine.step() else { panic!() };
    assert_eq!(request.method(), Method::Post);
    assert_eq!(request.field("content-type"), Some("text/xml; charset=utf-8"));
    assert_eq!(request.field("authorization"), Some(BASIC));
    let body = String::from_utf8(request.body().to_vec()).unwrap();
    assert!(body.contains(&format!("<AcceptableResponseSchema>{RESPONSE_SCHEMA}</AcceptableResponseSchema>")) && body.contains("<EMailAddress>ada@example.org</EMailAddress>"), "{body}");
    // The debug output of the request and the machine holds neither address nor secret.
    for shown in [format!("{request:?}"), format!("{machine:?}"), format!("{:?}", Consent { host: "secret.example.net".into(), reason: ConsentReason::Redirect })] {
        assert!(!shown.contains("example") && !shown.contains("Basic") && !shown.contains("c3Rhbm"), "{shown}");
    }
    // An address with markup in it cannot be made.
    for bad in ["a<b>@example.org", "ada@exa mple.org", "ada@localhost", "@example.org", "ada", "ada@", "ada@-x.org", "ada@example..org", "ada@exämple.org", "a\r\nb@example.org"] {
        assert!(Autodiscover::new(bad, Secret::new(BASIC)).is_err(), "{bad:?}");
    }
    assert!(Autodiscover::new(" Ada.Lovelace+tag@Example.ORG. ", Secret::new(BASIC)).is_ok());
}

#[test]
fn calls_out_of_turn_are_ignored() {
    let mut machine = search("ada@example.org");
    machine.response(Ok(HttpResponse::new(200, Vec::new())));
    machine.srv(Some(vec!["x.example.org".into()]));
    machine.agree();
    machine.decline();
    assert!(matches!(machine.step(), Step::Request(request) if request.url() == "https://example.org/autodiscover/autodiscover.xml"));
    machine.agree();
    machine.srv(None);
    assert!(matches!(machine.step(), Step::Request(_)));
    // A finished search stays finished.
    let standin = AutodiscoverStandIn::new().route("example.org", good());
    let mut machine = search("ada@example.org");
    let _ = block_on(machine.run(&standin, |_| None, |_| false));
    let before = standin.hosts_asked().len();
    machine.response(Ok(HttpResponse::new(404, Vec::new())));
    assert!(matches!(machine.step(), Step::Done(Outcome::Found(_))));
    assert_eq!(standin.hosts_asked().len(), before);
}

#[test]
fn the_domain_tests() {
    assert!(is_within("example.org", "example.org") && is_within("a.b.example.org", "example.org"));
    assert!(!is_within("evil-example.org", "example.org") && !is_within("example.org", "a.example.org") && !is_within("example.org.evil.net", "example.org"));
    assert_eq!(host_of("https://Mail.Example.org.:8443/x?y#z").as_deref(), Some("mail.example.org"));
    assert_eq!(host_of("https://[::1]:8443/x").as_deref(), Some("::1"));
    assert_eq!(host_of("http://example.org/"), None);
    assert_eq!(host_of("https:///x"), None);
    // A host that parsers read differently is no host.
    for odd in ["https://evil.net\\.example.org/x", "https://evil.net%2eexample.org/x", "https://exa mple.org/", "https://evil.net;.example.org/", "https://evil.net,.example.org/"] {
        assert_eq!(host_of(odd), None, "{odd}");
    }
    assert!(check_endpoint("https://evil.net\\.example.org/x").is_err());
}

// ---- any answer, in any order ----

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_answers_are_never_a_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        let _ = read_answer(&bytes);
    }

    #[test]
    fn xml_shaped_text_is_never_a_panic(text in "(<[A-Za-z:]{0,10}( xmlns=\"[a-z:/.0-9]{0,20}\")?/?>|</[A-Za-z:]{0,10}>|[a-z &;#0-9]{0,8}|<!DOCTYPE|&[a-z]{0,5};){0,60}") {
        let _ = read_answer(text.as_bytes());
    }

    #[test]
    fn any_run_of_responses_ends_with_nothing_sent_outside_the_domain(
        responses in proptest::collection::vec((any::<u16>(), "[ -~]{0,60}", proptest::collection::vec(any::<u8>(), 0..200), 0u8..4), 0..40),
        srv in proptest::collection::vec("[ -~]{0,30}", 0..5),
    ) {
        let mut machine = search("ada@example.org");
        let mut feed = responses.into_iter();
        let mut done = false;
        for _ in 0..200 {
            match machine.step() {
                Step::Request(request) => {
                    // Only ever HTTPS, and only to the owner's domain: nobody has agreed to anything.
                    prop_assert!(request.url().starts_with("https://"));
                    let host = host_of(request.url()).unwrap_or_default();
                    prop_assert!(is_within(&host, "example.org"), "{host}");
                    match feed.next() {
                        Some((status, location, body, kind)) => {
                            let response = HttpResponse::new(status, body).with_field("Location", &location);
                            machine.response(if kind == 0 { Err(TransportError::Other) } else { Ok(response) });
                        }
                        None => machine.response(Err(TransportError::TimedOut)),
                    }
                }
                Step::LookupSrv(_) => machine.srv(Some(srv.clone())),
                Step::NeedsYes(_) => machine.decline(),
                Step::Done(_) => { done = true; break; }
            }
        }
        prop_assert!(done);
    }
}

//! The owner's choice of how Outlook's Favorites are read for an account of an organisation,
//! the whole way for each branch: the window's commands, this host's jobs for Microsoft Graph
//! and for Exchange Web Services, `mailcraft-net` and the engine, against stand-ins that take
//! only what the documented services take (`mailcraft_net::graph::standin`,
//! `mailcraft_net::ews::standin`). Nothing here leaves this machine, nobody is signed in, and
//! every name and id is invented.
//!
//! The host's jobs run on the test's own thread (the stand-ins are spoken to in the services'
//! place on that thread only). Where the host would ask Microsoft for a token or put its page
//! in front of the owner, the test answers in its place.

use std::sync::{Arc, Mutex};

use mailcraft_engine::Session;
use mailcraft_model::sync::Tokens;
use mailcraft_model::{Directory, FavoritesSource, FolderKind, Id, Mailbox};
use mailcraft_net::NetError;
use mailcraft_net::ews::standin as ews;
use mailcraft_net::graph::standin as graph;
use mailcraft_net::oauth::GraphTokens;
use mailcraft_ui_egui::favorites_sync::{EWS_NOT_ALLOWED, EWS_SWITCHED_OFF, NEEDS_APPROVAL, REFUSED_FOR_ACCOUNT};
use mailcraft_ui_egui::{EwsRequest, GraphRequest, MailApp, Services, SignInRequest};
use serde_json::{Value, json};

/// Wednesday 2026-10-14, 16:00 UTC; and noon of the last day Exchange Web Services is there.
const NOW: i64 = 1_791_993_600;
const LAST_DAY: i64 = 1_806_494_400;
const SECOND: f64 = 1_000.0;
const MINUTE: f64 = 60.0 * SECOND;
/// The directory of the organisation the account belongs to, as its sign-in names it, and the
/// application this installation is known to Microsoft by: both invented.
const ORGANISATION: &str = "3a1c5e7b-9d2f-4a6c-8e0b-1d3f5a7c9e2b";
const APPLICATION: &str = "0f6c8d2a-1b3e-4c5d-9e7f-a1b2c3d4e5f6";

/// What Microsoft's page comes to when the owner is sent there, which the test stands in for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    /// The owner agrees.
    Agrees,
    /// The organisation keeps the permission for its administrators to give.
    NeedsApproval,
    /// This kind of account has no such permission.
    NoSuchThing,
}

/// What the owner has agreed to, as Microsoft's token endpoint knows, and what its page does.
struct Microsoft {
    /// Graph's two permissions for the Favorites.
    graph: bool,
    /// Exchange Web Services.
    ews: bool,
    page: Page,
    /// The token Exchange Web Services takes at the moment.
    ews_token: String,
    /// The sign-ins the window asked for: whether for Graph, for the Favorites, for EWS.
    sign_ins: Vec<(bool, bool, bool)>,
    /// How often a token for Exchange Web Services was asked for without the owner.
    ews_renewals: usize,
}

/// The folders the mailbox has here, by what they are in the tests.
#[derive(Clone, Copy)]
struct Here {
    inbox: Id,
    sent: Id,
    projects: Id,
    launch: Id,
    receipts: Id,
    team: Id,
}

struct Flow {
    app: MailApp,
    /// Microsoft Graph, where Outlook on the web keeps its list, and Exchange Web Services,
    /// where classic Outlook keeps its own.
    graph: graph::StandIn,
    ews: ews::StandIn,
    account: Id,
    here: Here,
    microsoft: Arc<Mutex<Microsoft>>,
    at: f64,
    _routes: (graph::Routed, ews::Routed),
}

fn fresh(access: &str, tenant: &str) -> Tokens {
    Tokens { access: access.into(), refresh: "ref".into(), expires_at: mailcraft_time::now() + 3_600, tenant: tenant.into(), ..Default::default() }
}

impl Flow {
    /// An account of an organisation that signs in with Microsoft, signed in, with the same
    /// folders here as on the server; two favourites of the owner's own making; and Outlook's
    /// lists on the server. Through Graph: Receipts, the Inbox, Launch. Through Exchange Web
    /// Services (classic Outlook's, which is another list): Team Lumen, Launch, Sent Items, the
    /// Inbox, among shortcuts to a folder that is gone and to one of another mailbox. The
    /// owner hasn't agreed to either way yet.
    fn start() -> Flow {
        Flow::start_with(Mailbox::with_local_account("", ""), NOW, ORGANISATION)
    }

    /// [`start`](Self::start) on `mailbox` (the account is added when it has none that signs
    /// in with Microsoft), at `now` by the session's clock, with a sign-in that names `tenant`.
    fn start_with(mailbox: Mailbox, now: i64, tenant: &str) -> Flow {
        let graph_server = graph::StandIn::start().unwrap();
        {
            let mut state = graph_server.state();
            state.owner = "avery@lumen.example".into();
            let inbox = state.well_known("inbox");
            let projects = state.add_folder("Projects", Some(&inbox));
            let apollo = state.add_folder("Apollo", Some(&projects));
            let launch = state.add_folder("Launch", Some(&apollo));
            let receipts = state.add_folder("Receipts", None);
            state.add_folder("Team Lumen", None);
            state.add_favorite(&receipts, 0);
            state.add_favorite(&inbox, 2);
            state.add_favorite(&launch, 4);
            // (Nobody has agreed to reading it yet.)
            state.configuration = graph::Configuration::Denied;
        }
        let ews_server = ews::StandIn::start().unwrap();
        {
            let mut state = ews_server.state();
            let (inbox, sent) = (state.distinguished("inbox"), state.distinguished("sentitems"));
            let projects = state.add_folder("Projects", Some(&inbox));
            let apollo = state.add_folder("Apollo", Some(&projects));
            let launch = state.add_folder("Launch", Some(&apollo));
            state.add_folder("Receipts", None);
            let team = state.add_folder("Team Lumen", None);
            state.add_favorite(&inbox, &[0xC0]);
            state.add_favorite(&team, &[0x40]);
            state.add_favorite(&sent, &[0x80, 0x80]);
            state.add_favorite(&launch, &[0x80]);
            state.add_stale_favorite(&[0x10]);
            state.add_favorite_elsewhere(&[0x20]);
        }
        let routes = (graph_server.route(), ews_server.route());
        let microsoft = Arc::new(Mutex::new(Microsoft {
            graph: false,
            ews: false,
            page: Page::Agrees,
            ews_token: ews::TOKEN.into(),
            sign_ins: Vec::new(),
            ews_renewals: 0,
        }));
        let mut app = MailApp::new(Session::at(mailbox, now, 0), Services { microsoft_client_id: APPLICATION.into(), ..Default::default() });
        app.autosave = false;
        let (asked, directory) = (microsoft.clone(), tenant.to_string());
        app.services.graph = Some(Box::new(move |request: GraphRequest| {
            let (tx, rx) = std::sync::mpsc::channel();
            // The host's own job, with the test in the token endpoint's place.
            let result = super::graph_with(request, "UTC", &|_, _, favorites| {
                let covers = favorites && asked.lock().unwrap().graph;
                Ok(GraphTokens { tokens: fresh(graph::TOKEN, &directory), extras_missing: false, covers_favorites: covers })
            });
            let _ = tx.send(result);
            rx
        }));
        let (asked, directory) = (microsoft.clone(), tenant.to_string());
        app.services.ews = Some(Box::new(move |request: EwsRequest| {
            let (tx, rx) = std::sync::mpsc::channel();
            // The host's own job, with the test in the token endpoint's place: a token when
            // the owner has agreed to Exchange Web Services, and "not agreed to" otherwise.
            let result = super::ews_with(request, &|_, _| {
                let mut microsoft = asked.lock().unwrap();
                microsoft.ews_renewals += 1;
                match microsoft.ews {
                    true => Ok(fresh(&microsoft.ews_token, &directory)),
                    false => Err(NetError::ConsentNeeded("the sign-in doesn't cover Exchange Web Services".into())),
                }
            });
            let _ = tx.send(result);
            rx
        }));
        let (asked, directory) = (microsoft.clone(), tenant.to_string());
        app.services.sign_in = Some(Box::new(move |request: SignInRequest| {
            let mut microsoft = asked.lock().unwrap();
            microsoft.sign_ins.push((request.graph, request.favorites, request.ews));
            let answer = match microsoft.page {
                Page::Agrees if request.ews => {
                    microsoft.ews = true;
                    Ok(fresh(&microsoft.ews_token, &directory))
                }
                Page::Agrees => {
                    microsoft.graph |= request.favorites;
                    Ok(fresh(graph::TOKEN, &directory))
                }
                Page::NeedsApproval => Err(NEEDS_APPROVAL.to_string()),
                Page::NoSuchThing => Err(REFUSED_FOR_ACCOUNT.to_string()),
            };
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(answer);
            rx
        }));
        // The host's own way of making the link an administrator is sent.
        app.services.admin_consent_link = Some(Box::new(|oauth, tenant, source| mailcraft_net::oauth::admin_consent_url(oauth, tenant, source).ok()));
        let microsoft_account = |app: &MailApp| app.session.mailbox.accounts.iter().find(|a| a.oauth.is_some()).map(|a| a.id);
        let account = match microsoft_account(&app) {
            Some(account) => account,
            None => {
                let added = app.execute(
                    "account.add",
                    json!({"email": "avery@lumen.example", "auth": "oauth2", "provider": "microsoft", "incoming": {"host": "imap.lumen.example"}}),
                );
                let account = added.unwrap()["id"].as_u64().unwrap();
                let inbox = app.session.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
                let mut folder = |name: &str, parent: Option<Id>| {
                    app.execute("folder.new", json!({"name": name, "parent": parent, "account": account})).unwrap()["id"].as_u64().unwrap()
                };
                let projects = folder("Projects", Some(inbox));
                let apollo = folder("Apollo", Some(projects));
                folder("Launch", Some(apollo));
                folder("Receipts", None);
                folder("Team Lumen", None);
                // The owner's own favourites of that account: its Inbox (as every account's is) and Projects.
                app.execute("folder.favorite", json!({"id": projects, "value": true})).unwrap();
                account
            }
        };
        // Signed in, as a Send/Receive leaves it: the sign-in said which directory it was.
        app.seed_graph_token(account, fresh(graph::TOKEN, tenant));
        let mailbox = &app.session.mailbox;
        let named = |path: &str| mailbox.folders.iter().find(|f| f.account == account && mailbox.folder_path(f.id) == path).unwrap().id;
        let here = Here {
            inbox: named("Inbox"),
            sent: named("Sent Items"),
            projects: named("Inbox/Projects"),
            launch: named("Inbox/Projects/Apollo/Launch"),
            receipts: named("Receipts"),
            team: named("Team Lumen"),
        };
        Flow { app, graph: graph_server, ews: ews_server, account, here, microsoft, at: mailcraft_ui_egui::now_ms(), _routes: routes }
    }

    /// Look at the account's servers `ms` on: the jobs that are due start, their answers are
    /// taken in, and a page the owner was sent to has come back.
    fn look(&mut self, ms: f64) {
        self.at += ms;
        for _ in 0..3 {
            self.app.calendar_tick_at(true, self.at);
            self.app.poll_calendar();
            self.app.favorites_tick_at(self.at);
            self.app.poll_favorites();
            self.app.poll_sign_in();
        }
    }

    /// The owner chooses (`ui.favoritesSource`), and what follows from it at once has happened.
    fn choose(&mut self, source: &str) -> Value {
        let answer = self.app.execute("ui.favoritesSource", json!({"account": self.account, "source": source})).unwrap();
        self.app.poll_sign_in();
        // (The page the owner agreed on stands for the server letting the list be read.)
        if self.microsoft.lock().unwrap().graph {
            self.graph.state().configuration = graph::Configuration::There;
        }
        self.look(SECOND);
        answer
    }

    /// The folders of the account that the pane lists under Favorites, in its order.
    fn listed(&self) -> Vec<Id> {
        let all = mailcraft_engine::cmd::folder::favorites(&self.app.session);
        all.into_iter().filter(|id| self.app.session.mailbox.folder(*id).is_some_and(|f| f.account == self.account)).collect()
    }

    /// What the window says of the account's Favorites (`ui.favorites`).
    fn state(&mut self) -> Value {
        self.app.execute("ui.favorites", json!({"account": self.account})).unwrap()
    }

    fn sign_ins(&self) -> Vec<(bool, bool, bool)> {
        self.microsoft.lock().unwrap().sign_ins.clone()
    }

    /// How often Graph was asked for the list, and Exchange Web Services for anything.
    fn reads(&self) -> (usize, usize) {
        let graph = self.graph.state().requests.iter().filter(|asked| asked.target.contains("userConfigurations")).count();
        (graph, self.ews.state().requests.len())
    }

    /// Neither server was ever asked to change anything.
    fn nothing_was_written(&self) -> bool {
        self.graph.state().writes().is_empty() && self.ews.state().writes().is_empty()
    }

    fn account(&self) -> &mailcraft_model::Account {
        self.app.session.mailbox.account(self.account).unwrap()
    }
}

#[test]
fn choosing_microsoft_graph_asks_on_microsofts_page_and_the_list_is_outlooks() {
    let mut flow = Flow::start();
    let here = flow.here;
    let own = flow.listed();
    assert_eq!(own, [here.inbox, here.projects]);
    // An account of an organisation: its sign-in named a directory that is not the one of
    // personal accounts. Nothing of that, or of a choice, is in the mailbox until one is made.
    let before = String::from_utf8(mailcraft_engine::io::write_mailbox(&flow.app.session.mailbox).unwrap()).unwrap();
    assert!(!before.contains("favoritesSource") && !before.contains("directory"));
    flow.look(0.0);
    let state = flow.state();
    assert_eq!(
        (state["choice"].clone(), state["directory"].clone(), state["source"].clone(), state["offered"].clone(), state["line"].clone()),
        (json!(true), json!("organisation"), json!(null), json!(["graph", "ews", "none"]), json!("offer"))
    );
    // (The sign-in said which kind of account it is; that alone writes nothing.)
    let looked = String::from_utf8(mailcraft_engine::io::write_mailbox(&flow.app.session.mailbox).unwrap()).unwrap();
    assert!(!looked.contains("favoritesSource") && !looked.contains("directory"));
    // The offer opens the choice, and doesn't go to anybody's page by itself.
    flow.app.execute("ui.dialog", json!({"name": "favoritesSource", "account": flow.account})).unwrap();
    assert!(flow.app.dialog.as_ref().is_some_and(|dialog| dialog.name() == "favoritesSource") && flow.sign_ins().is_empty());
    flow.app.execute("ui.closeDialog", json!({})).unwrap();

    // Microsoft Graph: its page, for the Favorites' two permissions; then the list is Outlook's.
    let chosen = flow.choose("graph");
    assert_eq!((chosen["pending"].clone(), chosen["source"].clone()), (json!("signIn"), json!("graph")));
    assert_eq!(flow.sign_ins(), [(true, true, false)]);
    assert_eq!(flow.listed(), [here.receipts, here.inbox, here.launch]);
    let state = flow.state();
    assert_eq!((state["from"].clone(), state["line"].clone(), state["source"].clone()), (json!("server"), json!(null), json!("graph")));
    // The choice and the kind of account are kept with the account, and survive the file.
    assert_eq!(
        (flow.account().favorites_source, flow.account().oauth.as_ref().unwrap().directory),
        (FavoritesSource::Graph, Directory::Organisation)
    );
    let file = mailcraft_engine::io::write_mailbox(&flow.app.session.mailbox).unwrap();
    let reopened = mailcraft_engine::io::read_mailbox(&file).unwrap();
    assert_eq!(reopened.account(flow.account).unwrap().favorites_source, FavoritesSource::Graph);
    // No directory's id, and no token, is in it.
    let file = String::from_utf8(file).unwrap();
    assert!(!file.contains(ORGANISATION) && !file.contains(graph::TOKEN) && !file.contains(ews::TOKEN));
    // Exchange Web Services was never spoken to, and nobody was asked for a token for it.
    assert_eq!((flow.reads().1, flow.microsoft.lock().unwrap().ews_renewals), (0, 0));
    // Chosen again where it works already: nothing to agree to, only to read.
    let again = flow.choose("graph");
    assert_eq!((again["pending"].clone(), flow.sign_ins().len()), (json!("read"), 1));
    assert!(flow.nothing_was_written() && flow.app.dialog.is_none());
}

#[test]
fn graph_that_an_administrator_has_to_approve_says_so_with_a_link_and_offers_exchange_web_services() {
    let mut flow = Flow::start();
    let here = flow.here;
    let own = flow.listed();
    flow.look(0.0);
    // The organisation keeps Graph's permission for its administrators to give.
    flow.microsoft.lock().unwrap().page = Page::NeedsApproval;
    let main = flow.graph.state().main_calendar();
    flow.graph.state().add_event(&main, "Budget review", NOW + 3_600);
    flow.choose("graph");
    let state = flow.state();
    assert_eq!(
        (state["line"].clone(), state["text"].clone(), state["steps"].clone(), state["from"].clone()),
        (json!("approval"), json!(NEEDS_APPROVAL), json!(["copyLink", "useEws"]), json!("mailbox"))
    );
    // Nothing else is any different: the owner's favourites, the calendar, no dialog, no problem.
    assert_eq!(flow.listed(), own);
    flow.look(6.0 * MINUTE);
    assert!(flow.app.session.mailbox.events.values().any(|event| event.title == "Budget review"));
    assert!(flow.app.dialog.is_none() && !flow.app.has_sync_problems());

    // The link for the administrator: Microsoft's admin-consent page, for this application and
    // the two permissions, in the organisation's own directory. Nothing is opened or sent.
    let copied = flow.app.execute("ui.copyAdminConsentLink", json!({"account": flow.account})).unwrap();
    let link = copied["url"].as_str().unwrap();
    assert!(link.starts_with(&format!("https://login.microsoftonline.com/{ORGANISATION}/v2.0/adminconsent?client_id={APPLICATION}&")), "{link}");
    assert!(link.contains("MailboxConfigItem.Read") && link.contains("Mail.ReadBasic") && link.contains("redirect_uri=http%3A%2F%2Flocalhost"));
    assert!(!link.contains("EWS") && !link.contains("avery") && !link.contains(graph::TOKEN) && copied["source"] == "graph");

    // Not asked again that day, however often the servers are looked at.
    let asked = flow.sign_ins().len();
    for _ in 0..40 {
        flow.look(31.0 * SECOND);
    }
    flow.look(6.0 * 60.0 * MINUTE);
    assert_eq!((flow.sign_ins().len(), flow.state()["line"].clone()), (asked, json!("approval")));

    // Exchange Web Services instead, while it exists: the sign-in doesn't cover it either, so
    // Microsoft's page comes up once, for that alone, because the owner chose it.
    flow.microsoft.lock().unwrap().page = Page::Agrees;
    let chosen = flow.choose("ews");
    assert_eq!(chosen["pending"], "read");
    assert_eq!(flow.sign_ins().last(), Some(&(false, false, true)));
    flow.look(SECOND);
    // Classic Outlook's list, in its order: by the ordinals, and without the shortcuts to a
    // folder that is gone and to one of another mailbox.
    assert_eq!(flow.listed(), [here.team, here.launch, here.sent, here.inbox]);
    let state = flow.state();
    assert_eq!((state["line"].clone(), state["source"].clone(), state["from"].clone()), (json!(null), json!("ews"), json!("server")));
    assert_eq!(flow.sign_ins().len(), asked + 1);
    assert!(flow.nothing_was_written());
}

#[test]
fn choosing_exchange_web_services_reads_classic_outlooks_list_in_its_order() {
    let mut flow = Flow::start();
    let here = flow.here;
    let marks: Vec<Id> = flow.app.session.mailbox.folders.iter().filter(|f| f.favorite).map(|f| f.id).collect();
    flow.look(0.0);
    // The organisation lets its people agree to it, and the owner did once: no page at all.
    flow.microsoft.lock().unwrap().ews = true;
    let chosen = flow.choose("ews");
    assert_eq!((chosen["pending"].clone(), flow.sign_ins().len()), (json!("read"), 0));
    assert_eq!(flow.listed(), [here.team, here.launch, here.sent, here.inbox]);
    assert_eq!(flow.state()["from"], "server");
    // The four questions the reader asks, all of them reads; and Graph is not asked for its list.
    {
        let state = flow.ews.state();
        let asked: Vec<(&str, &str)> = state.requests.iter().map(|asked| (asked.operation.as_str(), asked.traversal.as_str())).collect();
        assert_eq!(asked, [("FindFolder", "Shallow"), ("FindItem", "Associated"), ("GetFolder", ""), ("FindFolder", "Deep")]);
        assert!(state.requests.iter().all(|asked| asked.status == 200 && asked.header("authorization") == format!("Bearer {}", ews::TOKEN)));
    }
    let graph_reads = flow.reads().0;
    // The list comes from Outlook and is changed there; the owner's own marks are kept aside.
    let refused = flow.app.execute("folder.favorite", json!({"id": here.receipts, "value": true})).unwrap_err();
    assert!(refused.contains("Outlook"), "{refused}");
    assert_eq!(flow.app.session.mailbox.folders.iter().filter(|f| f.favorite).map(|f| f.id).collect::<Vec<_>>(), marks);
    // Nothing the server calls a folder by, and no token, is in the mailbox or in the answer.
    let file = String::from_utf8(mailcraft_engine::io::write_mailbox(&flow.app.session.mailbox).unwrap()).unwrap();
    let shown = format!("{file} {}", flow.state());
    assert!(!shown.contains("AAMk") && !shown.contains(ews::TOKEN) && file.contains(r#""favoritesFromServer""#));

    // Not more often than every quarter of an hour; then again, with the token that was kept.
    for _ in 0..20 {
        flow.look(31.0 * SECOND);
    }
    assert_eq!(flow.reads().1, 4);
    {
        // In Outlook meanwhile: Team Lumen is taken off, and Receipts is put first.
        let mut state = flow.ews.state();
        let team = state.folders.iter().find(|folder| folder.name == "Team Lumen").unwrap().entry_id.clone();
        state.shortcuts.retain(|shortcut| shortcut.entry_id.as_ref() != Some(&team));
        let receipts = state.folders.iter().find(|folder| folder.name == "Receipts").unwrap().id.clone();
        state.add_favorite(&receipts, &[0x30]);
    }
    flow.look(5.0 * MINUTE);
    assert_eq!((flow.reads().1, flow.microsoft.lock().unwrap().ews_renewals), (8, 1));
    assert_eq!(flow.listed(), [here.receipts, here.launch, here.sent, here.inbox]);
    // "Try again" and the owner's own Send/Receive read at once.
    flow.app.execute("ui.favorites", json!({"account": flow.account, "refresh": true})).unwrap();
    flow.look(SECOND);
    assert_eq!(flow.reads().1, 12);

    // The service no longer takes the token that was kept: it is dropped, another is had, and
    // the list is read with that. Nothing is said of it.
    flow.ews.state().token = "standin.another-ews-token".into();
    flow.microsoft.lock().unwrap().ews_token = "standin.another-ews-token".into();
    flow.look(16.0 * MINUTE);
    flow.look(SECOND);
    assert_eq!((flow.state()["line"].clone(), flow.microsoft.lock().unwrap().ews_renewals), (json!(null), 2));
    assert_eq!(flow.ews.state().requests.iter().filter(|asked| asked.status == 401).count(), 1);
    assert_eq!(flow.listed(), [here.receipts, here.launch, here.sent, here.inbox]);

    // A read that fails is one quiet line; what was listed stays.
    flow.ews.state().hostile = Some(ews::Hostile::CutShort);
    flow.look(16.0 * MINUTE);
    let state = flow.state();
    assert_eq!(
        (state["line"].clone(), state["text"].clone(), state["from"].clone()),
        (json!("failed"), json!("Couldn't read the list"), json!("server"))
    );
    assert_eq!(flow.listed().len(), 4);
    assert!(flow.app.dialog.is_none() && !flow.app.has_sync_problems() && flow.app.status_msg.is_none());
    flow.ews.state().hostile = None;
    flow.app.execute("ui.favorites", json!({"account": flow.account, "refresh": true})).unwrap();
    flow.look(SECOND);
    assert_eq!(flow.state()["line"], json!(null));
    // Graph was never asked for its list, nobody was sent to a page, and nothing was written.
    assert_eq!((flow.reads().0, flow.sign_ins().len()), (graph_reads, 0));
    assert!(flow.nothing_was_written());
}

#[test]
fn an_organisation_that_does_not_allow_exchange_web_services_is_said_in_a_line_and_graph_is_offered() {
    for reason in [ews::DISABLED, ews::BLOCKED_BY_POLICY] {
        let mut flow = Flow::start();
        let here = flow.here;
        let own = flow.listed();
        flow.look(0.0);
        flow.microsoft.lock().unwrap().ews = true;
        flow.ews.state().blocked = Some(reason.into());
        flow.choose("ews");
        let state = flow.state();
        assert_eq!(
            (state["line"].clone(), state["text"].clone(), state["steps"].clone(), state["from"].clone()),
            (json!("blocked"), json!(EWS_NOT_ALLOWED), json!(["useGraph"]), json!("mailbox")),
            "{reason}"
        );
        // One question was enough; the owner's favourites are shown; nobody is troubled.
        assert_eq!((flow.reads().1, flow.listed()), (1, own.clone()));
        assert!(flow.app.dialog.is_none() && !flow.app.has_sync_problems() && flow.sign_ins().is_empty());
        // Not asked again that day; the next day, once.
        for _ in 0..40 {
            flow.look(31.0 * SECOND);
        }
        flow.look(6.0 * 60.0 * MINUTE);
        assert_eq!(flow.reads().1, 1);
        flow.look(19.0 * 60.0 * MINUTE);
        assert_eq!((flow.reads().1, flow.state()["line"].clone()), (2, json!("blocked")));

        // Microsoft Graph instead: its page, and then Outlook's list as Graph has it.
        let chosen = flow.choose("graph");
        assert_eq!((chosen["pending"].clone(), flow.sign_ins()), (json!("signIn"), vec![(true, true, false)]));
        assert_eq!(flow.listed(), [here.receipts, here.inbox, here.launch]);
        let state = flow.state();
        assert_eq!((state["line"].clone(), state["source"].clone()), (json!(null), json!("graph")));
        // Exchange Web Services is left alone from then on.
        flow.look(16.0 * MINUTE);
        assert_eq!(flow.reads().1, 2);
        assert!(flow.nothing_was_written());
    }
}

#[test]
fn not_now_keeps_mailcrafts_own_favorites_and_asks_nobody() {
    let mut flow = Flow::start();
    let here = flow.here;
    let own = flow.listed();
    let chosen = flow.choose("none");
    assert_eq!((chosen["pending"].clone(), chosen["source"].clone(), chosen["from"].clone()), (json!(null), json!("none"), json!("mailbox")));
    for _ in 0..5 {
        flow.look(16.0 * MINUTE);
    }
    // Neither server is asked for a list, nobody is sent anywhere, and the favourites made
    // here work as they always did.
    assert_eq!((flow.reads(), flow.sign_ins().len(), flow.microsoft.lock().unwrap().ews_renewals), ((0, 0), 0, 0));
    assert_eq!(flow.listed(), own);
    flow.app.execute("folder.favorite", json!({"id": here.team, "value": true})).unwrap();
    assert_eq!(flow.listed(), [here.inbox, here.projects, here.team]);
    // "Not now" is not "never": the offer stands under the heading, and opens the same choice.
    let state = flow.state();
    assert_eq!(
        (state["line"].clone(), state["text"].clone(), state["choice"].clone()),
        (json!("offer"), json!("Show Outlook's Favorites"), json!(true))
    );
    assert_eq!(flow.account().favorites_source, FavoritesSource::None);

    // Later, Exchange Web Services after all; and then "not now" again: Outlook's list goes,
    // and the three made here are back exactly.
    flow.microsoft.lock().unwrap().ews = true;
    flow.choose("ews");
    assert_eq!(flow.listed(), [here.team, here.launch, here.sent, here.inbox]);
    flow.choose("none");
    assert_eq!(flow.listed(), [here.inbox, here.projects, here.team]);
    let asked = flow.reads();
    flow.look(16.0 * MINUTE);
    assert_eq!(flow.reads(), asked);
    assert!(flow.nothing_was_written() && flow.app.dialog.is_none());
}

#[test]
fn from_the_first_of_april_2027_exchange_web_services_is_not_offered_and_an_account_set_to_it_says_so() {
    // The last day: offered, chosen, and read.
    let mut flow = Flow::start_with(Mailbox::with_local_account("", ""), LAST_DAY, ORGANISATION);
    let here = flow.here;
    flow.microsoft.lock().unwrap().ews = true;
    assert_eq!(flow.state()["offered"], json!(["graph", "ews", "none"]));
    flow.choose("ews");
    assert_eq!(flow.listed(), [here.team, here.launch, here.sent, here.inbox]);
    let mailbox = flow.app.session.mailbox.clone();
    drop(flow);

    // The next day, with the mailbox as it was left: nothing is asked of Exchange Web
    // Services, the pane says in a line that Microsoft has switched it off and offers Graph,
    // and the favourites kept in MailCraft are shown.
    let mut flow = Flow::start_with(mailbox, LAST_DAY + 86_400, ORGANISATION);
    flow.microsoft.lock().unwrap().ews = true;
    assert_eq!(flow.account().favorites_source, FavoritesSource::Ews);
    flow.look(0.0);
    flow.look(16.0 * MINUTE);
    let state = flow.state();
    assert_eq!(
        (state["line"].clone(), state["text"].clone(), state["steps"].clone(), state["offered"].clone(), state["from"].clone()),
        (json!("ended"), json!(EWS_SWITCHED_OFF), json!(["useGraph"]), json!(["graph", "none"]), json!("mailbox"))
    );
    assert_eq!((flow.reads(), flow.microsoft.lock().unwrap().ews_renewals), ((0, 0), 0));
    assert_eq!(flow.listed(), [here.inbox, here.projects]);
    // It can't be chosen any more, by anybody.
    let refused = flow.app.execute("ui.favoritesSource", json!({"account": flow.account, "source": "ews"})).unwrap_err();
    assert!(refused.contains("1 April 2027"), "{refused}");
    // Microsoft Graph takes its place when the owner says so.
    flow.choose("graph");
    assert_eq!(flow.listed(), [here.receipts, here.inbox, here.launch]);
    assert_eq!((flow.state()["line"].clone(), flow.reads().1), (json!(null), 0));
    assert!(flow.nothing_was_written());
}

#[test]
fn a_personal_microsoft_account_has_no_choice_and_reads_through_graph_as_before() {
    // Its sign-in names the directory of personal accounts.
    let mut flow = Flow::start_with(Mailbox::with_local_account("", ""), NOW, Directory::PERSONAL_TENANT);
    let here = flow.here;
    flow.look(0.0);
    let state = flow.state();
    assert_eq!(
        (state["choice"].clone(), state["directory"].clone(), state["offered"].clone(), state["line"].clone()),
        (json!(false), json!("personal"), json!(["graph", "none"]), json!("offer"))
    );
    // The choice doesn't open for it, and Exchange Web Services can't be chosen behind its back.
    assert!(flow.app.execute("ui.dialog", json!({"name": "favoritesSource", "account": flow.account})).is_err());
    assert!(flow.app.execute("ui.favoritesSource", json!({"account": flow.account, "source": "ews"})).unwrap_err().contains("personal"));
    assert_eq!(flow.account().favorites_source, FavoritesSource::Unset);
    // The offer is what it was before there was a choice: Graph's page, and then the list.
    flow.app.execute("ui.connectFavorites", json!({"account": flow.account})).unwrap();
    flow.app.poll_sign_in();
    flow.graph.state().configuration = graph::Configuration::There;
    flow.look(SECOND);
    assert_eq!((flow.sign_ins(), flow.listed()), (vec![(true, true, false)], vec![here.receipts, here.inbox, here.launch]));
    assert_eq!((flow.reads().1, flow.microsoft.lock().unwrap().ews_renewals), (0, 0));
    // Nothing was written of a choice it never had.
    let file = String::from_utf8(mailcraft_engine::io::write_mailbox(&flow.app.session.mailbox).unwrap()).unwrap();
    assert!(!file.contains("favoritesSource") && !file.contains(r#""directory""#));

    // An account whose sign-in doesn't cover Exchange Web Services and never will (the page
    // says this kind of account has none): said in a line, and nothing else changes.
    let mut flow = Flow::start();
    flow.look(0.0);
    flow.microsoft.lock().unwrap().page = Page::NoSuchThing;
    flow.choose("ews");
    let state = flow.state();
    assert_eq!(
        (state["line"].clone(), state["text"].clone(), state["from"].clone()),
        (json!("refused"), json!(REFUSED_FOR_ACCOUNT), json!("mailbox"))
    );
    assert_eq!(flow.sign_ins(), [(false, false, true)]);
    assert!(flow.nothing_was_written());
}

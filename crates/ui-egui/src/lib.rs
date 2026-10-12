//! MailCraft's egui front end: an Outlook-style window (title bar with Quick Access Toolbar and
//! search, ribbon, module bar, folder pane, message list, reading pane, calendar, people, tasks,
//! status bar, Backstage, dialogs) over `mailcraft-engine`.
//!
//! The UI is thin: it reads `Session` state and acts through [`MailApp::run`] (command ids), so
//! the ribbon, shortcuts, command search, control channel and MCP all reach the same behaviour.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// An English UI string in the current interface language ([`i18n::t`]).
#[macro_export]
macro_rules! tl {
    ($s:expr) => {
        $crate::i18n::t($s)
    };
}

mod account_setup;
pub mod backstage;
mod calendar_sync;
pub mod calendar_view;
pub mod chrome;
pub mod compose;
pub mod control;
pub mod dialogs;
pub mod files;
pub mod forms;
pub mod i18n;
pub mod icons;
pub mod keys;
pub mod mail_view;
mod meeting_dialogs;
pub mod nav;
pub mod people_view;
pub mod reading;
mod reminders;
pub mod ribbon;
pub mod scheduling;
pub mod tasks_view;
pub mod theme;
pub mod widgets;
pub mod window_geometry;
pub mod windows;

use std::sync::mpsc::Receiver;

use mailcraft_engine::Session;
use mailcraft_model::Id;
use mailcraft_model::sync::{SyncRequest, SyncResult, Tokens};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub use control::ControlRequest;
pub use files::FileDialogRequest;
pub use scheduling::{FreeBusyRequest, FreeBusyResult};

/// Platform services injected by the host (file dialogs, downloads, servers, storage).
#[derive(Default)]
pub struct Services {
    /// Desktop: show a native file dialog without blocking the UI thread. The host sends the
    /// picked path (`None` when cancelled) through the returned channel and wakes the UI; a
    /// dropped sender counts as cancelled. The app shows one dialog at a time ([`files`]).
    pub file_dialog: Option<Box<dyn Fn(FileDialogRequest) -> Receiver<Option<String>>>>,
    /// Web: open a file picker; the file arrives later through `inbox`. The argument says what
    /// for (`mailbox`, `import`, `attach`).
    pub open_async: Option<Box<dyn Fn(&str)>>,
    /// Files (name, bytes) delivered asynchronously (the web picker, drag and drop).
    pub inbox: Option<Inbox>,
    /// Web: hand bytes to the browser as a download. An error means no download started.
    pub download: Option<Box<dyn Fn(&str, &[u8]) -> Result<(), String>>>,
    /// Web: told whether there are unsaved changes after each pass, for the leave-page guard.
    pub on_dirty: Option<Box<dyn Fn(bool)>>,
    /// Desktop: synchronise with a mail server on a background thread. What it brings arrives
    /// through the channel in pieces as it goes (`SyncResult::partial` on all but the last), and
    /// the host wakes the UI for each. Absent in the browser, which can't open sockets.
    pub sync: Option<Box<dyn Fn(SyncRequest) -> Receiver<SyncResult>>>,
    /// Set to call a Send/Receive off: the host's run stops at the next safe place and says so
    /// in its last piece. The host clears it when it starts a run.
    pub sync_cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Desktop: sign an account's owner in on the provider's own page (OAuth 2.0) on a background
    /// thread: renew quietly with the refresh token when there is one, else open the browser and
    /// wait. The tokens, or why there are none, arrive through the channel and the host wakes the
    /// UI. Absent in the browser.
    pub sign_in: Option<Box<dyn Fn(SignInRequest) -> Receiver<Result<Tokens, String>>>>,
    /// Desktop: what Microsoft Graph does for an account that signs in with Microsoft, on a
    /// background thread: send the Outbox (no "authenticated SMTP" switch stands in its way) and
    /// bring the calendar and the contacts down. Absent in the browser.
    pub graph: Option<Box<dyn Fn(GraphRequest) -> Receiver<GraphResult>>>,
    /// Desktop: ask the DNS for a domain's mail exchangers on a background thread (best first),
    /// so account setup can tell whose servers an address at a company's own domain uses. Only
    /// the domain is given, never the mailbox name.
    pub lookup_mx: Option<Box<dyn Fn(String) -> Receiver<Result<Vec<String>, String>>>>,
    /// Desktop: try signing in to an account's servers with a secret (a password, or an access
    /// token for an account that signs in on its provider's page) on a background thread, before
    /// the account is added.
    pub test_account: Option<Box<dyn Fn(mailcraft_model::Account, String) -> Receiver<Result<(), String>>>>,
    /// Desktop: the operating system's credential store (Windows Credential Manager, the macOS
    /// keychain), where a password or a refresh token is remembered between runs when the owner
    /// wants that. Never the mailbox file, the settings or a log.
    pub secrets: Option<Box<dyn SecretStore>>,
    /// The application (client) id this build presents to Microsoft when neither the account nor
    /// the settings name one; empty when the build has none.
    pub microsoft_client_id: String,
    /// Keep the mailbox where the host stores it (the browser's storage); called a moment after
    /// each change. Desktop saves to the mailbox file instead (see `saver`).
    pub persist: Option<Box<dyn Fn(&[u8]) -> Result<(), String>>>,
    /// Desktop: write the mailbox file on a background thread, so the window never waits for the
    /// disk. Without one (tests, scripts) the mailbox is saved in place through `file.save`.
    pub saver: Option<Box<dyn mailcraft_engine::io::BackgroundSave>>,
    /// Desktop: download one picture a message refers to on the web, on a background thread,
    /// once the owner has asked for that message's pictures ([`reading::remote`]). The address
    /// is the message's own choice: the host checks where it leads before going there, takes
    /// only so much and waits only so long (`mailcraft_net::pictures::fetch`). The bytes, or why
    /// there are none, arrive through the channel and the host wakes the UI. Absent in the
    /// browser and the tests: pictures on the web then stay placeholders, and nothing is offered.
    pub pictures: Option<Box<dyn Fn(String) -> Receiver<Result<Vec<u8>, String>>>>,
    /// Desktop: ask an account's server when people are free and busy, on a background thread,
    /// for a meeting's Scheduling Assistant ([`scheduling`]). It reads and writes nothing. The
    /// answer, or why there is none, arrives through the channel and the host wakes the UI.
    /// Absent in the browser and for accounts whose server has no calendar: the assistant then
    /// shows the owner's own calendar only, and says so.
    pub free_busy: Option<Box<dyn Fn(FreeBusyRequest) -> Receiver<FreeBusyResult>>>,
}

/// Files delivered asynchronously.
pub type Inbox = std::sync::Arc<std::sync::Mutex<Vec<(String, Vec<u8>)>>>;

/// The operating system's credential store ([`Services::secrets`]). Keys are `password:<address>`
/// and `refresh:<address>`.
pub trait SecretStore {
    fn get(&self, key: &str) -> Option<String>;
    fn set(&self, key: &str, secret: &str) -> Result<(), String>;
    fn delete(&self, key: &str);
}

/// What the host needs to sign an account's owner in ([`Services::sign_in`]).
pub struct SignInRequest {
    /// The account, or 0 while one is being set up.
    pub account: Id,
    /// The account's sign-in settings; an empty client id means the host's own.
    pub oauth: mailcraft_model::OAuth,
    /// The address, offered to the provider's page so the owner needn't type it.
    pub email: String,
    /// A refresh token from earlier in this session to try first; empty when there is none.
    pub refresh: String,
    /// Set when the owner gives up waiting, for the host to stop too.
    pub cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Nobody asked for this (a Send/Receive that started by itself): renew with the refresh
    /// token or give up, but don't put a sign-in page in front of the owner.
    pub quiet: bool,
    /// Ask for what Microsoft Graph needs (sending, calendar, contacts) rather than for mail:
    /// the owner chose "Connect calendar and contacts".
    pub graph: bool,
    /// Ask for mail alone, without the calendar and contacts along: the owner's way out when the
    /// provider's page won't have the fuller request.
    pub mail_only: bool,
}

/// What the host is asked to do with Microsoft Graph for one account ([`Services::graph`]).
pub struct GraphRequest {
    pub account: mailcraft_model::Account,
    /// The account's sign-in settings, with the application id filled in.
    pub oauth: mailcraft_model::OAuth,
    /// The account's refresh token, to get a Graph token with when `access` is empty.
    pub refresh: String,
    /// A Graph access token from earlier in this session that is still good; else empty.
    pub access: String,
    /// Mail waiting in the Outbox, to send through Graph.
    pub outbox: Vec<mailcraft_model::Message>,
    /// Also bring the calendar and the contacts down.
    pub directory: bool,
    /// A stretch of the calendar to bring down when `directory` isn't asked for (Unix seconds,
    /// from and up to): the days on screen, for a refresh that costs one question.
    pub calendar: Option<(i64, i64)>,
    /// What was made, changed or deleted in the calendar here, for the server to be told before
    /// the calendar is brought down.
    pub changes: Option<mailcraft_model::remote::EventsUpload>,
    /// The calendars of the account to bring down, each with the series the mailbox expects in
    /// the stretch (the account's main calendar is the one with no name; it comes anyway).
    pub asks: Vec<mailcraft_model::remote::CalendarAsk>,
    /// Also bring the working hours the mailbox has set: an extra that takes a permission of its
    /// own, without which the answer says [`GraphResult::extras_missing`].
    pub settings: bool,
}

/// What a [`GraphRequest`] came to. Never an error as a whole: mail goes on either way.
#[derive(Default)]
pub struct GraphResult {
    pub account: Id,
    /// A Graph token that was obtained for this (kept for the session; its refresh token is the
    /// account's newest).
    pub tokens: Option<Tokens>,
    /// The owner hasn't allowed this program to use Graph: "Connect calendar and contacts" asks.
    pub consent_needed: bool,
    /// The Graph token that was handed in is no longer taken.
    pub token_refused: bool,
    /// Outbox messages that were sent (Microsoft files the Sent Items copy itself).
    pub sent: Vec<Id>,
    /// Outbox messages that must not be sent another way in this run: whether they went is not
    /// known, or the server said to try later.
    pub held: Vec<Id>,
    pub events: Option<mailcraft_model::remote::EventsDownload>,
    pub contacts: Option<mailcraft_model::remote::ContactsDownload>,
    /// What the server said to the calendar changes it was told ([`GraphRequest::changes`]).
    pub uploaded: Option<mailcraft_model::remote::EventsUploaded>,
    pub errors: Vec<String>,
    /// The same stretch of the account's other calendars.
    pub more_events: Vec<mailcraft_model::remote::EventsDownload>,
    /// The account's calendars and the groups they are listed in, when they were asked for.
    pub calendars: Option<mailcraft_model::remote::CalendarsDownload>,
    /// The mailbox's own categories with their colours, when they could be read.
    pub categories: Option<mailcraft_model::remote::CategoriesDownload>,
    /// The sign-in covers everything but an optional extra (reading the mailbox's category
    /// list): all else works, and "Connect calendar and contacts" would add it.
    pub extras_missing: bool,
    /// The working hours the mailbox has set, when they were asked for and could be read.
    pub mailbox_hours: Option<mailcraft_engine::cmd::calendar_settings::MailboxHours>,
}

/// A sign-in the host is busy with.
struct SigningIn {
    /// For Microsoft Graph (calendar, contacts, sending) rather than for mail.
    graph: bool,
    account: Id,
    rx: Receiver<Result<Tokens, String>>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// UI state that persists between runs (and that agents can read and set).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UiState {
    pub tab: String,
    pub backstage: bool,
    pub backstage_page: String,
    pub ribbon_collapsed: bool,
    /// The ribbon's layout: one row of commands (simplified, the default), or three rows in groups.
    pub ribbon_mode: ribbon::Mode,
    /// Interface theme: light, dark, or follow the OS appearance.
    pub theme: theme::Appearance,
    /// Interface size in percent: scales the whole interface on top of the system's display scale.
    pub interface_size: theme::InterfaceSize,
    /// Desktop: the main window's size and position, restored at the next launch.
    pub window: Option<window_geometry::WindowGeometry>,
    /// Interface language: `auto` or a language code.
    pub language: String,
    /// Engine preferences; the session owns them while running (see [`MailApp::prefs`]).
    pub options: mailcraft_engine::Prefs,
    /// How the window was arranged; the session owns it while running.
    pub view: mailcraft_engine::ViewState,
    pub folder_pane_width: f32,
    pub list_width: f32,
    /// Folders shown collapsed in the folder pane.
    pub collapsed_folders: Vec<Id>,
    /// Mailbox files opened lately, most recent first.
    pub recent: Vec<String>,
    /// The application (client) id Microsoft knows this installation by, for accounts that sign
    /// in on Microsoft's page and name none of their own. An identifier, not a secret.
    pub microsoft_client_id: String,
    /// Keep passwords and sign-ins in the operating system's credential store between runs.
    pub remember_sign_in: bool,
    /// Reminders the owner dismissed (`event@occurrence`), so they don't come back at the next start.
    pub dismissed_reminders: Vec<String>,
    /// Download the pictures mail refers to on the web without being asked. Off unless the owner
    /// turns it on: a picture on the web tells its sender that the message was opened.
    pub download_pictures: bool,
    /// Senders whose pictures are downloaded without asking (addresses, in lower case).
    pub picture_senders: Vec<String>,
    /// The groups of the calendar list that are folded away under their headings.
    pub collapsed_calendar_groups: Vec<String>,
}

impl Default for UiState {
    fn default() -> Self {
        UiState {
            tab: "Home".into(),
            backstage: false,
            backstage_page: "info".into(),
            ribbon_collapsed: false,
            ribbon_mode: ribbon::Mode::default(),
            theme: theme::Appearance::default(),
            interface_size: theme::InterfaceSize::default(),
            window: None,
            language: i18n::AUTO.into(),
            options: mailcraft_engine::Prefs::default(),
            view: mailcraft_engine::ViewState::default(),
            folder_pane_width: 230.0,
            list_width: 380.0,
            collapsed_folders: Vec::new(),
            recent: Vec::new(),
            microsoft_client_id: String::new(),
            remember_sign_in: true,
            dismissed_reminders: Vec::new(),
            download_pictures: false,
            picture_senders: Vec::new(),
            collapsed_calendar_groups: Vec::new(),
        }
    }
}

/// UI-level commands (`ui.*`): id, label, location, params. They change the window, not the
/// mailbox, and are listed with the engine's commands in command search and `engine.commands`.
pub const UI_COMMANDS: &[(&str, &str, &str, &str)] = &[
    ("ui.tab", "Show Ribbon Tab", "Ribbon", r#"{"tab": "Home"|"Send / Receive"|"Folder"|"View"|"Help"}"#),
    ("ui.backstage", "File", "Ribbon", r#"{"value"?: bool, "page"?: "info"|"open"|"options"|"about"}"#),
    ("ui.collapseRibbon", "Collapse the Ribbon", "Ribbon", r#"{"value"?: bool}"#),
    ("ui.ribbonMode", "Ribbon Layout", "View › Layout", r#"{"value": "simplified"|"classic"}"#),
    (
        "ui.dialog",
        "Open Dialog",
        "Window",
        r#"{"name": "commands"|"event"|"meeting"|"editEvent"|"contact"|"editContact"|"task"|"editTask"|"folder"|"goToDate"|"account"|"categories"|"rules"|"about"|"proposeTime"|"forwardMeeting"|"proposals", …} (account: "email"?: string to start with, or "id": number for that account's settings; proposeTime: "id"?: number (an item of the calendar) | "message": number (an invitation in the mail); forwardMeeting: "id"?: number; proposals: "id"?: number | "message": number — the first two write to other people only from their own Propose and Send buttons)"#,
    ),
    ("ui.closeDialog", "Close Dialog", "Window", "{}"),
    ("ui.theme", "Interface Theme", "File › Options", r#"{"value": "light"|"dark"|"system"}"#),
    ("ui.interfaceSize", "Interface Size", "File › Options", r#"{"value": 80-200}"#),
    ("ui.focusSearch", "Search", "Search", "{}"),
    ("ui.sync", "Send/Receive All Folders", "Send / Receive", r#"{"account"?: number, "password"?: string}"#),
    ("ui.cancelSignIn", "Cancel Sign-In", "Send / Receive", "{}"),
    ("ui.cancelSync", "Cancel Send/Receive", "Send / Receive", "{}"),
    (
        "ui.connectMicrosoft",
        "Connect Calendar and Contacts",
        "File › Info",
        r#"{"account"?: number} — signs in for sending, calendar and contacts through Microsoft"#,
    ),
    ("ui.dismissReminder", "Dismiss Reminder", "Reminders", r#"{"event": number, "occurrence": "YYYY-MM-DDTHH:MM"} | {"all": true}"#),
    (
        "ui.snoozeReminders",
        "Snooze Reminders",
        "Reminders",
        r#"{"minutes": 1-10080, "event"?: number, "occurrence"?: string (default: all that are showing)}"#,
    ),
    ("ui.microsoftClientId", "Microsoft Application ID", "File › Options", r#"{"value": string (empty: the build's own)}"#),
    ("ui.rememberSignIn", "Remember Passwords and Sign-Ins", "File › Options", r#"{"value"?: bool (default: toggle)}"#),
    ("ui.open", "Open Mailbox", "File › Open & Export", "{}"),
    ("ui.import", "Import", "File › Open & Export", "{}"),
    ("ui.attach", "Attach File", "Message › Include", "{}"),
    ("ui.toggleFolder", "Expand or Collapse Folder", "Folder", r#"{"id": number}"#),
    (
        "ui.openWindow",
        "Open in a Window of Its Own",
        "Window",
        r#"{"message": number} | {"event": number, "proposed"?: {"start": "YYYY-MM-DDTHH:MM", "end": same} (another time, as somebody proposed it, put into the fields: nothing is changed or sent until the window sends it)} | {"new": "appointment"|"meeting", "start"?: "YYYY-MM-DDTHH:MM", "end"?: same, "allDay"?: bool, "calendar"?: number, "title"?: string, …the other fields of calendar.new} — a draft opens in the composer instead; a new appointment or meeting is made when its window is saved"#,
    ),
    (
        "ui.closeWindow",
        "Close Window",
        "Window",
        r#"{"window": number} | {"message": number} | {"event": number} | {"all": true} (default: the one in front) — closes without asking, also an appointment with unsaved changes"#,
    ),
    (
        "ui.eventWindow",
        "Appointment Window",
        "Window",
        r#"{"window"?: number (default: the appointment or meeting window in front), "page"?: "appointment"|"scheduling" (a meeting's Scheduling Assistant), "recurrence"?: bool (open or close its Appointment Recurrence dialog), "do"?: "save"|"saveAndClose"|"send"|"close" — as its keys and its close button do: closing asks first when something isn't saved, and nothing is sent to anybody without the owner's yes; "freeBusy"?: {"people": [{"email", "busy": [{"start": seconds, "end": seconds, "showAs"}], "error"?: string}]} — a stand-in for the server's answer about the people invited, for the Scheduling Assistant} — answers {"window", "event", "page", "meeting", "dirty", "recurrence", "asking", "importance": "low"|"normal"|"high", "online": bool (it is, or is to be, a Teams meeting), "joinUrl": string (the link to join by, when its server gave one: data, never followed by itself), "scheduling"?: {"day", "start", "end", "rows", "known", "unknown", "bars", "pending", "note"}}"#,
    ),
    (
        "ui.downloadPictures",
        "Download Pictures",
        "Message",
        r#"{"message"?: number (default: the selected message)} — its pictures from the web, until the app closes"#,
    ),
    (
        "ui.alwaysDownloadPictures",
        "Always Download Pictures from Sender",
        "File › Options",
        r#"{"sender": address} | {"message"?: number} (its sender), "value"?: bool (default: true; false forgets the sender)"#,
    ),
    ("ui.downloadPicturesAutomatically", "Download Pictures in Mail Automatically", "File › Options", r#"{"value"?: bool (default: toggle)}"#),
    (
        "ui.copySelection",
        "Copy Selected Text",
        "Message",
        r#"{"pane": true} | {"window": number} (default: where the keys go) — the text selected in a message being read, to the clipboard as plain text"#,
    ),
    (
        "ui.selectAllInMessage",
        "Select All Text in Message",
        "Message",
        r#"{"pane": true} | {"window": number} (default: where the keys go) — the whole body of the message being read"#,
    ),
    ("ui.copyLinkAddress", "Copy Link Address", "Message", r#"{"url": string} — a link's address (http, https, mailto or tel) to the clipboard"#),
    (
        "ui.toggleCalendarGroup",
        "Expand or Collapse Calendar Group",
        "Calendar",
        r#"{"group": string (its heading in the list of calendars), "value"?: bool (true: folded away; default: the other way than it is)}"#,
    ),
    (
        "ui.joinMeeting",
        "Join Online Meeting",
        "Meeting › Teams Meeting",
        r#"{"event"?: number (default: the selected item)} — opens the link to join an online meeting by in the system's browser. The link comes from the meeting's server: only an https address is opened, and only on this command"#,
    ),
    (
        "ui.findInvitation",
        "Look for This Meeting on the Calendar",
        "Meeting › Respond",
        r#"{"message"?: number (default: the selected message)} — asks the account's server for the days round the meeting a message invites to, when the calendar here doesn't have it yet"#,
    ),
];

/// The application.
pub struct MailApp {
    pub session: Session,
    pub ui: UiState,
    pub services: Services,
    pub dialog: Option<dialogs::Dialog>,
    pub status_msg: Option<(String, f64)>,
    /// macOS: the window has no title bar; leave room for the traffic lights.
    pub integrated_titlebar: bool,
    /// The search box's text while it is being typed.
    pub(crate) search_text: String,
    pub(crate) search_focus: bool,
    /// The Bcc line of the message being written is shown.
    pub(crate) compose_bcc: bool,
    /// The message list should scroll to the selected row (after a keyboard move).
    pub(crate) reveal_selection: bool,
    /// Passwords typed this session, per account. Never saved.
    passwords: std::collections::HashMap<Id, String>,
    /// What signing in on a provider's page brought back this session, per account. Never saved.
    tokens: std::collections::HashMap<Id, Tokens>,
    /// Microsoft Graph tokens obtained this session, per account. Never saved.
    graph_tokens: std::collections::HashMap<Id, Tokens>,
    /// What Graph is doing for an account before its mail is synchronised: the account, whether
    /// nobody asked, whether the calendar and contacts were asked for, and the answer to come.
    graph_rx: Option<(Id, bool, bool, Receiver<GraphResult>)>,
    /// When each account's calendar and contacts were last brought down (milliseconds).
    graph_checked: std::collections::HashMap<Id, f64>,
    /// Accounts whose owner hasn't allowed Graph yet ("Connect calendar and contacts").
    graph_consent_needed: std::collections::HashSet<Id>,
    /// Keeping the calendar current between Send/Receive runs ([`calendar_sync`]).
    calendar: calendar_sync::CalendarSync,
    /// The next sign-in asks for mail alone (account setup's "mail only").
    mail_only_next: bool,
    /// A sign-in the host is waiting on (the owner is in the browser).
    signing_in: Option<SigningIn>,
    /// What a sign-in for an account that is being set up came to, until its dialog takes it.
    setup_sign_in: Option<Result<Tokens, String>>,
    /// A synchronisation in flight.
    sync_rx: Option<Receiver<SyncResult>>,
    /// What the run in flight has come to so far, piece by piece.
    sync_totals: mailcraft_engine::cmd::account::SyncTotals,
    /// The run in flight: whose it is, how much was still waiting to go out when it turned to
    /// bringing mail down (unknown until then), and whether it was called off to let more out.
    sync_run: Option<(Id, Option<usize>, bool)>,
    /// A piece of the run in flight said the server refused the sign-in.
    sync_refused: bool,
    control_rx: Option<Receiver<ControlRequest>>,
    shot_token: u64,
    queued_shots: Vec<(u64, f64, u32)>,
    pending_shots: Vec<(u64, Option<String>, std::sync::mpsc::Sender<Value>, f64)>,
    pub(crate) synthetic: Vec<egui::Event>,
    styled: bool,
    /// The interface theme setting last installed in egui ([`theme::apply`]).
    applied_theme: Option<theme::Appearance>,
    fonts_frames: u32,
    pub frame_ms: f64,
    /// The window title last sent; a viewport command schedules a repaint, so only send changes.
    sent_title: String,
    pub quit_requested: bool,
    /// Save the mailbox by itself a moment after each change (on by default).
    pub autosave: bool,
    /// Open account setup when the app starts on a mailbox nobody has set up yet (the host turns
    /// this on for an ordinary start; not for the sample or a mailbox named on the command line).
    pub offer_setup: bool,
    /// Send/Receive by itself when the app starts and every few minutes (on by default; how often
    /// is the owner's `syncMinutes` option).
    pub auto_sync: bool,
    /// When the last round that started by itself began (milliseconds; 0 before the first).
    last_auto_sync: f64,
    /// Accounts waiting their turn in a Send/Receive round, and whether nobody asked for it.
    sync_queue: std::collections::VecDeque<(Id, bool)>,
    /// When it was last looked whether something made here waits to go to a server (milliseconds).
    push_checked: f64,
    /// Accounts whose next run that nobody asked for only tells the server what changed here
    /// (the quick way out for a read mark or a move), bringing nothing down.
    push_only_next: std::collections::HashSet<Id>,
    /// Per account: how much was still waiting after its last Send/Receive, and when that was.
    /// What a server refuses keeps waiting, and must not start a run every few seconds.
    push_seen: std::collections::HashMap<Id, (usize, f64)>,
    last_autosave: f64,
    saved_rev: u64,
    /// The egui context, once the first frame has run (background work wakes the UI with it).
    pub(crate) ctx: Option<egui::Context>,
    /// The file dialog the host is showing, and what its answer is for ([`files`]).
    pub(crate) file_dialog: Option<files::PendingDialog>,
    /// Web: what the file the picker will deliver is for.
    pub(crate) pending_open: Option<String>,
    /// Where each calendar occurrence was drawn (for clicks injected by tests and agents).
    pub(crate) hit: control::HitMap,
    /// The reminders window: what is due, what was put off.
    pub(crate) reminders: reminders::State,
    /// What went wrong in each account's last Send/Receive, until a run of it goes through:
    /// the account, its name, when, and the lines.
    sync_problems: Vec<(Id, String, String, Vec<String>)>,
    /// Messages and appointments opened in windows of their own ([`windows`]).
    pub(crate) windows: windows::Windows,
    /// Pictures from the web: which messages may show them, and the ones on their way.
    pub(crate) remote_pictures: reading::RemotePictures,
    /// The text selected in the messages that are being read, in the pane and in windows.
    pub(crate) text_selection: reading::Selections,
    /// The last calendar command the owner ran to its end ([`MailApp::run`]), numbered, with how
    /// it went: an appointment window whose save had to ask first learns from it what came of
    /// the question once it is answered.
    pub(crate) ran: (u64, String, Result<Value, String>),
    /// When people are free and busy, as the Scheduling Assistant found out ([`scheduling`]).
    pub(crate) free_busy: scheduling::Store,
}

impl MailApp {
    pub fn new(session: Session, services: Services) -> Self {
        let saved_rev = session.rev();
        MailApp {
            session,
            ui: UiState::default(),
            services,
            dialog: None,
            status_msg: None,
            integrated_titlebar: false,
            search_text: String::new(),
            search_focus: false,
            compose_bcc: false,
            reveal_selection: false,
            passwords: Default::default(),
            tokens: Default::default(),
            graph_tokens: Default::default(),
            graph_rx: None,
            graph_checked: Default::default(),
            graph_consent_needed: Default::default(),
            calendar: Default::default(),
            mail_only_next: false,
            signing_in: None,
            setup_sign_in: None,
            sync_rx: None,
            sync_totals: Default::default(),
            sync_run: None,
            sync_refused: false,
            control_rx: None,
            shot_token: 0,
            queued_shots: Vec::new(),
            pending_shots: Vec::new(),
            synthetic: Vec::new(),
            styled: false,
            applied_theme: None,
            fonts_frames: 0,
            frame_ms: 0.0,
            sent_title: String::new(),
            quit_requested: false,
            autosave: true,
            offer_setup: false,
            auto_sync: true,
            last_auto_sync: 0.0,
            sync_queue: Default::default(),
            push_checked: 0.0,
            push_only_next: Default::default(),
            push_seen: Default::default(),
            last_autosave: 0.0,
            saved_rev,
            ctx: None,
            file_dialog: None,
            pending_open: None,
            hit: Default::default(),
            reminders: Default::default(),
            sync_problems: Vec::new(),
            windows: Default::default(),
            remote_pictures: Default::default(),
            text_selection: Default::default(),
            ran: (0, String::new(), Ok(Value::Null)),
            free_busy: Default::default(),
        }
    }

    pub fn with_control(mut self, rx: Receiver<ControlRequest>) -> Self {
        self.control_rx = Some(rx);
        self
    }

    /// Preferences to save between runs: the UI state plus the engine's preferences and view.
    pub fn prefs(&self) -> UiState {
        let mut ui = self.ui.clone();
        ui.options = self.session.prefs.clone();
        ui.view = self.session.view.clone();
        // What is searched for and looked at belongs to the run, not to the next one.
        ui.view.search.clear();
        ui.view.people_search.clear();
        ui
    }

    /// Restore preferences saved by [`MailApp::prefs`].
    pub fn apply_prefs(&mut self, ui: UiState) {
        self.ui = ui;
        self.ui.backstage = false;
        self.ui.folder_pane_width = if self.ui.folder_pane_width.is_finite() { self.ui.folder_pane_width.clamp(150.0, 500.0) } else { 230.0 };
        self.ui.list_width = if self.ui.list_width.is_finite() { self.ui.list_width.clamp(240.0, 900.0) } else { 380.0 };
        self.ui.collapsed_folders.truncate(10_000);
        self.ui.recent.truncate(20);
        self.ui.collapsed_calendar_groups.truncate(200);
        self.session.prefs = self.ui.options.clone();
        // Layout choices carry over; the folder, the day and the search start fresh.
        let saved = self.ui.view.clone();
        let v = &mut self.session.view;
        (v.sort, v.sort_descending, v.conversations, v.reading_pane, v.folder_pane, v.calendar, v.task_filter) =
            (saved.sort, saved.sort_descending, saved.conversations, saved.reading_pane, saved.folder_pane, saved.calendar, saved.task_filter);
        // Which calendars lie over one another is a layout choice too (calendars that this
        // mailbox doesn't have are simply not found).
        (v.overlay, v.overlay_front) = (saved.overlay.iter().copied().take(200).collect(), saved.overlay_front);
    }

    /// Whether a command exists and is enabled.
    pub fn enabled(&self, id: &str) -> bool {
        match self.session.registry.get(id) {
            Some(s) => (s.enabled)(&self.session).is_none(),
            None if id == "ui.sync" => {
                self.session.enabled("sync.request")
                    && self.services.sync.is_some()
                    && self.sync_rx.is_none()
                    && self.graph_rx.is_none()
                    && self.signing_in.is_none()
            }
            None if id == "ui.cancelSignIn" => self.signing_in.is_some(),
            None if id == "ui.cancelSync" => self.sync_rx.is_some(),
            None if id == "ui.copySelection" => self.text_selection.any(),
            None => UI_COMMANDS.iter().any(|c| c.0 == id),
        }
    }

    /// Run a command the user asked for (ribbon, shortcut, menu, file drop). A command that needs
    /// input the user hasn't given yet opens its dialog; one that writes a file asks where (or, in
    /// the browser, downloads it); a failure shows in the status bar.
    pub fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        // (Whether one occurrence or the series was meant when a recurring item was opened.)
        let params = dialogs::as_opened(self, id, params);
        if let Some(name) = dialogs::input_dialog(id, &params)
            && self.enabled(id)
        {
            // An appointment or a meeting has a window of its own to be filled in.
            if let Some(opened) = windows::open_form(self, name, &params) {
                return opened;
            }
            self.dialog = dialogs::open_dialog(self, name, &params);
            return Ok(json!({"pending": name}));
        }
        if let Some(r) = files::intercept(self, id, &params) {
            return r;
        }
        // What can't be taken back is asked about first, once.
        // (An answer to an invitation in the mail has a question of its own.)
        if params.get("confirmed").and_then(Value::as_bool) != Some(true)
            && let Some(ask) = reading::invitation_question(self, id, &params).unwrap_or_else(|| dialogs::confirmation(self, id, &params))
        {
            let params = if params.is_object() { params } else { json!({}) };
            self.dialog = Some(dialogs::Dialog::Confirm { command: id.to_string(), params, ask });
            return Ok(json!({"pending": "confirm"}));
        }
        let r = self.execute(id, params);
        if id.starts_with("calendar.") {
            self.ran = (self.ran.0.wrapping_add(1), id.to_string(), r.clone());
        }
        match &r {
            Err(e) => self.status(e.clone()),
            Ok(v) => {
                if let Some(msg) = feedback(id, v) {
                    self.status(msg);
                }
            }
        }
        if matches!(id, "mail.next" | "mail.previous") {
            self.reveal_selection = true;
        }
        r
    }

    /// Run a command for a script or an agent (control channel, MCP bridge): never asks first.
    /// UI-level commands (`ui.*`) are handled here, the rest by the engine.
    pub fn execute(&mut self, id: &str, params: Value) -> Result<Value, String> {
        if let Some(r) = self.ui_command(id, &params) {
            return r;
        }
        let composing = self.session.compose.is_some();
        // An account that goes takes what was remembered for it along.
        let leaving = if id == "account.remove" {
            params.get("id").and_then(Value::as_u64).and_then(|a| self.session.mailbox.account(a)).map(|a| (a.id, a.email.clone()))
        } else {
            None
        };
        let r = self.session.run(id, &params).map_err(|e| e.to_string());
        // Asked for the reminders: their window comes up, due or not.
        if r.is_ok() && id == "view.reminders" {
            self.reminders.asked_for = true;
        }
        if let (Ok(_), Some((account, address))) = (&r, leaving) {
            // Its calendar and contacts were the server's, not the owner's own.
            let _ = mailcraft_engine::cmd::remote::forget(&mut self.session, account);
            self.graph_tokens.remove(&account);
            self.graph_checked.remove(&account);
            self.graph_consent_needed.remove(&account);
            self.passwords.remove(&account);
            self.tokens.remove(&account);
            self.forget_secrets(&address);
        }
        // Starting to write brings up the Message tab; finishing goes back to Home.
        if r.is_ok() && composing != self.session.compose.is_some() {
            self.compose_bcc = false;
            self.ui.tab = if composing { "Home".into() } else { "Message".into() };
            self.ui.backstage = false;
        }
        r
    }

    fn ui_command(&mut self, id: &str, p: &Value) -> Option<Result<Value, String>> {
        let s = |k: &str| p.get(k).and_then(Value::as_str);
        let b = |k: &str| p.get(k).and_then(Value::as_bool);
        Some(match id {
            "ui.tab" => {
                let Some(tab) = s("tab") else { return Some(Err("missing `tab`".into())) };
                if tab == "File" {
                    self.ui.backstage = true;
                } else if ribbon::tabs(self).contains(&tab) {
                    self.ui.tab = tab.to_string();
                    self.ui.backstage = false;
                    self.ui.ribbon_collapsed = false;
                } else {
                    return Some(Err(format!("no ribbon tab \"{tab}\"")));
                }
                Ok(json!({"tab": self.ui.tab}))
            }
            "ui.backstage" => {
                self.ui.backstage = b("value").unwrap_or(!self.ui.backstage);
                if let Some(page) = s("page") {
                    if !backstage::PAGES.iter().any(|(code, _, _)| *code == page) {
                        return Some(Err(format!("no File page \"{page}\"")));
                    }
                    self.ui.backstage_page = page.to_string();
                }
                Ok(json!({"value": self.ui.backstage, "page": self.ui.backstage_page}))
            }
            "ui.collapseRibbon" => {
                self.ui.ribbon_collapsed = b("value").unwrap_or(!self.ui.ribbon_collapsed);
                Ok(json!({"value": self.ui.ribbon_collapsed}))
            }
            "ui.ribbonMode" => match s("value").and_then(ribbon::Mode::from_code) {
                Some(mode) => {
                    self.ui.ribbon_mode = mode;
                    Ok(json!({"value": mode.code()}))
                }
                None => Err("`value` is simplified or classic".into()),
            },
            "ui.dialog" => {
                let Some(name) = s("name") else { return Some(Err("missing `name`".into())) };
                match dialogs::open_dialog(self, name, p) {
                    Some(d) => {
                        self.dialog = Some(d);
                        Ok(json!({"dialog": name}))
                    }
                    None => Err(format!("can't open the \"{name}\" dialog now")),
                }
            }
            "ui.closeDialog" => {
                self.dialog = None;
                Ok(json!({}))
            }
            "ui.theme" => match s("value").and_then(theme::Appearance::from_code) {
                Some(a) => {
                    self.ui.theme = a;
                    Ok(json!({"value": a.code()}))
                }
                None => Err("`value` is light, dark or system".into()),
            },
            "ui.interfaceSize" => match p.get("value").and_then(Value::as_f64).and_then(theme::InterfaceSize::from_percent) {
                Some(size) => {
                    self.ui.interface_size = size;
                    Ok(json!({"value": size.percent()}))
                }
                None => Err("`value` is a percentage from 80 to 200".into()),
            },
            "ui.focusSearch" => {
                self.search_focus = true;
                self.ui.backstage = false;
                let _ = self.session.run("view.mail", &json!({}));
                Ok(json!({}))
            }
            "ui.sync" => self.start_sync(p),
            "ui.microsoftClientId" => {
                let id = s("value").unwrap_or_default().trim();
                if id.len() > 200 || !id.chars().all(|c| c.is_ascii_alphanumeric() || "-._".contains(c)) {
                    return Some(Err(tl!("An application ID is letters, digits and dashes").to_string()));
                }
                self.ui.microsoft_client_id = id.to_string();
                Ok(json!({"value": id}))
            }
            "ui.rememberSignIn" => {
                let on = p.get("value").and_then(Value::as_bool).unwrap_or(!self.ui.remember_sign_in);
                self.ui.remember_sign_in = on;
                if !on {
                    // Switched off: what was kept goes too.
                    let addresses: Vec<String> = self.session.mailbox.accounts.iter().map(|a| a.email.clone()).collect();
                    for address in addresses {
                        self.forget_secrets(&address);
                    }
                }
                Ok(json!({"value": on}))
            }
            "ui.dismissReminder" => {
                let one = p.get("event").and_then(Value::as_u64).zip(s("occurrence"));
                if one.is_none() && p.get("all").and_then(Value::as_bool) != Some(true) {
                    return Some(Err(tl!("Say which reminder (`event` and `occurrence`), or `all`").to_string()));
                }
                Ok(json!({"dismissed": reminders::dismiss(self, one)}))
            }
            "ui.snoozeReminders" => {
                let Some(minutes) = p.get("minutes").and_then(Value::as_u64).filter(|m| (1..=10_080).contains(m)) else {
                    return Some(Err(tl!("`minutes` is 1 to 10080").to_string()));
                };
                let one = p.get("event").and_then(Value::as_u64).zip(s("occurrence"));
                Ok(json!({"snoozed": reminders::snooze(self, one, minutes), "minutes": minutes}))
            }
            "ui.connectMicrosoft" => {
                let account = p
                    .get("account")
                    .and_then(Value::as_u64)
                    .or_else(|| self.session.mailbox.accounts.iter().find(|a| a.oauth.is_some()).map(|a| a.id))
                    .and_then(|id| self.session.mailbox.account(id).cloned());
                let Some(account) = account else { return Some(Err(tl!("No account signs in with Microsoft").to_string())) };
                let Some(oauth) = account.oauth.clone() else { return Some(Err(tl!("This account doesn't sign in with Microsoft").to_string())) };
                self.begin_sign_in(account.id, oauth, &account.email, String::new(), false, true)
                    .map(|()| json!({"pending": "signIn", "account": account.id}))
            }
            "ui.cancelSync" => {
                if self.sync_rx.is_none() {
                    return Some(Err(tl!("No Send/Receive is running").to_string()));
                }
                // The run stops at the next safe place; what it brought stays. The rest of a
                // round that was under way is called off with it.
                self.services.sync_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                self.sync_queue.clear();
                self.session.sync_status = tl!("Stopping…").to_string();
                Ok(json!({"cancelling": true}))
            }
            "ui.cancelSignIn" => match self.signing_in.take() {
                Some(s) => {
                    s.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    self.session.sync_status = tl!("Sign-in cancelled").to_string();
                    Ok(json!({"cancelled": s.account}))
                }
                None => Err(tl!("Nobody is signing in").to_string()),
            },
            "ui.open" => {
                files::open(self, "mailbox");
                Ok(json!({"pending": "open"}))
            }
            "ui.import" => {
                files::open(self, "import");
                Ok(json!({"pending": "import"}))
            }
            "ui.attach" => {
                if self.session.compose.is_none() {
                    return Some(Err("no message is being written".into()));
                }
                files::open(self, "attach");
                Ok(json!({"pending": "attach"}))
            }
            "ui.toggleFolder" => {
                let Some(id) = p.get("id").and_then(Value::as_u64) else { return Some(Err("missing `id`".into())) };
                match self.ui.collapsed_folders.iter().position(|x| *x == id) {
                    Some(i) => {
                        self.ui.collapsed_folders.remove(i);
                    }
                    None => self.ui.collapsed_folders.push(id),
                }
                Ok(json!({"collapsed": self.ui.collapsed_folders.contains(&id)}))
            }
            _ => {
                return windows::ui_command(self, id, p)
                    .or_else(|| reading::remote::ui_command(self, id, p))
                    .or_else(|| calendar_view::ui_command(self, id, p))
                    .or_else(|| meeting_dialogs::ui_command(self, id, p));
            }
        })
    }

    fn secret_key(kind: &str, address: &str) -> String {
        format!("{kind}:{}", address.trim().to_ascii_lowercase())
    }

    /// What the owner had the operating system remember for `address` (`password` or `refresh`).
    fn remembered(&self, kind: &str, address: &str) -> Option<String> {
        if !self.ui.remember_sign_in {
            return None;
        }
        self.services.secrets.as_ref()?.get(&Self::secret_key(kind, address)).filter(|secret| !secret.is_empty())
    }

    /// Have the operating system remember a secret for `address`, when the owner wants that.
    fn remember(&self, kind: &str, address: &str, secret: &str) {
        if !self.ui.remember_sign_in || secret.is_empty() {
            return;
        }
        if let Some(store) = &self.services.secrets
            && let Err(e) = store.set(&Self::secret_key(kind, address), secret)
        {
            log::warn!("couldn't keep the sign-in for next time: {e}");
        }
    }

    /// Drop everything the operating system remembers for `address`.
    pub(crate) fn forget_secrets(&self, address: &str) {
        if let Some(store) = &self.services.secrets {
            for kind in ["password", "refresh"] {
                store.delete(&Self::secret_key(kind, address));
            }
        }
    }

    /// The sign-in settings as the host should use them: an account that names no application id
    /// of its own takes this installation's.
    pub(crate) fn effective_oauth(&self, mut oauth: mailcraft_model::OAuth) -> mailcraft_model::OAuth {
        if oauth.client_id.is_empty() && oauth.provider == mailcraft_model::OAuthProvider::Microsoft {
            let own = &self.ui.microsoft_client_id;
            oauth.client_id = if own.is_empty() { self.services.microsoft_client_id.clone() } else { own.clone() };
        }
        oauth
    }

    /// Hand a sign-in to the host. `account` is 0 while an account is being set up.
    fn begin_sign_in(
        &mut self,
        account: Id,
        oauth: mailcraft_model::OAuth,
        email: &str,
        refresh: String,
        quiet: bool,
        graph: bool,
    ) -> Result<(), String> {
        let mail_only = std::mem::take(&mut self.mail_only_next);
        if self.signing_in.is_some() {
            return Err(tl!("Finish signing in first (your browser is waiting)").to_string());
        }
        let Some(sign_in) = &self.services.sign_in else {
            return Err(tl!("This build can't sign in to this kind of account. Use the desktop app.").to_string());
        };
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let waiting = if refresh.is_empty() { tl!("Sign in on the page that opened in your browser…") } else { tl!("Renewing the sign-in…") };
        let request = SignInRequest {
            account,
            oauth: self.effective_oauth(oauth),
            email: email.to_string(),
            refresh,
            cancel: cancel.clone(),
            quiet,
            graph,
            mail_only,
        };
        let rx = sign_in(request);
        self.signing_in = Some(SigningIn { graph, account, rx, cancel });
        self.session.sync_status = waiting.to_string();
        Ok(())
    }

    /// Account setup: sign the owner in on the provider's page before the account exists. The
    /// answer is picked up with [`take_setup_sign_in`](Self::take_setup_sign_in).
    pub(crate) fn begin_setup_sign_in(&mut self, oauth: mailcraft_model::OAuth, email: &str, mail_only: bool) -> Result<(), String> {
        self.setup_sign_in = None;
        self.mail_only_next = mail_only;
        self.begin_sign_in(0, oauth, email, String::new(), false, false)
    }

    /// Account setup: what the sign-in started with `begin_setup_sign_in` came to, once it has.
    pub(crate) fn take_setup_sign_in(&mut self) -> Option<Result<Tokens, String>> {
        self.setup_sign_in.take()
    }

    /// Account setup: the account exists now; keep what signing in brought back for it.
    pub(crate) fn adopt_tokens(&mut self, account: Id, tokens: Tokens) {
        if let Some(address) = self.session.mailbox.account(account).map(|a| a.email.clone()) {
            self.remember("refresh", &address, &tokens.refresh);
        }
        self.tokens.insert(account, tokens);
    }

    /// Account setup: the password that was just shown to work, for this session's Send/Receive
    /// (it is remembered for later runs once the first one has gone through).
    pub(crate) fn adopt_password(&mut self, account: Id, password: String) {
        self.passwords.insert(account, password);
    }

    /// Start a Send/Receive for an account (default: the first one with a server). Asks for the
    /// password the first time, or has the owner sign in on the provider's page; the answer is
    /// kept in memory until the app closes, and by the operating system when that is wanted.
    fn start_sync(&mut self, p: &Value) -> Result<Value, String> {
        if self.sync_rx.is_some() || self.graph_rx.is_some() {
            return Err(tl!("Send/Receive is already running").to_string());
        }
        if self.services.sync.is_none() {
            return Err(tl!("This build can't connect to mail servers (a browser can't open mail connections). Use the desktop app, or bring mail in with File › Import.").to_string());
        }
        let quiet = p.get("quiet").and_then(Value::as_bool).unwrap_or(false);
        let account = match p.get("account").and_then(Value::as_u64) {
            Some(account) => account,
            // Nobody in particular: every account with a server, one after the other.
            None => {
                let mut all = self.server_accounts().into_iter();
                let first = all.next().ok_or_else(|| tl!("No account is connected to a mail server (File › Add Account)").to_string())?;
                self.sync_queue = all.map(|a| (a, quiet)).collect();
                first
            }
        };
        let mut request = mailcraft_engine::cmd::account::sync_request(&self.session, account).map_err(|e| e.to_string())?;
        // A run that only tells the server what changed here, when that is all it was started for.
        request.push_only = self.push_only_next.remove(&account) && quiet && request.outbox.is_empty();
        let address = request.account.email.clone();
        if let Some(oauth) = request.account.oauth.clone() {
            // Signed in on the provider's page: a token that is still good goes along; without
            // one the host renews it or the owner signs in, and this runs again after.
            let kept = self.tokens.get(&account).map(|t| t.refresh.clone()).filter(|r| !r.is_empty());
            let refresh = kept.or_else(|| self.remembered("refresh", &address)).unwrap_or_default();
            match self.tokens.get(&account).filter(|t| t.fresh(mailcraft_time::now())).map(|t| t.access.clone()) {
                Some(access) => request.access_token = access,
                None => {
                    self.begin_sign_in(account, oauth, &address, refresh, quiet, false)?;
                    return Ok(json!({"pending": "signIn", "account": account}));
                }
            }
            // Microsoft Graph first, for what it does better than the mail protocols: sending
            // (the answer says what it sent, and this runs again for the mail itself) and the
            // calendar and contacts, every quarter of an hour.
            let after_graph = p.get("afterGraph").and_then(Value::as_bool).unwrap_or(false);
            let microsoft = oauth.provider == mailcraft_model::OAuthProvider::Microsoft;
            if !after_graph && microsoft && !refresh.is_empty() && !self.graph_consent_needed.contains(&account) && self.services.graph.is_some() {
                let now = now_ms();
                // The calendar's part is left out while a calendar job of its own is at work
                // ([`calendar_sync`]): the two answers would cross.
                let free = !self.calendar_syncing();
                let directory = free && self.graph_checked.get(&account).is_none_or(|at| now - at > 900_000.0);
                // What was changed in the calendar here goes along; and a Send/Receive the owner
                // asked for brings the days on screen when the wide stretch isn't due.
                let changes = if free { self.calendar_changes(account) } else { None };
                let calendar = (free && !directory && (!quiet || changes.is_some())).then(|| self.calendar_window());
                if directory || !request.outbox.is_empty() || changes.is_some() || calendar.is_some() {
                    let access =
                        self.graph_tokens.get(&account).filter(|t| t.fresh(mailcraft_time::now())).map(|t| t.access.clone()).unwrap_or_default();
                    let ask = GraphRequest {
                        account: request.account.clone(),
                        oauth: self.effective_oauth(oauth),
                        refresh,
                        access,
                        outbox: request.outbox.clone(),
                        directory,
                        calendar,
                        changes,
                        asks: self.calendar_asks(account, directory, calendar),
                        // (The calendar's own job asks for them: [`calendar_sync`].)
                        settings: false,
                    };
                    self.session.sync_status =
                        if ask.outbox.is_empty() { tl!("Updating the calendar and contacts…") } else { tl!("Sending…") }.to_string();
                    if let Some(graph) = &self.services.graph {
                        self.graph_rx = Some((account, quiet, directory, graph(ask)));
                    }
                    return Ok(json!({"pending": "graph", "account": account}));
                }
            }
            // What Graph sent, or couldn't say whether it sent, is not sent again by SMTP.
            let skip: Vec<Id> = p.get("skip").and_then(Value::as_array).map(|ids| ids.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
            request.outbox.retain(|m| !skip.contains(&m.id));
        } else {
            if let Some(pw) = p.get("password").and_then(Value::as_str) {
                self.passwords.insert(account, pw.to_string());
            } else if !self.passwords.contains_key(&account)
                && let Some(pw) = self.remembered("password", &address)
            {
                self.passwords.insert(account, pw);
            }
            let Some(password) = self.passwords.get(&account).cloned() else {
                self.dialog = dialogs::Dialog::open("password", self, &json!({"account": account}));
                return Ok(json!({"pending": "password"}));
            };
            request.password = password;
        }
        let Some(start) = &self.services.sync else { return Err(tl!("This build can't connect to mail servers").to_string()) };
        self.sync_run = Some((account, None, false));
        self.sync_rx = Some(start(request));
        self.session.sync_status = tl!("Sending and receiving…").to_string();
        Ok(json!({"started": true, "account": account}))
    }

    /// Nobody has set this mailbox up: it holds only the placeholder account and no mail.
    pub fn needs_setup(&self) -> bool {
        let mb = &self.session.mailbox;
        mb.accounts.iter().all(|a| a.email.trim().is_empty()) && mb.messages.is_empty()
    }

    fn server_accounts(&self) -> Vec<Id> {
        self.session.mailbox.accounts.iter().filter(|a| a.kind == mailcraft_model::AccountKind::Imap).map(|a| a.id).collect()
    }

    /// How much of `account` waits to go to its server: mail in the Outbox, and changes made here
    /// that the server hasn't been told.
    fn waiting_to_go(&self, account: Id) -> usize {
        let mb = &self.session.mailbox;
        let outbox = mb.folder_of_kind(account, mailcraft_model::FolderKind::Outbox).map_or(0, |f| mb.count(f));
        outbox + mailcraft_engine::cmd::account::unpushed(&self.session, account)
    }

    /// Whether a Send/Receive for `account` can go ahead without asking anybody anything: its
    /// password or sign-in is at hand, in memory or with the operating system.
    fn can_sync_quietly(&self, account: Id) -> bool {
        let Some(a) = self.session.mailbox.account(account) else { return false };
        if a.oauth.is_some() {
            let kept = self.tokens.get(&account).is_some_and(|t| t.fresh(mailcraft_time::now()) || !t.refresh.is_empty());
            self.services.sign_in.is_some() && (kept || self.remembered("refresh", &a.email).is_some())
        } else {
            self.passwords.contains_key(&account) || self.remembered("password", &a.email).is_some()
        }
    }

    /// Send/Receive without being asked: the next account of a round that is under way, or a new
    /// round when the app starts and every few minutes after (the owner's `syncMinutes`). A round
    /// that started by itself skips accounts it would have to ask about.
    fn auto_sync(&mut self) {
        // Something the owner did (a read mark, a move, a message to send) waits while a long run
        // is still bringing mail down: the run is called off, which loses nothing, and the next
        // one tells the server first.
        if self.auto_sync
            && self.sync_rx.is_some()
            && let Some((whose, Some(baseline), false)) = self.sync_run
            && now_ms() - self.push_checked >= 3_000.0
        {
            self.push_checked = now_ms();
            if self.waiting_to_go(whose) > baseline {
                self.services.sync_cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                self.sync_run = Some((whose, Some(baseline), true));
                self.session.sync_status = tl!("Sending your changes…").to_string();
            }
        }
        if self.services.sync.is_none() || self.sync_rx.is_some() || self.graph_rx.is_some() || self.signing_in.is_some() || self.dialog.is_some() {
            return;
        }
        if let Some((account, quiet)) = self.sync_queue.pop_front() {
            if !quiet || self.can_sync_quietly(account) {
                let _ = self.start_sync(&json!({"account": account, "quiet": quiet}));
            }
            return;
        }
        let minutes = f64::from(self.session.prefs.sync_minutes);
        let now = now_ms();
        // Something made here waits to go out (a message to send, a read mark, a move): it
        // doesn't wait for the next round. Looked at every few seconds; what didn't go through
        // last time is left for a few minutes unless there is more of it.
        if self.auto_sync && now - self.push_checked >= 1_500.0 {
            self.push_checked = now;
            for account in self.server_accounts() {
                let waiting = self.waiting_to_go(account);
                let (seen, at) = self.push_seen.get(&account).copied().unwrap_or((0, 0.0));
                if waiting > 0 && (waiting != seen || now - at > 300_000.0) && self.can_sync_quietly(account) {
                    self.sync_queue.push_back((account, true));
                    self.push_only_next.insert(account);
                }
            }
            if !self.sync_queue.is_empty() {
                return;
            }
        }
        if !self.auto_sync || minutes == 0.0 || (self.last_auto_sync != 0.0 && now - self.last_auto_sync < minutes * 60_000.0) {
            return;
        }
        self.last_auto_sync = now;
        self.sync_queue = self.server_accounts().into_iter().map(|a| (a, true)).collect();
    }

    /// Whether the owner still has to allow sending, calendar and contacts through Microsoft for
    /// `account` ("Connect calendar and contacts").
    pub fn graph_consent_needed(&self, account: Id) -> bool {
        self.graph_consent_needed.contains(&account)
    }

    /// Graph answered: keep its token, take in the calendar and contacts, file what it sent, and
    /// go on with the mail.
    fn poll_graph(&mut self) {
        let Some((account, quiet, directory, rx)) = &self.graph_rx else { return };
        let (account, quiet, directory) = (*account, *quiet, *directory);
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => GraphResult { account, ..Default::default() },
        };
        self.graph_rx = None;
        let address = self.session.mailbox.account(account).map(|a| a.email.clone()).unwrap_or_default();
        let mut errors = result.errors;
        if result.token_refused {
            self.graph_tokens.remove(&account);
        }
        if let Some(tokens) = result.tokens {
            // The newest refresh token is the account's, for mail and Graph alike.
            if !tokens.refresh.is_empty() {
                self.remember("refresh", &address, &tokens.refresh);
                if let Some(mail) = self.tokens.get_mut(&account) {
                    mail.refresh = tokens.refresh.clone();
                }
            }
            let now = mailcraft_time::now();
            self.graph_tokens.insert(account, Tokens { expires_at: tokens.expires_at.clamp(now, now.saturating_add(86_400)), ..tokens });
        }
        if result.consent_needed {
            if self.graph_consent_needed.insert(account) && !quiet {
                self.status(tl!("Calendar and contacts need your permission: File › Info › Connect calendar and contacts"));
            }
        } else {
            self.graph_consent_needed.remove(&account);
        }
        // What the server said to the calendar changes it was told, then what it has now.
        self.take_in_calendar_list(account, result.calendars.as_ref(), result.categories.as_ref(), result.extras_missing, &mut errors);
        let calendar_ok = self.take_in_calendar_of_sync(account, result.uploaded.as_ref(), result.events.as_ref(), &mut errors);
        self.take_in_more_calendars(&result.more_events, &mut errors);
        let mut contacts_ok = false;
        if let Some(contacts) = &result.contacts {
            match mailcraft_engine::cmd::remote::apply_contacts(&mut self.session, contacts) {
                Ok(_) => contacts_ok = true,
                Err(e) => errors.push(format!("{}: {e}", tl!("The contacts couldn't be taken in"))),
            }
        }
        if directory && calendar_ok && contacts_ok {
            self.graph_checked.insert(account, now_ms());
        }
        if !result.sent.is_empty() {
            let sent = mailcraft_model::sync::SyncResult { account, sent: result.sent.clone(), filed: result.sent.clone(), ..Default::default() };
            if let Err(e) = mailcraft_engine::cmd::account::apply_sync(&mut self.session, sent) {
                errors.push(e.to_string());
            }
        }
        self.note_sync_problems(account, &errors, false);
        let skip: Vec<Id> = result.sent.iter().chain(&result.held).copied().collect();
        if let Err(e) = self.start_sync(&json!({"account": account, "quiet": quiet, "afterGraph": true, "skip": skip})) {
            self.session.sync_status.clear();
            self.status(e);
        }
    }

    /// Whether the owner is being signed in on a provider's page.
    pub fn signing_in(&self) -> bool {
        self.signing_in.is_some()
    }

    /// The sign-in came back: keep the tokens for the session and carry on with the Send/Receive
    /// that asked for them; or say why not.
    fn poll_sign_in(&mut self) {
        let Some(waiting) = &self.signing_in else { return };
        let answer = match waiting.rx.try_recv() {
            Ok(answer) => answer,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(tl!("Signing in stopped unexpectedly").to_string()),
        };
        let (account, for_graph) = (waiting.account, waiting.graph);
        self.signing_in = None;
        // A day at most, whatever the provider said; a sign-in is asked for again after.
        let now = mailcraft_time::now();
        let answer = answer.and_then(|tokens| {
            if tokens.access.is_empty() {
                Err(tl!("Signing in brought nothing back").to_string())
            } else {
                Ok(Tokens { expires_at: tokens.expires_at.clamp(now, now.saturating_add(86_400)), ..tokens })
            }
        });
        if account == 0 {
            // For an account that is being set up: its dialog takes it from here.
            self.session.sync_status.clear();
            self.setup_sign_in = Some(answer);
            return;
        }
        let address = self.session.mailbox.account(account).map(|a| a.email.clone()).unwrap_or_default();
        if for_graph {
            // "Connect calendar and contacts": the token is Graph's; the next run uses it.
            match answer {
                // The owner picks who signs in on the page: it has to be this account.
                Ok(tokens) if !tokens.email.is_empty() && !tokens.email.trim().eq_ignore_ascii_case(address.trim()) => {
                    self.session.sync_status.clear();
                    self.status(format!("{} {} {} {address}", tl!("You signed in as"), tokens.email, tl!("but this account is")));
                }
                Ok(tokens) => {
                    if !tokens.refresh.is_empty() {
                        self.remember("refresh", &address, &tokens.refresh);
                        if let Some(mail) = self.tokens.get_mut(&account) {
                            mail.refresh = tokens.refresh.clone();
                        }
                    }
                    self.graph_tokens.insert(account, tokens);
                    self.graph_consent_needed.remove(&account);
                    self.graph_checked.remove(&account);
                    // Calendar changes that waited for this permission go now.
                    self.calendar_connected(account);
                    if let Err(e) = self.start_sync(&json!({"account": account})) {
                        self.session.sync_status.clear();
                        self.status(e);
                    }
                }
                Err(e) => {
                    self.session.sync_status.clear();
                    self.status(e);
                }
            }
            return;
        }
        match answer {
            Ok(tokens) => {
                self.remember("refresh", &address, &tokens.refresh);
                self.tokens.insert(account, tokens);
                if let Err(e) = self.start_sync(&json!({"account": account})) {
                    self.session.sync_status.clear();
                    self.status(e);
                }
            }
            Err(e) => {
                // What was kept stays: the host only gives up on it when the provider refuses it,
                // and a sign-in that works replaces it. The access token is spent either way.
                if let Some(tokens) = self.tokens.get_mut(&account) {
                    tokens.access.clear();
                }
                self.session.sync_status.clear();
                self.note_sync_problems(account, std::slice::from_ref(&e), false);
                self.status(e);
            }
        }
    }

    /// Whether the last Send/Receive of any account had problems.
    pub fn has_sync_problems(&self) -> bool {
        !self.sync_problems.is_empty()
    }

    /// What went wrong in the last Send/Receive, account by account, as lines to show and copy.
    pub(crate) fn sync_problem_lines(&self) -> Vec<String> {
        let mut out = Vec::new();
        for (_, name, at, lines) in &self.sync_problems {
            out.push(format!("{name} — {at}"));
            out.extend(lines.iter().map(|l| format!("  {l}")));
        }
        out
    }

    /// Keep what a Send/Receive said went wrong (and put it in the log); a run without problems
    /// clears what was kept for its account.
    fn note_sync_problems(&mut self, account: Id, errors: &[String], finished: bool) {
        if errors.is_empty() {
            if finished {
                self.sync_problems.retain(|(a, ..)| *a != account);
            }
            return;
        }
        for e in errors {
            log::warn!("send/receive, account {account}: {e}");
        }
        let name = self.session.mailbox.account(account).map(|a| a.name.clone()).unwrap_or_default();
        let at = self.session.local_now().time_short(self.session.prefs.twelve_hour);
        let clip = |e: &String| e.chars().take(600).collect::<String>();
        match self.sync_problems.iter_mut().find(|(a, ..)| *a == account) {
            Some((_, _, when, lines)) => {
                *when = at;
                lines.extend(errors.iter().map(clip));
                let extra = lines.len().saturating_sub(60);
                lines.drain(..extra);
            }
            None => self.sync_problems.push((account, name, at, errors.iter().take(60).map(clip).collect())),
        }
    }

    /// Whether a Send/Receive is running.
    pub fn syncing(&self) -> bool {
        self.sync_rx.is_some()
    }

    /// Take in what a Send/Receive has brought so far. It arrives in pieces (what was sent, what
    /// the server was told, then mail folder by folder, the Inbox first), each applied as it
    /// comes, so mail shows up while the rest is still on its way; the last piece ends the run.
    fn poll_sync(&mut self) {
        // A few pieces a frame: each one redraws the lists, and the window should stay quick.
        for _ in 0..4 {
            let Some(rx) = &self.sync_rx else { return };
            let piece = match rx.try_recv() {
                Ok(piece) => piece,
                Err(std::sync::mpsc::TryRecvError::Empty) => return,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.sync_rx = None;
                    self.session.sync_status = tl!("Send/Receive stopped unexpectedly").to_string();
                    return;
                }
            };
            let (account, last) = (piece.account, !piece.partial);
            let downloading = !piece.folders.is_empty();
            let refused = piece.errors.iter().any(|e| {
                let e = e.to_lowercase();
                e.contains("sign-in") || e.contains("sign in")
            });
            self.sync_refused |= refused;
            self.note_sync_problems(account, &piece.errors, false);
            match mailcraft_engine::cmd::account::apply_sync_part(&mut self.session, &mut self.sync_totals, piece) {
                Ok(v) => {
                    // New mail while the owner is elsewhere: the taskbar (or Dock) says so.
                    if v["new"].as_u64().unwrap_or(0) > 0
                        && let Some(ctx) = &self.ctx
                        && !ctx.input(|i| i.focused)
                    {
                        ctx.send_viewport_cmd(egui::ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
                    }
                }
                Err(e) => self.status(e.to_string()),
            }
            // Once it brings mail down, the run has told the server what there was to tell: what
            // waits beyond what waits at this moment was done here since.
            if downloading && let Some((whose, None, false)) = self.sync_run {
                self.sync_run = Some((whose, Some(self.waiting_to_go(whose)), false));
            }
            if !last {
                continue;
            }
            // The run is over.
            self.sync_rx = None;
            // Called off so that what the owner did could go out: the next run starts at once,
            // tells the server first, and carries on bringing mail down.
            if let Some((whose, _, true)) = self.sync_run.take() {
                self.sync_queue.push_front((whose, true));
            }
            let totals = std::mem::take(&mut self.sync_totals);
            let address = self.session.mailbox.account(account).map(|a| a.email.clone()).unwrap_or_default();
            if std::mem::take(&mut self.sync_refused) {
                // Forget the password so the next try asks again, and the access token so the
                // next try renews it (the refresh token stays for that).
                self.passwords.remove(&account);
                if let Some(store) = &self.services.secrets {
                    store.delete(&Self::secret_key("password", &address));
                }
                if let Some(tokens) = self.tokens.get_mut(&account) {
                    tokens.access.clear();
                }
            } else if totals.errors.is_empty() {
                if let Some(password) = self.passwords.get(&account) {
                    // It went through: this password is worth remembering.
                    self.remember("password", &address, password);
                }
                // And nothing is left of the last run's problems.
                self.note_sync_problems(account, &[], true);
            }
            self.status(totals.summary());
            self.push_seen.insert(account, (self.waiting_to_go(account), now_ms()));
            return;
        }
    }

    /// Open a file by path (the command line, a drop on the window, the system's Open With): a
    /// mailbox opens, a message, calendar or contact file is imported, anything else is attached
    /// to the message being written. A failure shows in the status bar.
    pub fn open_path(&mut self, path: &str) {
        files::dropped_path(self, path);
    }

    pub fn status(&mut self, s: impl Into<String>) {
        self.status_msg = Some((s.into(), now_ms()));
    }

    /// The name of what is in view, for the window title.
    pub fn title_stem(&self) -> String {
        use mailcraft_engine::Module;
        let account =
            self.session.mailbox.folder(self.session.view.folder).and_then(|f| self.session.mailbox.account(f.account)).map(|a| a.name.clone());
        match self.session.view.module {
            Module::Mail => {
                let folder = self.session.mailbox.folder(self.session.view.folder).map_or(tl!("Mail"), |f| f.name.as_str());
                match account.filter(|a| !a.is_empty()) {
                    Some(a) => format!("{folder} - {a}"),
                    None => folder.to_string(),
                }
            }
            m => tl!(m.label()).to_string(),
        }
    }

    /// Per-frame logic before layout: control requests, screenshots, background results, file
    /// drops, saving.
    pub fn logic(&mut self, ctx: &egui::Context) {
        i18n::set_current(i18n::Lang::from_pref(&self.ui.language));
        if !self.styled {
            theme::install_fonts(ctx);
            // Mod with -, = and 0 would also scale the whole window; interface size is in Options.
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
            self.styled = true;
        }
        if self.applied_theme != Some(self.ui.theme) {
            theme::apply(ctx, self.ui.theme);
            self.applied_theme = Some(self.ui.theme);
        }
        let zoom = self.ui.interface_size.zoom_factor();
        if ctx.zoom_factor() != zoom {
            ctx.set_zoom_factor(zoom);
        }
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
        self.drain_control(ctx);
        self.poll_sign_in();
        // A sign-in for an account that is being set up has nobody waiting for it once that dialog
        // is gone, however it went (a script closing it, say): the host is told to stop.
        if self.signing_in.as_ref().is_some_and(|s| s.account == 0) && !matches!(&self.dialog, Some(d) if d.name() == "account") {
            let _ = self.execute("ui.cancelSignIn", json!({}));
            self.session.sync_status.clear();
        }
        self.poll_graph();
        self.poll_calendar();
        self.poll_free_busy();
        self.poll_sync();
        self.auto_sync();
        // The calendar by itself, between Send/Receive runs ([`calendar_sync`]).
        self.calendar_tick(ctx);
        reminders::tick(self, ctx);
        if std::mem::take(&mut self.offer_setup) && self.dialog.is_none() && self.needs_setup() {
            self.dialog = dialogs::Dialog::open("account", self, &json!({}));
        }
        let _ = files::poll(self);
        files::drain_inbox(self);
        self.autosave_tick(ctx);
        self.collect_screenshots(ctx);
        self.issue_screenshots(ctx);
        if !self.synthetic.is_empty() || self.sync_rx.is_some() || self.graph_rx.is_some() || self.signing_in.is_some() || !self.sync_queue.is_empty()
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        } else if self.auto_sync && self.services.sync.is_some() && self.session.prefs.sync_minutes > 0 && !self.server_accounts().is_empty() {
            // Nothing is happening: look again in a while, in case a Send/Receive round is due or
            // something waits to go out.
            ctx.request_repaint_after(std::time::Duration::from_millis(1_500));
        }
        #[cfg(not(target_arch = "wasm32"))]
        for f in ctx.input(|i| i.raw.dropped_files.clone()) {
            self.open_path(&f.path().to_string_lossy());
        }
        self.report_dirty();
    }

    /// Tell the host whether there are unsaved changes ([`Services::on_dirty`]).
    fn report_dirty(&self) {
        if let Some(f) = &self.services.on_dirty {
            f(self.session.dirty && self.session.rev() != self.saved_rev);
        }
    }

    /// Save a moment after the last change: to the mailbox file on the desktop (through the
    /// host's background saver when it has one, so the window never waits for the disk), to the
    /// host's storage in the browser. A failure shows in the status bar and is tried again later.
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        self.collect_saves();
        if !self.autosave || self.session.rev() == self.saved_rev {
            return;
        }
        // From the first change on, whatever replaces the mailbox (File › Open, File › New)
        // writes what is unsaved first.
        if let (Some(saver), Some(path)) = (&self.services.saver, &self.session.path) {
            saver.follow(path);
        }
        let now = now_ms();
        if now - self.last_autosave < 3000.0 {
            ctx.request_repaint_after(std::time::Duration::from_millis(1000));
            return;
        }
        self.last_autosave = now;
        let rev = self.session.rev();
        let result = if let Some(persist) = &self.services.persist {
            mailcraft_engine::io::write_mailbox(&self.session.mailbox).and_then(|bytes| persist(&bytes))
        } else if let (Some(saver), Some(path)) = (&self.services.saver, &self.session.path) {
            // A snapshot costs next to nothing (the items are shared). Writing it is the saver's
            // business; `collect_saves` hears how it went. Handing in a state that is already on
            // its way changes nothing, so asking again until then is fine.
            saver.submit(path, self.session.mailbox.clone(), rev);
            ctx.request_repaint_after(std::time::Duration::from_millis(1000));
            return;
        } else if self.session.path.is_some() {
            self.session.run("file.save", &json!({})).map(|_| ()).map_err(|e| e.to_string())
        } else {
            return;
        };
        match result {
            Ok(()) => self.saved_rev = rev,
            Err(e) => {
                log::warn!("saving the mailbox failed: {e}");
                self.status(i18n::fmt(tl!("Couldn't save the mailbox: {error}"), &[("error", &e)]));
            }
        }
    }

    /// What the background saver got done since the last look ([`Services::saver`]).
    fn collect_saves(&mut self) {
        let reports = match &self.services.saver {
            Some(saver) => saver.reports(),
            None => return,
        };
        for report in reports {
            // About a mailbox that isn't the one open any more: nothing to tell the owner.
            if self.session.path.as_deref() != Some(report.path.as_path()) {
                if let Err(e) = &report.result {
                    log::warn!("saving {} failed: {e}", report.path.display());
                }
                continue;
            }
            match report.result {
                Ok(()) => {
                    self.saved_rev = self.saved_rev.max(report.rev);
                    if report.rev == self.session.rev() {
                        self.session.dirty = false;
                    }
                }
                Err(e) => {
                    log::warn!("saving the mailbox failed: {e}");
                    self.status(i18n::fmt(tl!("Couldn't save the mailbox: {error}"), &[("error", &e)]));
                    // Tried again in ten seconds rather than three: a full disk needs a moment.
                    self.last_autosave = now_ms() + 7000.0;
                }
            }
        }
    }

    /// Inject synthetic input events from the control channel (one pointer event per frame).
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        if self.synthetic.is_empty() {
            return;
        }
        let n = match self.synthetic.first() {
            Some(egui::Event::PointerMoved(_) | egui::Event::PointerButton { .. }) => 1,
            _ => self.synthetic.iter().position(|e| matches!(e, egui::Event::Key { pressed: false, .. })).map_or(self.synthetic.len(), |i| i + 1),
        };
        if let Some(egui::Event::PointerMoved(p) | egui::Event::PointerButton { pos: p, .. }) = self.synthetic.first() {
            raw.events.push(egui::Event::PointerMoved(*p));
        }
        let n = n.min(self.synthetic.len());
        raw.events.extend(self.synthetic.drain(..n));
    }

    /// Lay out the window.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let t0 = now_ms();
        let ctx = ui.ctx().clone();
        i18n::set_current(i18n::Lang::from_pref(&self.ui.language));
        // Fonts installed by `logic` take effect on the next frame.
        if self.fonts_frames < 2 {
            self.fonts_frames += 1;
            ctx.request_repaint();
            return;
        }
        let t = theme::Tokens::get(&ctx);
        self.hit.clear();
        // Windows of their own (a message, an appointment), before the shortcuts: Escape closes
        // the one in front where they are inside this window.
        windows::show(self, &ctx);
        // Shortcuts first, so the frame shows what the key did.
        keys::global_shortcuts(self, &ctx);
        if self.ui.backstage {
            backstage::show(self, ui);
        } else {
            // The module bar stands beside the tabs and the ribbon, from under the title bar down
            // to the status bar across the window's foot: both are laid out before the ribbon.
            chrome::title_bar(self, ui);
            chrome::status_bar(self, ui);
            nav::module_bar(self, ui);
            ribbon::show(self, ui);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(t.canvas)).show(ui, |ui| match self.session.view.module {
                mailcraft_engine::Module::Mail => mail_view::show(self, ui),
                mailcraft_engine::Module::Calendar => calendar_view::show(self, ui),
                mailcraft_engine::Module::People => people_view::show(self, ui),
                mailcraft_engine::Module::Tasks => tasks_view::show(self, ui),
            });
        }
        dialogs::show(self, &ctx);
        reminders::show(self, &ctx);
        let title = format!("{} - MailCraft", self.title_stem());
        if title != self.sent_title {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.sent_title = title;
        }
        self.frame_ms = now_ms() - t0;
        self.report_dirty();
    }

    /// The window's close button. MailCraft saves by itself, so closing only waits for a message
    /// being written: `false` keeps the window open and asks about it.
    pub fn close_requested(&mut self) -> bool {
        if self.session.compose.as_ref().is_some_and(|c| c.dirty) {
            self.dialog = dialogs::Dialog::open("discardDraft", self, &json!({"then": "quit"}));
            return false;
        }
        // An appointment window with unsaved changes asks about them first.
        if windows::hold_quit(self) {
            return false;
        }
        true
    }

    /// The window is going away: write the changes the timer hadn't got to yet, and wait for
    /// whatever is still being written. `Err` says why the mailbox couldn't be saved.
    pub fn save_on_exit(&mut self) -> Result<(), String> {
        let mut result = Ok(());
        if self.autosave && self.session.path.is_some() && (self.session.dirty || self.session.rev() != self.saved_rev) {
            // In place: this is the newest state, and an older one still waiting gives way to it.
            result = self.session.run("file.save", &json!({})).map(|_| ()).map_err(|e| e.to_string());
            if result.is_ok() {
                self.saved_rev = self.session.rev();
            }
        }
        if let Some(saver) = &self.services.saver {
            saver.flush();
        }
        result
    }

    fn drain_control(&mut self, ctx: &egui::Context) {
        let Some(rx) = self.control_rx.take() else { return };
        while let Ok(req) = rx.try_recv() {
            let reply = req.reply.clone();
            match control::handle(self, ctx, &req) {
                control::Outcome::Done(v) => {
                    let _ = reply.send(v);
                }
                control::Outcome::Screenshot { path } => {
                    self.shot_token += 1;
                    let token = self.shot_token;
                    self.queued_shots.push((token, now_ms() + 120.0, 0));
                    self.pending_shots.push((token, path, reply, now_ms() + 8000.0));
                }
            }
        }
        self.control_rx = Some(rx);
    }

    fn issue_screenshots(&mut self, ctx: &egui::Context) {
        let now = now_ms();
        self.queued_shots.retain_mut(|(token, at, frames)| {
            *frames += 1;
            if now >= *at && *frames >= 3 {
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::new(*token)));
                false
            } else {
                true
            }
        });
        if !self.queued_shots.is_empty() || !self.pending_shots.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(16));
        }
    }

    fn collect_screenshots(&mut self, ctx: &egui::Context) {
        if self.pending_shots.is_empty() {
            return;
        }
        let events: Vec<_> = ctx.input(|i| {
            i.raw
                .events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Screenshot { user_data, image, .. } => {
                        let token = user_data.data.as_ref().and_then(|d| d.downcast_ref::<u64>()).copied()?;
                        Some((token, image.clone()))
                    }
                    _ => None,
                })
                .collect()
        });
        for (token, image) in events {
            if let Some(i) = self.pending_shots.iter().position(|(t, ..)| *t == token) {
                let (_, path, reply, _) = self.pending_shots.remove(i);
                let _ = reply.send(control::save_screenshot(&image, path.as_deref()));
            }
        }
        let now = now_ms();
        self.pending_shots.retain(|(_, _, reply, deadline)| {
            if now < *deadline {
                return true;
            }
            let _ = reply.send(json!({"ok": false, "error": "no frame was presented (window hidden?)"}));
            false
        });
    }
}

/// What the status bar says after a command the user ran, when it is worth saying.
fn feedback(id: &str, v: &Value) -> Option<String> {
    let n = |k: &str| v.get(k).and_then(Value::as_u64);
    let plural = |n: u64, one: &str, many: &str| if n == 1 { format!("1 {one}") } else { format!("{n} {many}") };
    match id {
        "compose.send" if v.get("queued").and_then(Value::as_bool) == Some(true) => Some(tl!("In the Outbox: it is on its way").to_string()),
        // A mailbox whose content folder lost something still opens; say how much is missing.
        "file.open" => n("unavailable").filter(|c| *c > 0).map(|c| {
            format!("{} {}", plural(c, tl!("part of a message"), tl!("parts of messages")), tl!("couldn't be read from the mailbox's content folder"))
        }),
        "compose.send" => Some(tl!("Sent").to_string()),
        "compose.saveDraft" => Some(tl!("Draft saved").to_string()),
        "mail.markAllRead" | "rules.run" => n("changed").or(n("matched")).map(|c| plural(c, tl!("message changed"), tl!("messages changed"))),
        "file.import" => {
            let mut parts = Vec::new();
            for (key, one, many) in
                [("messages", "message", "messages"), ("events", "event", "events"), ("tasks", "task", "tasks"), ("contacts", "contact", "contacts")]
            {
                if let Some(c) = n(key).filter(|c| *c > 0) {
                    parts.push(plural(c, tl!(one), tl!(many)));
                }
            }
            (!parts.is_empty()).then(|| i18n::fmt(tl!("Imported {what}"), &[("what", &parts.join(", "))]))
        }
        "people.fromMessage" if v.get("existing").and_then(Value::as_bool) == Some(true) => Some(tl!("Already in your contacts").to_string()),
        "folder.empty" => n("removed").map(|c| plural(c, tl!("message removed"), tl!("messages removed"))),
        // An appointment that goes into another calendar of its account is made there first, and
        // taken out of the one it was in once that is done: said, as it takes a moment.
        "calendar.edit" if v.get("moved").and_then(Value::as_bool) == Some(true) => {
            Some(tl!("Moving it to the other calendar: the account's server makes it there, then takes it out of this one").to_string())
        }
        _ => None,
    }
}

/// Wall-clock milliseconds since the Unix epoch: the system clock, or the browser's on the web.
pub fn now_ms() -> f64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
    }
    // The std clock panics on wasm32-unknown-unknown, so ask the browser's.
    #[cfg(target_arch = "wasm32")]
    {
        let ms = js_sys::Date::now();
        if ms.is_finite() && ms > 0.0 { ms } else { 0.0 }
    }
}

#[cfg(test)]
mod tests;

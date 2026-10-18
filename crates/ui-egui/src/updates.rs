//! Telling the owner when a new version of MailCraft is out (File › About, File › Options, the
//! status bar), and, for a copy that updates itself, downloading it. Nothing is installed: the
//! owner is told, with a link to the release's page.
//!
//! - The daily check is off until the owner turns it on. At the first starts of a build that has
//!   it, a question above the status bar asks once a run, at most [`MOST_ASKS`] runs: Yes or No
//!   is final (File › Options changes it), and a run where it is left unanswered checks nothing.
//!   A development build never asks and never checks by itself.
//! - On, it checks at most once a day, through the host ([`Services::update_check`]), which asks
//!   GitHub alone and tells it nothing of the owner. The window does no I/O of its own.
//! - File › About's Check for Updates checks once, when pressed, on or off. An agent or a script
//!   may check only while the daily check is on (`ui.checkForUpdates`).
//! - A newer version is said once in the status bar, and stays in About while it is newer than
//!   the running build. A failure is one log line, said only when the owner pressed the button.
//! - A copy that updates itself (the Windows `.msi`, the macOS app, a Linux AppImage: the host
//!   says so by giving [`Services::update_download`]) downloads a newer release in the
//!   background, when the daily check (or the owner's press while it is on) finds one and File ›
//!   Options › Install updates automatically is on (as it is unless the owner turns it off; it
//!   needs the daily check). The host checks it by the release's own signature and deletes
//!   what doesn't pass; the owner is told only the notice, as for any other copy. Not for a
//!   pre-release, nor in a development build, nor when the organisation has turned it off
//!   ([`Services::updates_turned_off_by_organisation`]), nor for a check an agent or a script
//!   asked for. Cut off, it is carried on at the next start.
//! - Kept in `ui.json`: the settings, how often it was asked, when a check last got an answer,
//!   and the last version told of ([`UiState`]); nothing else of GitHub's answer.

use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};

use egui::{Ui, vec2};
use mailcraft_model::update::{Release, Version};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::theme::Tokens;
use crate::widgets::primary_button;
use crate::{MailApp, Services, UiState};

/// Most starts the question is put at, unanswered, before it is never put again.
pub const MOST_ASKS: u8 = 3;
/// How long after a check that got an answer the next is due (seconds).
const DAY: i64 = 86_400;
/// How long after a check that got none the next is tried (seconds).
const AFTER_A_FAILURE: i64 = 3_600;
/// The repository the build came from, whose releases are the ones told of.
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
/// The command that checks once.
pub const CHECK: &str = "ui.checkForUpdates";
/// The refusal of a check by an agent or a script while the daily check is off.
pub const NOT_ALLOWED: &str = "The owner hasn't allowed update checks.";

/// Whether MailCraft looks for new versions by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateCheck {
    /// The owner hasn't answered yet: off.
    #[default]
    NotAsked,
    On,
    Off,
}

impl UpdateCheck {
    pub fn code(self) -> &'static str {
        match self {
            UpdateCheck::NotAsked => "notAsked",
            UpdateCheck::On => "on",
            UpdateCheck::Off => "off",
        }
    }
}

/// The refusal of Install updates automatically while the organisation has it off.
pub const TURNED_OFF: &str = "Turned off by your organisation";

/// Who asked for a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Asker {
    /// The daily check, by itself.
    Itself,
    /// The owner, pressing Check for Updates.
    Owner,
    /// An agent or a script: what it finds is said, and never downloaded.
    Script,
}

/// What a run knows of new versions (not kept).
#[derive(Default)]
pub struct Updates {
    /// What is done at the start has been done.
    started: bool,
    /// The question is on screen.
    pub(crate) asking: bool,
    /// A check on its way, and who asked for it.
    pending: Option<(Receiver<Result<Release, String>>, Asker)>,
    /// A download on its way, of that version.
    downloading: Option<(Receiver<Result<PathBuf, String>>, Version)>,
    /// The version downloaded and checked this run.
    downloaded: Option<Version>,
    /// A check has been made at the start of this run to carry on a download of the version
    /// told of before (or there was none to carry on).
    resumed: bool,
    /// The newest release this run found, when it is newer than the running build.
    newer: Option<Release>,
    /// A check got an answer this run: what it found stands, whatever was told before.
    answered: bool,
    /// After a check that got no answer, the next one by itself is not before this (seconds).
    retry_at: i64,
    /// What the last check the owner pressed for came to, for About.
    said: Option<String>,
}

/// The version that is running.
fn running() -> Option<Version> {
    Version::parse(env!("CARGO_PKG_VERSION"))
}

/// Whether this build checks by itself and asks about it: it has a host that can ask, it is not
/// a development build, and its own version reads as one.
fn may_check_by_itself(services: &Services) -> bool {
    services.update_check.is_some() && !services.development_build && running().is_some()
}

/// The page of `version`'s release, made from its tag as this project writes them.
fn page_of(version: &Version) -> String {
    format!("{}/releases/tag/v{version}", REPOSITORY.trim_end_matches('/'))
}

/// What is said of a newer release, in the status bar and in About.
fn notice(release: &Release) -> String {
    let text =
        if release.security { tl!("MailCraft {version} is available and fixes a security problem") } else { tl!("MailCraft {version} is available") };
    crate::i18n::fmt(text, &[("version", &release.version.to_string())])
}

/// Saved settings made safe to use: a count beyond the most and a version that isn't one are
/// a hostile or broken file's, not the owner's.
pub(crate) fn sanitize(ui: &mut UiState) {
    ui.update_asked = ui.update_asked.min(MOST_ASKS);
    if Version::parse(&ui.update_told).is_none() {
        ui.update_told.clear();
    }
}

impl MailApp {
    /// The newest release known of that is newer than the running build: found this run, or
    /// told of in an earlier one.
    pub(crate) fn newer_release(&self) -> Option<Release> {
        let running = running()?;
        if self.updates.answered {
            return self.updates.newer.clone();
        }
        let told = Version::parse(&self.ui.update_told).filter(|told| told.is_newer_than(&running))?;
        Some(Release { page: page_of(&told), version: told, security: false, assets: Vec::new() })
    }

    /// Once, at the start of a run (after the settings are read): put the question, when it is
    /// still to be put.
    fn updates_at_start(&mut self) {
        if std::mem::replace(&mut self.updates.started, true) {
            return;
        }
        if may_check_by_itself(&self.services) && self.ui.update_check == UpdateCheck::NotAsked && self.ui.update_asked < MOST_ASKS {
            self.ui.update_asked = self.ui.update_asked.saturating_add(1);
            self.updates.asking = true;
        }
    }

    /// Whether this copy downloads a newer release by itself: it is one that updates itself, the
    /// daily check is on, so is Install updates automatically, the organisation hasn't turned it
    /// off, and this is not a development build.
    pub(crate) fn installs_updates(&self) -> bool {
        install_setting(self) == "on"
    }

    /// Every frame: the start's question, a check that is due, and the answer to one.
    pub(crate) fn updates_tick(&mut self, ctx: &egui::Context) {
        self.updates_at_start();
        self.poll_update_check();
        self.poll_update_download();
        if self.ui.update_check != UpdateCheck::On || !may_check_by_itself(&self.services) || self.updates.pending.is_some() {
            return;
        }
        let now = self.session.now();
        // A newer version told of in an earlier run, whose download is still to be had (it was
        // cut off, or the computer was turned off): checked again now, and carried on.
        let told_newer = Version::parse(&self.ui.update_told).is_some_and(|told| running().is_some_and(|r| told.is_newer_than(&r)));
        let carry_on = !std::mem::replace(&mut self.updates.resumed, true) && told_newer && self.installs_updates();
        let due = self.ui.update_checked.saturating_add(DAY).max(self.updates.retry_at);
        if now >= due || carry_on {
            self.begin_update_check(Asker::Itself);
        } else {
            // Looked at again when it is due (an hour at a time: a computer that slept wakes up late).
            let wait = u64::try_from(due.saturating_sub(now).clamp(1, AFTER_A_FAILURE)).unwrap_or(1);
            ctx.request_repaint_after(std::time::Duration::from_secs(wait));
        }
    }

    /// Hand a check to the host.
    fn begin_update_check(&mut self, asker: Asker) -> bool {
        let Some(check) = &self.services.update_check else { return false };
        let pre_releases = running().is_some_and(|v| v.is_pre_release());
        self.updates.pending = Some((check(pre_releases), asker));
        if asker == Asker::Owner {
            self.updates.said = Some(tl!("Checking for a new version…").to_string());
        }
        true
    }

    /// Hand the download of `release` to the host, when this copy takes updates by itself and it
    /// isn't downloaded or on its way already. Never for what an agent or a script asked for.
    fn begin_update_download(&mut self, release: &Release, asker: Asker) {
        let wanted = asker != Asker::Script
            && self.installs_updates()
            && !release.version.is_pre_release()
            && running().is_some_and(|running| release.version.is_newer_than(&running))
            && self.updates.downloaded.as_ref() != Some(&release.version)
            && self.updates.downloading.as_ref().is_none_or(|(_, v)| *v != release.version);
        let Some(download) = self.services.update_download.as_ref().filter(|_| wanted) else { return };
        self.updates.downloading = Some((download(release.clone()), release.version.clone()));
    }

    fn poll_update_download(&mut self) {
        let Some((rx, version)) = &self.updates.downloading else { return };
        let answer = match rx.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the download stopped unexpectedly".to_string()),
        };
        let version = version.clone();
        self.updates.downloading = None;
        // One line in the log, naming no address (the host's words name none); the owner is told
        // the notice and nothing more.
        match answer {
            Ok(_) => {
                log::info!("MailCraft {version} is downloaded and checked by its release signature");
                self.updates.downloaded = Some(version);
            }
            Err(why) => log::info!("MailCraft {version} wasn't downloaded: {why}"),
        }
    }

    fn poll_update_check(&mut self) {
        let Some((rx, asker)) = &self.updates.pending else { return };
        let asker = *asker;
        let pressed = asker == Asker::Owner;
        let answer = match rx.try_recv() {
            Ok(answer) => answer,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the check stopped unexpectedly".to_string()),
        };
        self.updates.pending = None;
        let now = self.session.now();
        match answer {
            Ok(release) => {
                self.ui.update_checked = now;
                self.updates.answered = true;
                let newer = running().is_some_and(|running| release.version.is_newer_than(&running));
                if newer {
                    let said = notice(&release);
                    let version = release.version.to_string();
                    // Once per version by itself; whenever the owner asks.
                    if pressed || self.ui.update_told != version {
                        self.status(said.clone());
                    }
                    self.ui.update_told = version;
                    self.updates.said = pressed.then_some(said);
                    self.begin_update_download(&release, asker);
                    self.updates.newer = Some(release);
                } else {
                    self.updates.newer = None;
                    if pressed {
                        let said = crate::i18n::fmt(tl!("MailCraft is up to date ({version})."), &[("version", env!("CARGO_PKG_VERSION"))]);
                        self.status(said.clone());
                        self.updates.said = Some(said);
                    }
                }
            }
            Err(why) => {
                // (The host's words name no address.)
                log::info!("couldn't find out whether a new version of MailCraft is out: {why}");
                self.updates.retry_at = now.saturating_add(AFTER_A_FAILURE);
                if pressed {
                    let said = crate::i18n::fmt(tl!("Couldn't find out whether a new version is out: {why}."), &[("why", &why)]);
                    self.status(said.clone());
                    self.updates.said = Some(said);
                } else {
                    self.updates.said = None;
                }
            }
        }
    }

    /// `ui.checkForUpdates`: check once. `pressed`: the owner pressed Check for Updates; else an
    /// agent or a script asks, which only the daily check being on allows.
    pub(crate) fn check_for_updates(&mut self, pressed: bool) -> Result<Value, String> {
        if !pressed && self.ui.update_check != UpdateCheck::On {
            return Err(tl!(NOT_ALLOWED).to_string());
        }
        let asker = if pressed { Asker::Owner } else { Asker::Script };
        if self.updates.pending.is_none() && !self.begin_update_check(asker) {
            return Err(tl!("This build doesn't check for new versions.").to_string());
        }
        Ok(json!({"pending": "updateCheck", "version": env!("CARGO_PKG_VERSION")}))
    }

    /// `ui.updateCheck`: the daily check on or off (the question's answer, File › Options).
    pub(crate) fn set_update_check(&mut self, p: &Value) -> Result<Value, String> {
        let on = match p.get("value") {
            None | Some(Value::Null) => self.ui.update_check != UpdateCheck::On,
            Some(value) => value.as_bool().ok_or_else(|| tl!("`value` is true or false").to_string())?,
        };
        self.ui.update_check = if on { UpdateCheck::On } else { UpdateCheck::Off };
        self.updates.asking = false;
        Ok(update_state(self))
    }

    /// `ui.updateInstall`: Install updates automatically on or off (File › Options).
    pub(crate) fn set_update_install(&mut self, p: &Value) -> Result<Value, String> {
        if self.services.updates_turned_off_by_organisation {
            return Err(tl!(TURNED_OFF).to_string());
        }
        if self.services.update_download.is_none() {
            return Err(tl!("This copy of MailCraft doesn't update itself.").to_string());
        }
        let on = match p.get("value") {
            None | Some(Value::Null) => !self.ui.update_install,
            Some(value) => value.as_bool().ok_or_else(|| tl!("`value` is true or false").to_string())?,
        };
        self.ui.update_install = on;
        Ok(update_state(self))
    }
}

/// Where Install updates automatically stands: `on` (a newer release is downloaded), `off` (the
/// owner turned it off), `needsCheck` (the daily check is off), `organisation` (turned off for
/// every user of the computer), or `notThisCopy` (a copy that doesn't update itself, or a
/// development build).
fn install_setting(app: &MailApp) -> &'static str {
    if app.services.update_download.is_none() || app.services.development_build || !may_check_by_itself(&app.services) {
        "notThisCopy"
    } else if app.services.updates_turned_off_by_organisation {
        "organisation"
    } else if app.ui.update_check != UpdateCheck::On {
        "needsCheck"
    } else if app.ui.update_install {
        "on"
    } else {
        "off"
    }
}

/// What `ui.updateCheck` answers: the setting, and what is known.
fn update_state(app: &MailApp) -> Value {
    let newer = app.newer_release();
    json!({
        "install": {
            "value": app.installs_updates(),
            "setting": install_setting(app),
            "downloading": app.updates.downloading.is_some(),
            "downloaded": app.updates.downloaded.as_ref().map(ToString::to_string),
        },
        "value": app.ui.update_check == UpdateCheck::On,
        "setting": app.ui.update_check.code(),
        "asked": app.ui.update_asked,
        "version": env!("CARGO_PKG_VERSION"),
        "newer": newer.as_ref().map(|r| json!({"version": r.version.to_string(), "security": r.security, "page": r.page})),
        "checking": app.updates.pending.is_some(),
    })
}

/// The question, across the window above the status bar, while it is on screen.
pub(crate) fn question(app: &mut MailApp, ui: &mut Ui) {
    if !app.updates.asking {
        return;
    }
    let t = Tokens::get(ui.ctx());
    egui::Panel::bottom("update_question")
        .exact_size(40.0)
        .show_separator_line(false)
        .frame(egui::Frame::NONE.fill(t.panel).stroke(egui::Stroke::new(1.0, t.border)).inner_margin(egui::Margin {
            left: 12,
            right: 12,
            top: 0,
            bottom: 0,
        }))
        .show(ui, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing = vec2(10.0, 0.0);
                crate::icons::paint(
                    ui.painter(),
                    egui::Rect::from_min_size(ui.cursor().min + vec2(0.0, 11.0), vec2(18.0, 18.0)),
                    "info",
                    t.icon,
                    t.accent,
                );
                ui.add_space(24.0);
                ui.label(
                    egui::RichText::new(tl!(
                        "Check once a day for new versions of MailCraft? Only GitHub is asked; nothing about you or your mail is sent."
                    ))
                    .color(t.text),
                );
                let yes = primary_button(ui, tl!("Yes"), true);
                let no = ui.add(egui::Button::new(tl!("No")).min_size(vec2(64.0, 26.0)));
                app.hit.add("updateYes", 0, yes.rect);
                app.hit.add("updateNo", 0, no.rect);
                if yes.clicked() {
                    let _ = app.run("ui.updateCheck", json!({"value": true}));
                } else if no.clicked() {
                    let _ = app.run("ui.updateCheck", json!({"value": false}));
                }
            });
        });
}

/// File › Options: the switch, where the build can check at all.
pub(crate) fn options(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    if app.services.update_check.is_none() {
        return;
    }
    let mut on = app.ui.update_check == UpdateCheck::On;
    let switch = ui.checkbox(&mut on, tl!("Check for new versions"));
    app.hit.add("updateOption", 0, switch.rect);
    if switch.changed() {
        let _ = app.run("ui.updateCheck", json!({"value": on}));
    }
    let setting = install_setting(app);
    let detail = if app.services.development_build {
        tl!("This is a development build: it checks only when you press Check for Updates in File › About.")
    } else if setting == "notThisCopy" {
        tl!("Once a day. Only GitHub is asked; nothing about you or your mail is sent. Nothing is downloaded or installed.")
    } else {
        tl!("Once a day. Only GitHub is asked; nothing about you or your mail is sent.")
    };
    ui.label(egui::RichText::new(detail).color(t.text_dim));
    if setting == "notThisCopy" {
        return;
    }
    // Install updates automatically: it needs the daily check, and the organisation's word wins.
    ui.add_space(4.0);
    let mut install = setting == "on";
    let enabled = matches!(setting, "on" | "off");
    let switch = ui.add_enabled(enabled, egui::Checkbox::new(&mut install, tl!("Install updates automatically")));
    app.hit.add("updateInstallOption", 0, switch.rect);
    if switch.changed() {
        let _ = app.run("ui.updateInstall", json!({"value": install}));
    }
    let (detail, color) = match setting {
        "organisation" => (tl!(TURNED_OFF), t.text),
        "needsCheck" => (tl!("Needs Check for new versions."), t.text_dim),
        _ => (
            tl!(
                "A new version is downloaded in the background and kept only when MailCraft's own release signature says it is ours. Nothing is installed yet."
            ),
            t.text_dim,
        ),
    };
    let said = ui.label(egui::RichText::new(detail).color(color));
    app.hit.add("updateInstallDetail", 0, said.rect);
}

/// File › About: the newer version, when there is one, and Check for Updates.
pub(crate) fn about(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    if app.services.update_check.is_none() {
        return;
    }
    if let Some(release) = app.newer_release() {
        let color = if release.security { t.red } else { t.accent_text };
        ui.label(egui::RichText::new(notice(&release)).font(crate::theme::semibold(13.0)).color(color));
        ui.horizontal(|ui| {
            ui.label(tl!("What's new and how to get it:"));
            let shown = release.page.trim_start_matches("https://");
            // Opened in the browser when clicked, and at no other time.
            if ui.link(shown).on_hover_text(&release.page).clicked() {
                ui.ctx().open_url(egui::OpenUrl::new_tab(release.page.clone()));
            }
        });
        if app.updates.downloaded.as_ref() == Some(&release.version) {
            let said = crate::i18n::fmt(tl!("MailCraft {version} is downloaded and checked."), &[("version", &release.version.to_string())]);
            ui.label(egui::RichText::new(said).color(t.text_dim));
        }
    }
    let checking = app.updates.pending.is_some();
    let label = if checking { tl!("Checking…") } else { tl!("Check for Updates") };
    let pressed = ui.add_enabled(!checking, egui::Button::new(label));
    app.hit.add("checkForUpdates", 0, pressed.rect);
    if pressed.clicked() {
        let _ = app.run(CHECK, json!({}));
    }
    if let Some(said) = app.updates.said.clone().filter(|_| !checking) {
        ui.label(egui::RichText::new(said).color(t.text_dim));
    }
    let how = if app.ui.update_check == UpdateCheck::On && !app.services.development_build {
        tl!("MailCraft looks for a new version once a day (File › Options). Only GitHub is asked.")
    } else {
        tl!("MailCraft doesn't look for new versions by itself; File › Options can have it look once a day.")
    };
    ui.label(egui::RichText::new(how).color(t.text_dim));
}

/// The window commands about new versions (`UI_COMMANDS` lists them). `ui.checkForUpdates` here
/// is a script's or an agent's; the owner's press goes through [`MailApp::run`].
pub(crate) fn ui_command(app: &mut MailApp, id: &str, p: &Value) -> Option<Result<Value, String>> {
    Some(match id {
        CHECK => app.check_for_updates(false),
        "ui.updateCheck" => app.set_update_check(p),
        "ui.updateInstall" => app.set_update_install(p),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use mailcraft_engine::Session;

    use super::*;

    /// Wednesday 2026-10-14 16:00 UTC.
    const NOW: i64 = 1_791_993_600;

    /// A stand-in host: every check answers what `answer` holds, and is counted.
    struct Host {
        answer: Arc<Mutex<Result<Release, String>>>,
        asked: Arc<AtomicUsize>,
    }

    fn release(version: &str, security: bool) -> Release {
        let version = Version::parse(version).unwrap();
        Release { page: page_of(&version), version, security, assets: Vec::new() }
    }

    fn stand_in(answer: Result<Release, String>) -> (Host, Services) {
        let host = Host { answer: Arc::new(Mutex::new(answer)), asked: Arc::new(AtomicUsize::new(0)) };
        let (answer, asked) = (host.answer.clone(), host.asked.clone());
        let services = Services {
            update_check: Some(Box::new(move |_| {
                asked.fetch_add(1, Ordering::Relaxed);
                let (tx, rx) = std::sync::mpsc::channel();
                let _ = tx.send(answer.lock().unwrap().clone());
                rx
            })),
            ..Default::default()
        };
        (host, services)
    }

    /// A start of the app at `now`, with the settings `saved` from the run before.
    fn start(services: Services, saved: &UiState, now: i64) -> MailApp {
        let mut app = MailApp::new(Session::at(mailcraft_model::Mailbox::with_local_account("", ""), now, 0), services);
        app.autosave = false;
        app.apply_prefs(saved.clone());
        app
    }

    fn frame(app: &mut MailApp) {
        app.updates_tick(&egui::Context::default());
    }

    fn said(app: &MailApp) -> Option<String> {
        app.status_msg.as_ref().map(|(text, _)| text.clone())
    }

    fn newer() -> String {
        let running = running().unwrap();
        format!("{}.{}.{}", running.major, running.minor.saturating_add(1), 0)
    }

    #[test]
    fn the_question_is_put_at_three_starts_at_most() {
        let mut saved = UiState::default();
        for start_number in 1..=5u8 {
            let (host, services) = stand_in(Ok(release(&newer(), false)));
            let mut app = start(services, &saved, NOW);
            frame(&mut app);
            assert_eq!(app.updates.asking, start_number <= MOST_ASKS, "start {start_number}");
            // Unanswered: nothing is checked in that run.
            for _ in 0..3 {
                frame(&mut app);
            }
            assert_eq!(host.asked.load(Ordering::Relaxed), 0);
            assert_eq!(app.ui.update_check, UpdateCheck::NotAsked);
            saved = app.prefs();
        }
        assert_eq!(saved.update_asked, MOST_ASKS);
    }

    #[test]
    fn an_answer_is_final() {
        // No: never asked again, nothing checked.
        let (host_no, services) = stand_in(Ok(release(&newer(), false)));
        let mut app = start(services, &UiState::default(), NOW);
        frame(&mut app);
        assert!(app.updates.asking);
        assert_eq!(app.run("ui.updateCheck", json!({"value": false})).unwrap()["setting"], "off");
        assert!(!app.updates.asking);
        frame(&mut app);
        let saved = app.prefs();
        let (_, services) = stand_in(Ok(release(&newer(), false)));
        let mut again = start(services, &saved, NOW + 2 * DAY);
        frame(&mut again);
        assert!(!again.updates.asking && again.ui.update_check == UpdateCheck::Off);
        assert_eq!(host_no.asked.load(Ordering::Relaxed), 0);

        // Yes: checked at once, and not asked again either.
        let (host_yes, services) = stand_in(Ok(release(&newer(), false)));
        let mut app = start(services, &UiState::default(), NOW);
        frame(&mut app);
        assert_eq!(app.run("ui.updateCheck", json!({"value": true})).unwrap()["value"], true);
        frame(&mut app);
        frame(&mut app);
        assert_eq!(host_yes.asked.load(Ordering::Relaxed), 1);
        let (_, services) = stand_in(Ok(release(&newer(), false)));
        let mut again = start(services, &app.prefs(), NOW + 60);
        frame(&mut again);
        assert!(!again.updates.asking);
    }

    #[test]
    fn once_a_day_and_once_per_version() {
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let mut saved = UiState { update_check: UpdateCheck::On, ..Default::default() };
        let mut app = start(services, &saved, NOW);
        frame(&mut app);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 1);
        assert_eq!(said(&app), Some(format!("MailCraft {} is available", newer())));
        assert_eq!(app.ui.update_told, newer());
        assert_eq!(app.ui.update_checked, NOW);
        assert_eq!(app.newer_release().unwrap().page, format!("https://github.com/UCS-Care-Group/mailcraft/releases/tag/v{}", newer()));
        saved = app.prefs();

        // Within the day: nobody is asked.
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let mut app = start(services, &saved, NOW + DAY - 1);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 0);
        // About still has it, from what was kept.
        assert_eq!(app.newer_release().map(|r| r.version.to_string()), Some(newer()));

        // A day on: asked again, but the same version isn't said twice.
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let mut app = start(services, &saved, NOW + DAY);
        frame(&mut app);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 1);
        assert_eq!(said(&app), None);

        // A newer one still, which fixes a security problem, is said.
        let running = running().unwrap();
        let next = format!("{}.{}.0", running.major, running.minor.saturating_add(2));
        *host.answer.lock().unwrap() = Ok(release(&next, true));
        app.ui.update_checked = 0;
        frame(&mut app);
        frame(&mut app);
        assert_eq!(said(&app), Some(format!("MailCraft {next} is available and fixes a security problem")));
    }

    #[test]
    fn nothing_newer_and_failures_are_said_only_when_pressed() {
        let here = env!("CARGO_PKG_VERSION");
        let (host, services) = stand_in(Ok(release(here, false)));
        let saved = UiState { update_check: UpdateCheck::On, ..Default::default() };
        let mut app = start(services, &saved, NOW);
        frame(&mut app);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 1);
        assert_eq!(said(&app), None);
        assert!(app.newer_release().is_none());
        // Pressed: said.
        assert_eq!(app.run(CHECK, json!({})).unwrap()["pending"], "updateCheck");
        frame(&mut app);
        assert_eq!(said(&app), Some(format!("MailCraft is up to date ({here}).")));

        // Failures: by itself, a line in the log and another try in an hour; pressed, said.
        *host.answer.lock().unwrap() = Err("GitHub couldn't be reached".into());
        app.status_msg = None;
        app.ui.update_checked = 0;
        frame(&mut app);
        frame(&mut app);
        assert_eq!(said(&app), None);
        assert_eq!(host.asked.load(Ordering::Relaxed), 3);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 3, "not again straight away");
        app.run(CHECK, json!({})).unwrap();
        frame(&mut app);
        assert_eq!(said(&app), Some("Couldn't find out whether a new version is out: GitHub couldn't be reached.".into()));
        // An older release is nothing newer.
        *host.answer.lock().unwrap() = Ok(release("0.0.1", false));
        app.run(CHECK, json!({})).unwrap();
        frame(&mut app);
        assert_eq!(said(&app), Some(format!("MailCraft is up to date ({here}).")));
        // What was told in an earlier run doesn't outlast an answer that has nothing newer.
        app.ui.update_told = newer();
        assert!(app.newer_release().is_none());
    }

    #[test]
    fn development_builds_never_ask_or_check_by_themselves() {
        let (host, mut services) = stand_in(Ok(release(&newer(), false)));
        services.development_build = true;
        let mut app = start(services, &UiState::default(), NOW);
        frame(&mut app);
        assert!(!app.updates.asking);
        app.ui.update_check = UpdateCheck::On;
        frame(&mut app);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 0);
        // Check for Updates still works.
        app.run(CHECK, json!({})).unwrap();
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 1);
        assert_eq!(said(&app), Some(format!("MailCraft {} is available", newer())));
    }

    #[test]
    fn agents_and_scripts_check_only_while_the_owner_has_it_on() {
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let mut app = start(services, &UiState { update_check: UpdateCheck::Off, ..Default::default() }, NOW);
        frame(&mut app);
        assert_eq!(app.execute(CHECK, json!({})), Err(NOT_ALLOWED.to_string()));
        app.ui.update_check = UpdateCheck::NotAsked;
        assert_eq!(app.execute(CHECK, json!({})), Err(NOT_ALLOWED.to_string()));
        assert_eq!(host.asked.load(Ordering::Relaxed), 0);
        // Through the agents' gate as the MCP bridge sends it: refused the same way.
        let (request, _answer) = crate::ControlRequest::new("engine.execute", json!({"command": CHECK, "params": {}}));
        let request = request.from_agent(Vec::new());
        let Some(crate::control::Outcome::Done(reply)) = crate::agent_gate::screen(&mut app, &request, &[]) else { panic!("settled by the gate") };
        assert_eq!(reply["ok"], false, "{reply}");
        assert!(reply["error"].as_str().unwrap().contains(NOT_ALLOWED), "{reply}");
        assert_eq!(host.asked.load(Ordering::Relaxed), 0);
        // On: an agent may.
        app.ui.update_check = UpdateCheck::On;
        assert_eq!(app.execute(CHECK, json!({})).unwrap()["pending"], "updateCheck");
        assert_eq!(host.asked.load(Ordering::Relaxed), 1);
        // Turning it on is a setting: the agents' gate asks the owner first.
        assert_eq!(
            mailcraft_engine::agent::access("ui.updateCheck"),
            Some(mailcraft_engine::agent::Access::Owner(mailcraft_engine::agent::Class::Settings))
        );
        // A build without a host has nothing to check with.
        let mut bare = start(Services::default(), &UiState::default(), NOW);
        assert!(bare.run(CHECK, json!({})).is_err());
        assert!(bare.execute("ui.updateCheck", json!({"value": "yes"})).is_err());
    }

    #[test]
    fn what_was_kept_is_made_safe() {
        let saved = UiState { update_asked: 250, update_told: "9.9.9\r\n<script>".into(), update_checked: i64::MAX, ..Default::default() };
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let mut app = start(services, &saved, NOW);
        assert_eq!(app.ui.update_asked, MOST_ASKS);
        assert!(app.ui.update_told.is_empty());
        frame(&mut app);
        assert!(!app.updates.asking);
        // A last check in the far future doesn't overflow anything.
        app.ui.update_check = UpdateCheck::On;
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 0);
        let json: UiState = serde_json::from_value(json!({"updateCheck": "sometimes", "updateAsked": -1})).unwrap_or_default();
        assert_eq!(json.update_check, UpdateCheck::NotAsked);
    }

    /// `services` with a stand-in download that answers `answer` and counts what it was asked for.
    fn downloading(mut services: Services, answer: Result<PathBuf, String>) -> (Services, Arc<Mutex<Vec<String>>>) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let kept = asked.clone();
        services.update_download = Some(Box::new(move |release: Release| {
            kept.lock().unwrap().push(release.version.to_string());
            let (tx, rx) = std::sync::mpsc::channel();
            let _ = tx.send(answer.clone());
            rx
        }));
        (services, asked)
    }

    fn on() -> UiState {
        UiState { update_check: UpdateCheck::On, ..Default::default() }
    }

    #[test]
    fn a_copy_that_updates_itself_downloads_a_newer_release() {
        let (_host, services) = stand_in(Ok(release(&newer(), false)));
        let (services, asked) = downloading(services, Ok(PathBuf::from("/updates/x")));
        let mut app = start(services, &on(), NOW);
        assert!(UiState::default().update_install, "on by default");
        assert_eq!(update_state(&app)["install"]["setting"], "on");
        for _ in 0..3 {
            frame(&mut app);
        }
        assert_eq!(*asked.lock().unwrap(), [newer()]);
        assert_eq!(app.updates.downloaded.as_ref().map(ToString::to_string), Some(newer()));
        // The owner is told the notice, as before.
        assert_eq!(said(&app), Some(format!("MailCraft {} is available", newer())));
        // Not twice in a run.
        app.run(CHECK, json!({})).unwrap();
        frame(&mut app);
        frame(&mut app);
        assert_eq!(asked.lock().unwrap().len(), 1);
        // The owner's press, while the daily check is on, downloads too.
        let (_host, services) = stand_in(Ok(release(&newer(), false)));
        let (services, asked) = downloading(services, Ok(PathBuf::from("/updates/x")));
        let mut app = start(services, &UiState { update_checked: NOW, ..on() }, NOW);
        app.updates.resumed = true;
        frame(&mut app);
        assert!(asked.lock().unwrap().is_empty());
        app.run(CHECK, json!({})).unwrap();
        frame(&mut app);
        frame(&mut app);
        assert_eq!(asked.lock().unwrap().len(), 1);
    }

    #[test]
    fn nothing_is_downloaded_unless_everything_allows_it() {
        let cases: Vec<(&str, Box<dyn Fn(&mut Services, &mut UiState)>)> = vec![
            ("the owner turned it off", Box::new(|_, ui| ui.update_install = false)),
            ("the daily check is off", Box::new(|_, ui| ui.update_check = UpdateCheck::Off)),
            ("the organisation turned it off", Box::new(|s, _| s.updates_turned_off_by_organisation = true)),
            ("a development build", Box::new(|s, _| s.development_build = true)),
        ];
        for (why, adjust) in cases {
            let (_host, services) = stand_in(Ok(release(&newer(), false)));
            let (mut services, asked) = downloading(services, Ok(PathBuf::new()));
            let mut saved = on();
            adjust(&mut services, &mut saved);
            let mut app = start(services, &saved, NOW);
            for _ in 0..3 {
                frame(&mut app);
            }
            app.run(CHECK, json!({})).unwrap();
            frame(&mut app);
            frame(&mut app);
            assert!(asked.lock().unwrap().is_empty(), "{why}");
            assert!(!app.installs_updates(), "{why}");
        }
        // A pre-release, and an older or the same version.
        for version in ["99.0.0-beta.1".to_string(), env!("CARGO_PKG_VERSION").to_string(), "0.0.1".to_string()] {
            let (_host, services) = stand_in(Ok(release(&version, false)));
            let (services, asked) = downloading(services, Ok(PathBuf::new()));
            let mut app = start(services, &on(), NOW);
            for _ in 0..3 {
                frame(&mut app);
            }
            assert!(asked.lock().unwrap().is_empty(), "{version}");
        }
        // A copy that doesn't update itself has no switch to turn on.
        let (_host, services) = stand_in(Ok(release(&newer(), false)));
        let mut app = start(services, &on(), NOW);
        assert_eq!(update_state(&app)["install"]["setting"], "notThisCopy");
        assert!(app.run("ui.updateInstall", json!({"value": true})).is_err());
    }

    #[test]
    fn agents_and_scripts_never_start_a_download() {
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let (services, asked) = downloading(services, Ok(PathBuf::new()));
        let mut app = start(services, &UiState { update_checked: NOW, ..on() }, NOW);
        app.updates.resumed = true;
        frame(&mut app);
        assert_eq!(app.execute(CHECK, json!({})).unwrap()["pending"], "updateCheck");
        frame(&mut app);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 1);
        assert!(app.newer_release().is_some());
        assert!(asked.lock().unwrap().is_empty());
        // The switch is a setting: the agents' gate asks the owner first.
        assert_eq!(
            mailcraft_engine::agent::access("ui.updateInstall"),
            Some(mailcraft_engine::agent::Access::Owner(mailcraft_engine::agent::Class::Settings))
        );
    }

    #[test]
    fn the_switch_and_the_organisations_word() {
        let (_host, services) = stand_in(Ok(release(&newer(), false)));
        let (services, _) = downloading(services, Ok(PathBuf::new()));
        let mut app = start(services, &on(), NOW);
        assert_eq!(app.run("ui.updateInstall", json!({"value": false})).unwrap()["install"]["setting"], "off");
        assert!(!app.prefs().update_install);
        assert_eq!(app.run("ui.updateInstall", json!({})).unwrap()["install"]["value"], true);
        assert!(app.run("ui.updateInstall", json!({"value": "yes"})).is_err());
        // Without the daily check it waits for it.
        app.run("ui.updateCheck", json!({"value": false})).unwrap();
        assert_eq!(update_state(&app)["install"]["setting"], "needsCheck");
        // The organisation's off-switch wins over the owner's.
        let (_host, mut services) = stand_in(Ok(release(&newer(), false)));
        services.updates_turned_off_by_organisation = true;
        let (services, _) = downloading(services, Ok(PathBuf::new()));
        let mut app = start(services, &on(), NOW);
        assert_eq!(update_state(&app)["install"]["setting"], "organisation");
        assert_eq!(app.run("ui.updateInstall", json!({"value": true})), Err(TURNED_OFF.to_string()));
        assert!(!app.installs_updates());
        // What was kept says nothing of the organisation: a file without the field is on.
        let kept: UiState = serde_json::from_value(json!({"updateCheck": "on"})).unwrap();
        assert!(kept.update_install);
    }

    #[test]
    fn a_download_cut_off_is_carried_on_at_the_next_start() {
        // The last run was told of a newer version within the day, and its download was cut off.
        let saved = UiState { update_checked: NOW - 60, update_told: newer(), ..on() };
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let (services, asked) = downloading(services, Err("the update couldn't be downloaded: GitHub took too long".into()));
        let mut app = start(services, &saved, NOW);
        for _ in 0..3 {
            frame(&mut app);
        }
        assert_eq!(host.asked.load(Ordering::Relaxed), 1, "checked at once, not a day on");
        assert_eq!(asked.lock().unwrap().len(), 1);
        assert!(app.updates.downloaded.is_none());
        // Failed: not again in this run until the next check is due, and nothing is said.
        for _ in 0..3 {
            frame(&mut app);
        }
        assert_eq!(host.asked.load(Ordering::Relaxed), 1);
        assert_eq!(said(&app), None);
        // Without anything told of before, a start within the day asks nobody.
        let (host, services) = stand_in(Ok(release(&newer(), false)));
        let (services, _) = downloading(services, Ok(PathBuf::new()));
        let mut app = start(services, &UiState { update_checked: NOW - 60, ..on() }, NOW);
        frame(&mut app);
        assert_eq!(host.asked.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn the_question_options_and_about_are_laid_out() {
        let (_host, services) = stand_in(Ok(release(&newer(), true)));
        let app = start(services, &UiState::default(), NOW);
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1200.0, 800.0)).build_ui_state(
            |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        harness.run_steps(4);
        assert!(harness.state().updates.asking);
        assert!(harness.state().hit.rect("updateYes", 0).is_some());
        harness.state_mut().run("ui.updateCheck", json!({"value": true})).unwrap();
        harness.run_steps(4);
        assert!(harness.state().newer_release().is_some_and(|r| r.security));
        for page in ["options", "about"] {
            harness.state_mut().run("ui.backstage", json!({"value": true, "page": page})).unwrap();
            harness.run_steps(4);
        }
        assert!(harness.state().hit.rect("checkForUpdates", 0).is_some());
    }
}

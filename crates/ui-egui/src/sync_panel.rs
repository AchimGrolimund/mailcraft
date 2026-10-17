//! Send/Receive Progress (#120): a panel in the main window, over its foot on the right, that
//! says what the Send/Receive under way is doing, or what the last one did, account by account:
//! the phase and counts of the one running, the ones waiting their turn, what each one that is
//! done came to, and the problems of each under its row. It opens only when the owner asks
//! (the Send/Receive text in the status bar, Send / Receive › Show Progress,
//! `ui.dialog {"name": "syncProgress"}`), and never by itself. It isn't modal: Escape or a click
//! anywhere else closes it. What it shows is the engine's ([`mailcraft_engine::cmd::sync_progress`])
//! with the problems the window keeps ([`MailApp::sync_problem_lines`]).
//!
//! Problems may name a message by its subject; messages being sent are only counted.

use egui::{Align2, Rect, RichText, Sense, Ui, pos2, vec2};
use mailcraft_engine::cmd::sync_progress::{AccountRun, RunState};
use mailcraft_model::Id;
use serde_json::json;

use crate::MailApp;
use crate::i18n;
use crate::theme::{Tokens, regular, semibold};

/// How wide the panel is, at most.
const WIDTH: f32 = 440.0;

/// The panel, while it is open.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Panel {
    /// It was opened for the problems: the first of them is scrolled to (once).
    pub to_problem: bool,
}

impl Panel {
    /// The panel a name of `ui.dialog` opens: `syncProgress`, or `syncProblems` (at the first
    /// problem).
    pub fn named(name: &str) -> Option<Panel> {
        match name {
            "syncProgress" => Some(Panel { to_problem: false }),
            "syncProblems" => Some(Panel { to_problem: true }),
            _ => None,
        }
    }
}

/// One account's row as the panel shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub account: Id,
    pub name: String,
    /// Where it is in the run; `None` for an account that has problems kept from a run that
    /// isn't the one shown (it never got started in it, say).
    pub run: Option<AccountRun>,
    pub problems: Vec<String>,
}

impl Row {
    /// What the row says under the account's name, in one line.
    pub fn line(&self) -> String {
        let count = |n: usize, one: &str, many: &str| i18n::fmt(if n == 1 { tl!(one) } else { tl!(many) }, &[("n", &n.to_string())]);
        let problems = || count(self.problems.len(), "{n} problem", "{n} problems");
        let Some(run) = &self.run else { return problems() };
        match run.state {
            RunState::Waiting => tl!("Waiting").to_string(),
            RunState::Running => {
                let Some(phase) = run.phase else { return tl!("Starting…").to_string() };
                let counted = match (run.done, run.total) {
                    (_, 0) if run.done == 0 => String::new(),
                    (done, 0) => done.to_string(),
                    (done, total) => i18n::fmt(tl!("{done} of {total}"), &[("done", &done.to_string()), ("total", &total.to_string())]),
                };
                let detail: Vec<&str> = [run.folder.as_str(), counted.as_str()].into_iter().filter(|s| !s.is_empty()).collect();
                if detail.is_empty() { tl!(phase.label()).to_string() } else { format!("{}: {}", tl!(phase.label()), detail.join(", ")) }
            }
            _ if !self.problems.is_empty() => problems(),
            RunState::Stopped => tl!("Stopped").to_string(),
            RunState::Done => {
                if run.new == 0 && run.sent == 0 {
                    return tl!("Done: up to date").to_string();
                }
                let mut line = i18n::fmt(tl!("Done: {n} new"), &[("n", &run.new.to_string())]);
                if run.sent > 0 {
                    line.push_str(&i18n::fmt(tl!(", {n} sent"), &[("n", &run.sent.to_string())]));
                }
                line
            }
        }
    }

    /// How far it has got, 0 to 1, when that is known (it is running and has a total).
    pub fn fraction(&self) -> Option<f32> {
        let run = self.run.as_ref().filter(|r| r.state == RunState::Running && r.total > 0)?;
        Some((run.done as f32 / run.total as f32).clamp(0.0, 1.0))
    }

    fn running(&self) -> bool {
        self.run.as_ref().is_some_and(|r| r.state == RunState::Running)
    }
}

/// The rows, in the run's order; then any account with problems that isn't in it.
pub(crate) fn rows(app: &MailApp) -> Vec<Row> {
    let mb = &app.session.mailbox;
    let problems = |account: Id| app.sync_problems.iter().find(|(a, ..)| *a == account).map(|(.., lines)| lines.clone()).unwrap_or_default();
    let mut rows: Vec<Row> = app
        .session
        .sync_progress
        .accounts
        .iter()
        .filter_map(|run| {
            let name = mb.account(run.account)?.name.clone();
            Some(Row { account: run.account, name, run: Some(run.clone()), problems: problems(run.account) })
        })
        .collect();
    for (account, name, _, lines) in &app.sync_problems {
        if !rows.iter().any(|r| r.account == *account) && !lines.is_empty() {
            let name = mb.account(*account).map_or_else(|| name.clone(), |a| a.name.clone());
            rows.push(Row { account: *account, name, run: None, problems: lines.clone() });
        }
    }
    rows
}

/// Escape closes the panel, before anything else hears it (a search, the selection).
pub(crate) fn escape(app: &mut MailApp, ctx: &egui::Context) {
    if app.sync_panel.is_some() && app.dialog.is_none() && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape)) {
        app.sync_panel = None;
    }
}

/// The panel, when it is open, over the foot of the window on the right.
pub(crate) fn show(app: &mut MailApp, ctx: &egui::Context) {
    let Some(panel) = app.sync_panel.clone() else { return };
    // File's pages cover the window; a dialog has the window's attention.
    if app.ui.backstage {
        app.sync_panel = None;
        return;
    }
    if app.dialog.is_some() {
        return;
    }
    let t = Tokens::get(ctx);
    let screen = ctx.content_rect();
    let width = WIDTH.min(screen.width() - 24.0).max(200.0);
    let rows = rows(app);
    let syncing = app.syncing();
    let mut stop = false;
    let mut close = false;
    let mut scrolled = false;
    let shown = egui::Area::new(egui::Id::new("sync_progress_panel"))
        .order(egui::Order::Foreground)
        .anchor(Align2::RIGHT_BOTTOM, vec2(-12.0, -32.0))
        .show(ctx, |ui| {
            egui::Frame::popup(ui.style()).fill(t.surface).stroke(egui::Stroke::new(1.0, t.border_strong)).inner_margin(12).show(ui, |ui| {
                ui.set_width(width);
                ui.spacing_mut().item_spacing = vec2(8.0, 4.0);
                // The heading, with Stop while a run goes on, and the way out.
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tl!("Send/Receive Progress")).font(semibold(14.0)).color(t.text));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let shut = crate::widgets::icon_button(ui, "close", tl!("Close"), 12.0, true);
                        app.hit.add("syncPanelClose", 0, shut.rect);
                        close |= shut.clicked();
                        if syncing {
                            let r = ui
                                .add(egui::Button::new(tl!("Stop")).min_size(vec2(60.0, 24.0)))
                                .on_hover_text(tl!("Stop this Send/Receive; what has arrived stays"));
                            app.hit.add("syncStop", 0, r.rect);
                            stop |= r.clicked();
                        }
                        if app.has_sync_problems() && ui.link(RichText::new(tl!("Copy problems")).font(regular(12.0))).clicked() {
                            ui.ctx().copy_text(app.sync_problem_lines().join("\n"));
                        }
                    });
                });
                ui.add_space(2.0);
                if rows.is_empty() {
                    let none = if app.server_accounts().is_empty() {
                        tl!("No account is connected to a mail server (File › Add Account)")
                    } else {
                        tl!("Nothing has been sent or received yet.")
                    };
                    ui.label(RichText::new(none).font(regular(12.5)).color(t.text_dim));
                    return;
                }
                let most = (screen.height() - 220.0).clamp(120.0, 520.0);
                egui::ScrollArea::vertical().max_height(most).auto_shrink([false, true]).show(ui, |ui| {
                    let mut said = 0u64;
                    for (n, row) in rows.iter().enumerate() {
                        if n > 0 {
                            ui.add_space(4.0);
                            let y = ui.cursor().min.y;
                            ui.painter().hline(ui.max_rect().x_range(), y, egui::Stroke::new(1.0, t.border));
                            ui.add_space(6.0);
                        }
                        let top = ui.cursor().min.y;
                        account_row(ui, &t, row);
                        for (k, problem) in row.problems.iter().enumerate() {
                            let r = problem_line(ui, &t, problem);
                            app.hit.add("syncProblem", said, r);
                            if said == 0 && k == 0 && panel.to_problem {
                                ui.scroll_to_rect(r, Some(egui::Align::TOP));
                                scrolled = true;
                            }
                            said += 1;
                        }
                        let rect = Rect::from_min_max(pos2(ui.max_rect().min.x, top), pos2(ui.max_rect().max.x, ui.cursor().min.y));
                        app.hit.add("syncRow", row.account, rect);
                    }
                });
            });
        });
    let area = shown.response.rect;
    app.hit.add("syncPanel", 0, area);
    // Scrolled to the first problem once; after that, it scrolls as the owner wants.
    if scrolled && let Some(open) = &mut app.sync_panel {
        open.to_problem = false;
    }
    if stop {
        let _ = app.run("ui.cancelSync", json!({}));
    }
    // A press anywhere else closes it; on what opens it, that does (a click there toggles it).
    let openers = [app.hit.rect("syncStatus", 0), app.hit.rect("syncProblemsLink", 0)];
    let away = ctx.input(|i| {
        i.pointer.any_pressed()
            && i.pointer.interact_pos().is_some_and(|pos| !area.contains(pos) && !openers.iter().flatten().any(|r| r.contains(pos)))
    });
    if close || away {
        app.sync_panel = None;
    }
    if syncing || rows.iter().any(Row::running) {
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }
}

/// An account's name and how it is doing: a bar when the total is known, else a spinner while
/// it runs.
fn account_row(ui: &mut Ui, t: &Tokens, row: &Row) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    // (Room on the right for what a waiting account can be told: Skip.)
    let room = r.width() - 90.0;
    crate::theme::paint_clipped(ui.painter(), pos2(r.min.x, r.center().y), room.max(40.0), &row.name, semibold(13.0), t.text);
    let line = row.line();
    let color = match &row.run {
        Some(run) if run.state == RunState::Running => t.text,
        _ if !row.problems.is_empty() => t.red,
        _ => t.text_dim,
    };
    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
    let mut x = r.min.x;
    if row.running() && row.fraction().is_none() {
        let spin = Rect::from_center_size(pos2(x + 7.0, r.center().y), vec2(12.0, 12.0));
        egui::Spinner::new().size(12.0).color(t.accent).paint_at(ui, spin);
        x += 20.0;
    }
    crate::theme::paint_clipped(ui.painter(), pos2(x, r.center().y), (r.max.x - x - 90.0).max(40.0), &line, regular(12.5), color);
    if row.run.as_ref().is_some_and(|run| run.kept) {
        resp.on_hover_text(tl!("What the Send/Receive before came to: the last one brought nothing new"));
    }
    if let Some(done) = row.fraction() {
        let (bar, _) = ui.allocate_exact_size(vec2(ui.available_width() - 90.0, 6.0), Sense::hover());
        ui.painter().rect_filled(bar, 3.0, t.border);
        let filled = Rect::from_min_size(bar.min, vec2((bar.width() * done).max(6.0), bar.height()));
        ui.painter().rect_filled(filled, 3.0, t.accent);
    }
}

/// A problem under its account's row, wrapped, with a red mark before it. Its rect.
fn problem_line(ui: &mut Ui, t: &Tokens, problem: &str) -> Rect {
    let response = ui
        .horizontal_top(|ui| {
            let (mark, _) = ui.allocate_exact_size(vec2(10.0, 16.0), Sense::hover());
            ui.painter().circle_filled(pos2(mark.center().x, mark.min.y + 8.0), 2.5, t.red);
            ui.add(egui::Label::new(RichText::new(problem).font(regular(12.0)).color(t.text)).selectable(true).wrap());
        })
        .response;
    response.rect
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex, mpsc};

    use egui_kittest::Harness;
    use mailcraft_engine::Session;
    use mailcraft_model::sync::{FolderDelta, SyncRequest, SyncResult, SyncStage, SyncStep};
    use mailcraft_model::{Address, FolderKind, Message};
    use serde_json::json;

    use super::*;
    use crate::Services;

    /// Wednesday 2026-10-14 16:00 UTC.
    const NOW: i64 = 1_791_993_600;

    /// The pipe the pieces of the run under way come through.
    type Pipe = Arc<Mutex<Option<mpsc::Sender<SyncResult>>>>;

    /// A mailbox with three accounts on mail servers (passwords at hand), and a host
    /// whose runs the test hands the pieces of.
    fn app() -> (MailApp, Vec<Id>, Pipe) {
        // (Not the sample: its reminders would come up over the window.)
        let mut a =
            MailApp::new(Session::at(mailcraft_model::Mailbox::with_local_account("Noor Haddad", "noor@example.org"), NOW, 0), Services::default());
        a.autosave = false;
        a.auto_sync = false;
        let pipe: Pipe = Default::default();
        let to = pipe.clone();
        a.services.sync = Some(Box::new(move |_: SyncRequest| {
            let (tx, rx) = mpsc::channel();
            if let Ok(mut pipe) = to.lock() {
                *pipe = Some(tx);
            }
            rx
        }));
        let mut ids = Vec::new();
        for who in ["avery", "blake", "casey"] {
            let added = a.execute("account.add", json!({"email": format!("{who}@mail.example"), "incoming": {"host": "imap.mail.example"}}));
            let id = added.unwrap()["id"].as_u64().unwrap();
            a.passwords.insert(id, "pw".into());
            ids.push(id);
        }
        (a, ids, pipe)
    }

    fn send(pipe: &Pipe, piece: SyncResult) {
        pipe.lock().unwrap().as_ref().unwrap().send(piece).unwrap();
    }

    /// New mail for the account's Inbox.
    fn mail(a: &MailApp, account: Id, uids: &[u32], partial: bool) -> SyncResult {
        let inbox = a.session.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
        let new = uids
            .iter()
            .map(|uid| Message {
                uid: *uid,
                subject: format!("Message {uid}"),
                from: Address::new("", "x@example.org"),
                date: NOW,
                ..Default::default()
            })
            .collect();
        let delta =
            FolderDelta { id: Some(inbox), remote: "INBOX".into(), kind: FolderKind::Inbox, uid_validity: 7, uid_seen: 9, new, ..Default::default() };
        SyncResult { account, partial, folders: vec![delta], ..Default::default() }
    }

    fn lines(a: &MailApp) -> Vec<String> {
        rows(a).iter().map(Row::line).collect()
    }

    fn window(app: MailApp, height: f32) -> Harness<'static, MailApp> {
        let mut harness = Harness::builder().with_size(egui::vec2(1280.0, height)).build_ui_state(
            |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
        harness.run_steps(3);
        harness
    }

    fn click_at(harness: &mut Harness<'static, MailApp>, pos: egui::Pos2) {
        harness.event(egui::Event::PointerMoved(pos));
        harness.step();
        for pressed in [true, false] {
            harness.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            harness.step();
        }
        harness.run_steps(2);
    }

    fn escape(harness: &mut Harness<'static, MailApp>) {
        for pressed in [true, false] {
            let key = egui::Key::Escape;
            harness.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: egui::Modifiers::NONE });
            harness.step();
        }
        harness.run_steps(2);
    }

    /// A round over three accounts is one run: the ones still to go wait, the one running says
    /// its phase and counts, each one done says what it came to, and Stop stops the rest.
    #[test]
    fn a_round_of_accounts_waits_runs_and_ends_row_by_row() {
        let (mut a, ids, pipe) = app();
        assert!(rows(&a).is_empty());
        a.execute("ui.sync", json!({})).unwrap();
        assert_eq!(rows(&a).iter().map(|r| r.account).collect::<Vec<_>>(), ids, "every account, in the run's order");
        assert_eq!(lines(&a), ["Waiting", "Waiting", "Waiting"]);

        // The first account: a folder's new mail, counted; a bar, as the total is known.
        let step = Some(SyncStep::new(SyncStage::Reading, "Inbox", 150, 500));
        send(&pipe, SyncResult { step, ..mail(&a, ids[0], &[1, 2], true) });
        a.poll_sync();
        let first = rows(&a).remove(0);
        assert_eq!((first.line(), first.fraction()), ("New mail: Inbox, 150 of 500".to_string(), Some(0.3)));
        // Sending is counted, never named; a step with no total yet has no bar.
        send(&pipe, SyncResult { account: ids[0], partial: true, step: Some(SyncStep::new(SyncStage::Sending, "", 1, 3)), ..Default::default() });
        a.poll_sync();
        assert_eq!(rows(&a)[0].line(), "Sending: 1 of 3");
        send(
            &pipe,
            SyncResult { account: ids[0], partial: true, step: Some(SyncStep::new(SyncStage::Checking, "Archive", 2, 5)), ..Default::default() },
        );
        a.poll_sync();
        assert_eq!((rows(&a)[0].line(), rows(&a)[0].fraction()), ("New mail: Archive".to_string(), None));
        send(&pipe, mail(&a, ids[0], &[3], false));
        a.poll_sync();
        assert_eq!(lines(&a), ["Done: 3 new", "Waiting", "Waiting"]);

        // The next one starts by itself; Stop ends it and the one still waiting.
        a.auto_sync();
        send(&pipe, SyncResult { account: ids[1], partial: true, step: Some(SyncStep::new(SyncStage::Connecting, "", 0, 0)), ..Default::default() });
        a.poll_sync();
        assert_eq!(lines(&a), ["Done: 3 new", "Folders", "Waiting"]);
        a.execute("ui.cancelSync", json!({})).unwrap();
        assert_eq!(lines(&a), ["Done: 3 new", "Folders", "Stopped"]);
        send(&pipe, SyncResult { account: ids[1], stopped: true, ..Default::default() });
        a.poll_sync();
        a.auto_sync();
        assert_eq!(lines(&a), ["Done: 3 new", "Stopped", "Stopped"]);
        assert!(!a.session.sync_progress.running() && !a.syncing());

        // A run whose account never starts (its password isn't at hand) leaves it out.
        a.passwords.remove(&ids[2]);
        a.execute("ui.sync", json!({"account": ids[0]})).unwrap();
        a.session.sync_progress.begin(&[ids[0], ids[2]]);
        a.sync_queue.push_back((ids[2], true));
        send(&pipe, mail(&a, ids[0], &[4], false));
        a.poll_sync();
        a.auto_sync();
        a.auto_sync();
        assert_eq!(rows(&a).iter().map(|r| r.account).collect::<Vec<_>>(), [ids[0]]);
        assert!(!a.session.sync_progress.running());
    }

    /// It opens from the status bar's Send/Receive text and from Send / Receive › Show Progress;
    /// Escape closes it, and so does a click anywhere else. It never opens by itself.
    #[test]
    fn the_panel_opens_when_asked_and_closes_with_escape_or_a_click_elsewhere() {
        let (mut a, ids, pipe) = app();
        a.execute("ui.sync", json!({"account": ids[0]})).unwrap();
        send(&pipe, SyncResult { step: Some(SyncStep::new(SyncStage::Reading, "Inbox", 2, 8)), ..mail(&a, ids[0], &[1, 2], true) });
        let mut h = window(a, 800.0);
        assert!(h.state().sync_panel.is_none(), "a run doesn't open it");
        let status = h.state().hit.rect("syncStatus", 0).expect("the status bar says what Send/Receive is doing");
        click_at(&mut h, status.center());
        assert!(h.state().sync_panel.is_some());
        let panel = h.state().hit.rect("syncPanel", 0).unwrap();
        assert!(h.state().hit.rect("syncStop", 0).is_some_and(|r| panel.contains(r.center())), "Stop, while the run goes on");
        assert!(h.state().hit.rect("syncRow", ids[0]).is_some());
        // A click on it doesn't close it; Escape does.
        click_at(&mut h, panel.center());
        assert!(h.state().sync_panel.is_some());
        escape(&mut h);
        assert!(h.state().sync_panel.is_none());
        // Stop in the panel is Cancel Send/Receive.
        h.state_mut().run("ui.dialog", json!({"name": "syncProgress"})).unwrap();
        h.run_steps(2);
        let stop = h.state().hit.rect("syncStop", 0).unwrap();
        click_at(&mut h, stop.center());
        assert!(h.state().services.sync_cancel.load(std::sync::atomic::Ordering::Relaxed));
        send(&pipe, SyncResult { account: ids[0], stopped: true, ..Default::default() });
        h.run_steps(3);
        assert!(h.state().hit.rect("syncStop", 0).is_none(), "nothing to stop once it has");
        assert_eq!(lines(h.state()), ["Stopped"]);

        // From the ribbon; then a click elsewhere in the window closes it.
        h.state_mut().sync_panel = None;
        h.state_mut().run("ui.tab", json!({"tab": "Send / Receive"})).unwrap();
        h.run_steps(2);
        let button = h.state().hit.rect("ribbon", crate::ribbon::hit_id("sync.progress")).expect("Show Progress");
        click_at(&mut h, button.center());
        assert!(h.state().sync_panel.is_some());
        click_at(&mut h, egui::pos2(400.0, 400.0));
        assert!(h.state().sync_panel.is_none());
        // A dialog opened takes its place.
        h.state_mut().run("ui.dialog", json!({"name": "syncProgress"})).unwrap();
        h.state_mut().run("ui.dialog", json!({"name": "goToDate"})).unwrap();
        assert!(h.state().sync_panel.is_none());
    }

    /// `syncProblems` opens the panel at the first problem, however far down its account is.
    #[test]
    fn sync_problems_opens_the_panel_at_the_first_problem() {
        let (mut a, _, _) = app();
        let mut ids = Vec::new();
        for n in 0..10 {
            let added = a.execute("account.add", json!({"email": format!("owner{n}@mail.example"), "incoming": {"host": "imap.mail.example"}}));
            ids.push(added.unwrap()["id"].as_u64().unwrap());
        }
        a.session.sync_progress.begin(&ids);
        for id in &ids {
            a.session.sync_progress.end(*id, false);
        }
        let last = *ids.last().unwrap();
        a.keep_sync_problems(last, &["Couldn't open \"Projects\": the server said no".to_string(), "Another one".to_string()]);
        assert_eq!(lines(&a).last().map(String::as_str), Some("2 problems"));
        a.execute("ui.dialog", json!({"name": "syncProblems"})).unwrap();
        let mut h = window(a, 600.0);
        h.run_steps(4);
        let panel = h.state().hit.rect("syncPanel", 0).unwrap();
        let first = h.state().hit.rect("syncProblem", 0).unwrap();
        assert!(panel.contains(first.center()), "the first problem is in view: {first:?} in {panel:?}");
        let top = h.state().hit.rect("syncRow", ids[0]).unwrap();
        assert!(!panel.contains(top.center()), "scrolled past the accounts above it");
        assert!(h.state().sync_panel.as_ref().is_some_and(|p| !p.to_problem), "once");
        // The red link in the status bar opens it there too.
        h.state_mut().sync_panel = None;
        h.run_steps(2);
        let link = h.state().hit.rect("syncProblemsLink", 0).expect("the red link stays");
        click_at(&mut h, link.center());
        assert!(h.state().sync_panel.is_some());
    }

    /// What a row says, whatever the numbers.
    #[test]
    fn a_row_says_where_it_is_in_one_line() {
        use mailcraft_model::sync::SyncPhase;
        let row = |run: AccountRun, problems: usize| Row { account: 1, name: "A".into(), run: Some(run), problems: vec!["x".into(); problems] };
        let running = |phase, folder: &str, done, total| AccountRun {
            state: RunState::Running,
            phase: Some(phase),
            folder: folder.into(),
            done,
            total,
            ..Default::default()
        };
        assert_eq!(row(running(SyncPhase::NewMail, "Inbox", 150, 500), 0).line(), "New mail: Inbox, 150 of 500");
        assert_eq!(row(running(SyncPhase::NewMail, "Inbox", 40, 0), 0).line(), "New mail: Inbox, 40");
        assert_eq!(row(running(SyncPhase::CalendarAndContacts, "", 0, 0), 0).line(), "Calendar and contacts");
        assert_eq!(row(AccountRun { state: RunState::Running, ..Default::default() }, 0).line(), "Starting…");
        // (A count past its total is a full bar, not more.)
        assert_eq!(row(running(SyncPhase::NewMail, "Inbox", u32::MAX, 3), 0).fraction(), Some(1.0));
        let done = |new, sent| AccountRun { state: RunState::Done, new, sent, ..Default::default() };
        assert_eq!(row(done(34, 2), 0).line(), "Done: 34 new, 2 sent");
        assert_eq!(row(done(1, 0), 0).line(), "Done: 1 new");
        assert_eq!(row(done(0, 0), 0).line(), "Done: up to date");
        assert_eq!(row(done(3, 0), 1).line(), "1 problem");
        assert_eq!(row(AccountRun { state: RunState::Stopped, new: 4, ..Default::default() }, 0).line(), "Stopped");
        assert_eq!(Row { account: 1, name: "A".into(), run: None, problems: vec!["x".into(); 2] }.line(), "2 problems");
    }
}

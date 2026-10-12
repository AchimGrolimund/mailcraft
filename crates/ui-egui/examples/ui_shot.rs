//! Headless UI screenshots: renders the whole MailCraft window offscreen (wgpu), so it works
//! with a locked screen or a hidden window.
//!
//! `cargo run -p mailcraft-ui-egui --example ui_shot -- script.jsonl`
//!
//! The script is JSON lines: control-channel requests (`{"method": …, "params": …}`, see
//! `docs/control-protocol.md`), `{"shot": "/abs/out.png"}` to save the window as PNG,
//! `{"steps": n}` to run extra frames, or `{"wait": ms}` to keep running frames for a while. The
//! window is 1440×900 pt at 2× (override with `{"width": w, "height": h}` and `{"scale": s}` on
//! any line; `{"scale": 1}` is a screen at 100 %, where the fonts' hinting shows) and opens the
//! sample mailbox, as of a fixed Wednesday afternoon so shots compare, unless the first line is
//! `{"empty": true}`. The computer is in UTC unless a line says where it is (`{"zone":
//! "America/Chicago"}`: the calendar is then shown in that zone, and `view.timeZone` can show it
//! in another).

use mailcraft_engine::Session;
use mailcraft_ui_egui::{ControlRequest, MailApp, Services};

static READY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

fn main() {
    let path = std::env::args().nth(1).expect("usage: ui_shot script.jsonl");
    let script = std::fs::read_to_string(&path).expect("read script");
    let (tx, rx) = std::sync::mpsc::channel::<ControlRequest>();
    let lines: Vec<serde_json::Value> =
        script.lines().filter(|l| !l.trim().is_empty()).map(|l| serde_json::from_str(l).expect("json line")).collect();
    // Where the computer is, when a line says: a zone with a name, so that it has daylight saving.
    let zone = lines.iter().find_map(|l| l.get("zone").and_then(|v| v.as_str())).and_then(mailcraft_time::Zone::named);
    let offset = zone.map_or(0, |zone| zone.offset_at(NOW));
    let mailbox = if lines.first().is_some_and(|l| l.get("empty").is_some()) {
        mailcraft_model::Mailbox::with_local_account("", "")
    } else {
        mailcraft_engine::sample::mailbox(NOW, offset)
    };
    let session = match zone {
        Some(zone) => Session::in_zone(mailbox, NOW, zone),
        None => Session::at(mailbox, NOW, 0),
    };
    let mut app = MailApp::new(session, Services::default()).with_control(rx);
    app.autosave = false;
    let w = lines.iter().find_map(|l| l.get("width").and_then(|v| v.as_f64())).unwrap_or(1440.0) as f32;
    let h = lines.iter().find_map(|l| l.get("height").and_then(|v| v.as_f64())).unwrap_or(900.0) as f32;
    let scale = lines.iter().find_map(|l| l.get("scale").and_then(|v| v.as_f64())).unwrap_or(2.0).clamp(0.5, 4.0) as f32;
    let mut harness =
        // Frames a sixtieth of a second apart, as on a screen: two clicks one frame after the other
        // then count as a double click (egui allows 0.3 s between them).
        egui_kittest::Harness::builder()
            .with_size(egui::vec2(w, h))
            .with_pixels_per_point(scale)
            .with_max_steps(1_000_000)
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_ui_state(
            |ui, app: &mut MailApp| {
                if !READY.load(std::sync::atomic::Ordering::Relaxed) {
                    return;
                }
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    harness.input_mut().max_texture_side = Some(8192);
    READY.store(true, std::sync::atomic::Ordering::Relaxed);
    step_n(&mut harness, 6);
    for l in lines {
        if let Some(p) = l.get("shot").and_then(|v| v.as_str()) {
            step_n(&mut harness, 4);
            match harness.render() {
                Ok(img) => {
                    img.save(p).expect("save png");
                    println!("{p}");
                }
                Err(e) => eprintln!("render failed: {e}"),
            }
        } else if let Some(n) = l.get("steps").and_then(|v| v.as_u64()) {
            step_n(&mut harness, n as usize);
        } else if let Some(ms) = l.get("wait").and_then(|v| v.as_u64()) {
            let end = std::time::Instant::now() + std::time::Duration::from_millis(ms);
            while std::time::Instant::now() < end {
                step(&mut harness);
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        } else if let Some(m) = l.get("method").and_then(|v| v.as_str()) {
            let (req, reply) = ControlRequest::new(m, l.get("params").cloned().unwrap_or_default());
            tx.send(req).expect("send");
            for _ in 0..60 {
                step(&mut harness);
                if let Ok(r) = reply.try_recv() {
                    let s = r.to_string();
                    // `UI_SHOT_FULL=1` prints whole replies (e.g. `ui.inspect` for scripted checks).
                    let cut = if std::env::var_os("UI_SHOT_FULL").is_some() { s.len() } else { s.len().min(300) };
                    println!("{m}: {}", s.get(..cut).unwrap_or(&s));
                    break;
                }
            }
            step_n(&mut harness, 2);
        }
    }
}

fn step(harness: &mut egui_kittest::Harness<'_, MailApp>) {
    let mut raw = std::mem::take(harness.input_mut());
    harness.state_mut().raw_input_hook(&mut raw);
    *harness.input_mut() = raw;
    harness.step();
}

fn step_n(harness: &mut egui_kittest::Harness<'_, MailApp>, n: usize) {
    for _ in 0..n {
        step(harness);
    }
}

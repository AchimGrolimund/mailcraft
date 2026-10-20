// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The desktop's light or dark appearance on Linux and the BSDs (#73), for File › Options ›
//! Theme › "Use system setting".
//!
//! winit 0.30 reports no theme on these systems, so the window would always be light. The
//! desktop says what it prefers through the XDG desktop portal's settings
//! (`org.freedesktop.portal.Settings`, the setting `org.freedesktop.appearance` `color-scheme`:
//! 1 prefers dark, 2 prefers light, 0 has no preference), which GNOME, KDE Plasma and the other
//! desktops' portals answer over the session's D-Bus. It is read once at start, on a thread of
//! its own and waited for a moment only, and then followed: a change on the desktop changes the
//! window, live. No portal, no answer or anything that goes wrong leaves winit's answer, which
//! is none, and the window is light, as before. The answer reaches the window through its input
//! (`egui::RawInput::system_theme`), where Windows and macOS have winit's.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use zbus::zvariant::{OwnedValue, Value};

/// The portal's service, object and interface.
const PORTAL: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const SETTINGS: &str = "org.freedesktop.portal.Settings";
/// The setting.
const NAMESPACE: &str = "org.freedesktop.appearance";
const KEY: &str = "color-scheme";

/// How long the window waits, as it starts, for the first answer: long enough for a portal
/// that is running, short enough that a missing or stuck one isn't noticed.
const START_WAIT: Duration = Duration::from_millis(300);
/// How long one call to the portal may take before it counts as no answer.
const CALL_TIMEOUT: Duration = Duration::from_secs(2);

/// The theme for a `color-scheme` value: dark for 1; light for 2, for 0 (no preference) and for
/// any value the portal's specification doesn't have.
pub fn theme_of(scheme: u32) -> egui::Theme {
    if scheme == 1 { egui::Theme::Dark } else { egui::Theme::Light }
}

/// The `color-scheme` number in an answer: `ReadOne` gives a variant holding the uint32, the
/// older `Read` one more variant around it. Another type, or variants nested deeper than that
/// and a little more, is not a number.
fn scheme_in(value: &Value<'_>) -> Option<u32> {
    let mut value = value;
    for _ in 0..4 {
        match value {
            Value::U32(n) => return Some(*n),
            Value::Value(inner) => value = inner,
            _ => return None,
        }
    }
    None
}

/// The theme an answer of the portal's means: one that can't be read counts as no preference.
pub fn theme_in(value: &Value<'_>) -> egui::Theme {
    scheme_in(value).map_or(egui::Theme::Light, theme_of)
}

/// What the desktop prefers, as far as it is known; shared between the window and the thread
/// that listens to the portal.
#[derive(Clone, Debug, Default)]
pub struct DesktopTheme(Arc<AtomicU8>);

/// The codes kept in [`DesktopTheme`]; 0 is not known yet.
const DARK: u8 = 1;
const LIGHT: u8 = 2;

impl DesktopTheme {
    /// What the desktop prefers: none until the portal has answered.
    pub fn get(&self) -> Option<egui::Theme> {
        match self.0.load(Ordering::Relaxed) {
            DARK => Some(egui::Theme::Dark),
            LIGHT => Some(egui::Theme::Light),
            _ => None,
        }
    }

    /// The desktop now prefers `theme`: whether that is a change.
    pub fn set(&self, theme: egui::Theme) -> bool {
        let code = match theme {
            egui::Theme::Dark => DARK,
            egui::Theme::Light => LIGHT,
        };
        self.0.swap(code, Ordering::Relaxed) != code
    }

    /// The window's input for a frame: the desktop's preference once the portal has said it;
    /// winit's answer (none, on these systems) until then.
    pub fn apply(&self, raw: &mut egui::RawInput) {
        if let Some(theme) = self.get() {
            raw.system_theme = Some(theme);
        }
    }
}

/// Ask the portal what the desktop prefers and follow it from now on, on a thread of its own;
/// the window is woken by every change. Waits [`START_WAIT`] at most for the first answer, so
/// that the window comes up in the right colours without waiting for a portal that isn't there.
pub fn follow(ctx: &egui::Context) -> DesktopTheme {
    let theme = DesktopTheme::default();
    let (first, answered) = std::sync::mpsc::channel();
    let (shared, wake) = (theme.clone(), ctx.clone());
    let spawned = std::thread::Builder::new().name("desktop theme".into()).spawn(move || {
        if let Err(e) = listen(&shared, &wake, &first) {
            log::debug!("the desktop's light or dark preference is not followed: {e}");
        }
    });
    match spawned {
        Ok(_) => {
            let _ = answered.recv_timeout(START_WAIT);
        }
        Err(e) => log::debug!("the desktop's light or dark preference is not asked for: {e}"),
    }
    theme
}

/// The listening thread: read the setting, say so through `first`, then take in every change
/// until the session's bus goes away.
fn listen(theme: &DesktopTheme, ctx: &egui::Context, first: &std::sync::mpsc::Sender<()>) -> zbus::Result<()> {
    let bus = zbus::blocking::connection::Builder::session()?.method_timeout(CALL_TIMEOUT).build()?;
    let portal: zbus::blocking::Proxy<'_> = zbus::blocking::proxy::Builder::new(&bus)
        .destination(PORTAL)?
        .path(PORTAL_PATH)?
        .interface(SETTINGS)?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()?;
    let take = |now: egui::Theme| {
        if theme.set(now) {
            ctx.request_repaint();
        }
    };
    // The first read also starts a portal that is started on demand.
    if let Some(now) = read(&portal) {
        take(now);
    }
    let _ = first.send(());
    let changes = portal.receive_signal_with_args("SettingChanged", &[(0, NAMESPACE), (1, KEY)])?;
    // A change between the first read and the listening isn't missed.
    if let Some(now) = read(&portal) {
        take(now);
    }
    for change in changes {
        let Ok((namespace, key, value)) = change.body().deserialize::<(String, String, OwnedValue)>() else {
            continue;
        };
        if namespace == NAMESPACE && key == KEY {
            take(theme_in(&value));
        }
    }
    Ok(())
}

/// The setting, read with `ReadOne` or, from a portal older than that, `Read`: none when
/// neither answers.
fn read(portal: &zbus::blocking::Proxy<'_>) -> Option<egui::Theme> {
    let answer =
        portal.call::<_, _, OwnedValue>("ReadOne", &(NAMESPACE, KEY)).or_else(|_| portal.call::<_, _, OwnedValue>("Read", &(NAMESPACE, KEY)));
    match answer {
        Ok(value) => Some(theme_in(&value)),
        Err(e) => {
            log::debug!("the portal didn't say whether the desktop prefers light or dark: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_scheme_values_map_to_themes() {
        assert_eq!(theme_of(1), egui::Theme::Dark);
        assert_eq!(theme_of(2), egui::Theme::Light);
        assert_eq!(theme_of(0), egui::Theme::Light, "no preference");
        for junk in [3, 7, 255, u32::MAX] {
            assert_eq!(theme_of(junk), egui::Theme::Light, "{junk}");
        }
    }

    #[test]
    fn answers_of_read_one_and_of_read_are_understood() {
        // ReadOne: v(u). Read: v(v(u)).
        assert_eq!(theme_in(&Value::U32(1)), egui::Theme::Dark);
        assert_eq!(theme_in(&Value::Value(Box::new(Value::U32(1)))), egui::Theme::Dark);
        assert_eq!(theme_in(&Value::Value(Box::new(Value::U32(2)))), egui::Theme::Light);
        assert_eq!(theme_in(&Value::Value(Box::new(Value::U32(0)))), egui::Theme::Light);
    }

    #[test]
    fn junk_answers_are_light() {
        let mut deep = Value::U32(1);
        for _ in 0..10 {
            deep = Value::Value(Box::new(deep));
        }
        for junk in [Value::Str("dark".into()), Value::I32(1), Value::U8(1), Value::Bool(true), Value::F64(1.0), deep] {
            assert_eq!(theme_in(&junk), egui::Theme::Light, "{junk:?}");
        }
    }

    #[test]
    fn the_window_has_the_desktops_theme_once_it_is_known() {
        let theme = DesktopTheme::default();
        let mut raw = egui::RawInput::default();
        theme.apply(&mut raw);
        assert_eq!(raw.system_theme, None, "winit's answer until the portal has said");
        assert!(theme.set(egui::Theme::Dark));
        assert!(!theme.set(egui::Theme::Dark), "the same again is no change");
        theme.apply(&mut raw);
        assert_eq!(raw.system_theme, Some(egui::Theme::Dark));
        assert!(theme.set(egui::Theme::Light));
        theme.apply(&mut raw);
        assert_eq!(raw.system_theme, Some(egui::Theme::Light));
        let mut from_winit = egui::RawInput { system_theme: Some(egui::Theme::Dark), ..Default::default() };
        DesktopTheme::default().apply(&mut from_winit);
        assert_eq!(from_winit.system_theme, Some(egui::Theme::Dark), "an unknown preference leaves winit's");
    }

    /// By hand, against this computer's own portal: `cargo test -p mailcraft desktop_theme -- --ignored --nocapture`.
    #[test]
    #[ignore = "asks this computer's desktop portal"]
    fn this_desktops_portal() {
        let ctx = egui::Context::default();
        let theme = follow(&ctx);
        std::thread::sleep(Duration::from_millis(500));
        println!("the desktop prefers {:?}", theme.get());
    }
}

//! Global show/hide shortcut.
//!
//! Once the widget is hidden, the only other route back is a tray icon that
//! Windows files into an overflow flyout by default — so this is the difference
//! between a widget you can summon and one you have to go looking for.
//!
//! Registration failure is surfaced rather than swallowed: another application
//! may already own the combination, and a shortcut that silently does nothing is
//! worse than one that says why.

use std::sync::Mutex;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::GlobalShortcutExt;

use crate::window;

/// Whether the shortcut registered at startup, so Settings can explain itself
/// rather than showing a switch that appears on but does nothing.
#[derive(Default)]
pub struct HotkeyStatus {
    pub error: Mutex<Option<String>>,
}

/// Chosen to be unlikely to collide: Ctrl+Shift+ a letter is rare in system
/// shortcuts, and G is not taken by Windows itself (Win+G is, but that is a
/// different modifier).
pub const DEFAULT_ACCELERATOR: &str = "CmdOrCtrl+Shift+G";

/// Applies the stored shortcut, replacing whatever was registered before.
///
/// Unregistering first matters: re-registering the same accelerator without
/// clearing it leaves a stale handler and the toggle fires twice, which reads as
/// the window flickering.
pub fn apply(app: &AppHandle, accelerator: Option<&str>) -> Result<(), String> {
    let shortcuts = app.global_shortcut();
    let _ = shortcuts.unregister_all();

    let record = |app: &AppHandle, message: Option<String>| {
        if let Some(status) = app.try_state::<HotkeyStatus>() {
            if let Ok(mut slot) = status.error.lock() {
                *slot = message;
            }
        }
    };

    let Some(accelerator) = accelerator else {
        log::info!("global shortcut disabled");
        record(app, None);
        return Ok(());
    };

    match shortcuts.register(accelerator) {
        Ok(()) => {
            log::info!("global shortcut registered");
            record(app, None);
            Ok(())
        }
        Err(err) => {
            log::warn!("could not register global shortcut: {err}");
            let message = format!(
                "Windows wouldn't give us {accelerator} — another app is \
                 probably using it. Try a different combination."
            );
            record(app, Some(message.clone()));
            Err(message)
        }
    }
}

/// Shows every note, or hides every note.
///
/// Operates on all of them rather than the main window alone: with several notes
/// on screen, a shortcut that summoned only one would leave the others behind
/// and make the gesture feel broken.
///
/// The rule is deliberately asymmetric — if *anything* is visible, the press
/// means "get them out of the way"; only when nothing is showing does it mean
/// "bring them back". Otherwise a partially-visible set toggles unpredictably.
pub fn on_pressed(app: &AppHandle) {
    let notes: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| crate::notes::is_note_label(label))
        .map(|(_, win)| win)
        .collect();

    if notes.is_empty() {
        return;
    }

    let any_visible = notes.iter().any(|w| w.is_visible().unwrap_or(false));

    if any_visible {
        log::info!("hotkey: hiding {} note(s)", notes.len());
        for win in &notes {
            window::hide(win);
        }
    } else {
        log::info!("hotkey: showing {} note(s)", notes.len());
        for win in &notes {
            let _ = win.show();
            let _ = win.unminimize();
        }
        // Focus one of them, so the set comes back ready to type into rather
        // than merely visible.
        if let Some(main) = window::main_window(app) {
            let _ = main.set_focus();
        } else if let Some(first) = notes.first() {
            let _ = first.set_focus();
        }
    }
}

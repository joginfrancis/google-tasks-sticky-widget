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

use crate::settings::{AppState, WindowLayer};
use crate::window;

/// Whether the shortcut registered at startup, so Settings can explain itself
/// rather than showing a switch that appears on but does nothing.
#[derive(Default)]
pub struct HotkeyStatus {
    pub error: Mutex<Option<String>>,
    /// Whether the last press hid the notes.
    ///
    /// Remembered rather than inferred from what is currently on screen,
    /// because a pinned note stays visible through a hide — so "is anything
    /// visible?" would answer yes and the next press would try to hide again.
    ///
    /// Unpinning that note leaves this flag alone, which is what makes the next
    /// press show everything, as it should.
    pub hidden: Mutex<bool>,
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

/// Shows every note, or hides every unpinned note.
///
/// Operates on all of them rather than the main window alone: with several notes
/// on screen, a shortcut that summoned only one would leave the others behind
/// and make the gesture feel broken.
///
/// Two rules, both of which matter:
///
/// 1. **Pinned notes are never hidden.** Pinning means "keep this in front", and
///    a shortcut that hid it anyway would make the pin meaningless.
/// 2. **The direction is remembered, not inferred.** Asking "is anything
///    visible?" gives the wrong answer once a pinned note is exempt — it stays
///    on screen through a hide, so the next press would try to hide again.
///    Tracking the last action instead means unpinning that note and pressing
///    the shortcut shows everything, which is what the user expects.
pub fn on_pressed(app: &AppHandle) {
    let notes: Vec<(String, tauri::WebviewWindow)> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| crate::notes::is_note_label(label))
        .collect();

    if notes.is_empty() {
        return;
    }

    let Some(status) = app.try_state::<HotkeyStatus>() else {
        return;
    };
    let Ok(mut hidden) = status.hidden.lock() else {
        return;
    };

    if *hidden {
        log::info!("hotkey: showing {} note(s)", notes.len());
        for (_, win) in &notes {
            let _ = win.show();
            let _ = win.unminimize();
        }
        // Focus one, so the set comes back ready to type into rather than
        // merely visible.
        if let Some(main) = window::main_window(app) {
            let _ = main.set_focus();
        } else if let Some((_, first)) = notes.first() {
            let _ = first.set_focus();
        }
        *hidden = false;
        return;
    }

    // A pinned note is exempt: pinning says "keep this in front", and a
    // shortcut that hid it anyway would make the pin meaningless.
    let pinned: Vec<String> = match app.state::<AppState>().settings.lock() {
        Ok(settings) => notes
            .iter()
            .map(|(label, _)| label.clone())
            .filter(|label| settings.layer_for(label) == WindowLayer::Top)
            .collect(),
        Err(_) => Vec::new(),
    };

    let mut hidden_count = 0;
    for (label, win) in &notes {
        if pinned.contains(label) {
            continue;
        }
        window::hide(win);
        hidden_count += 1;
    }

    log::info!(
        "hotkey: hid {hidden_count} note(s), {} left pinned in front",
        pinned.len()
    );
    *hidden = true;
}

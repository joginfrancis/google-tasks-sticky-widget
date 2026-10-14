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
/// Ignore a second press within this long — see `on_pressed`.
const REPEAT_GUARD_MS: u64 = 700;

fn millis_since_start() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

pub fn on_pressed(app: &AppHandle) {
    log::info!("hotkey: pressed");
    // Holding the combination for even a moment makes Windows repeat it, and
    // a toggle that fires twice is a toggle that does nothing: the notes were
    // shown and hidden again inside the same second, which looks exactly like
    // a shortcut that is not working. The log showed the pairs.
    static LAST_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let now = millis_since_start();
    let last = LAST_MS.load(std::sync::atomic::Ordering::Relaxed);
    if now.saturating_sub(last) < REPEAT_GUARD_MS {
        log::info!("hotkey: ignored a repeat {}ms after the last", now - last);
        return;
    }
    LAST_MS.store(now, std::sync::atomic::Ordering::Relaxed);

    let notes = crate::notes::note_windows(app);

    // Nothing on screen at all: the shortcut means "show my notes", so it puts
    // the arrangement back rather than doing nothing. Closing every note is
    // therefore recoverable without finding the tray.
    if notes.is_empty() {
        let known = crate::notes::known_list_ids(app);
        let opened = crate::notes::open_layout(app, &known);
        log::info!("hotkey: nothing was open; opened {opened} note(s)");
        if let Some(status) = app.try_state::<HotkeyStatus>() {
            if let Ok(mut hidden) = status.hidden.lock() {
                *hidden = false;
            }
        }
        return;
    }

    let Some(status) = app.try_state::<HotkeyStatus>() else {
        return;
    };
    let Ok(mut hidden) = status.hidden.lock() else {
        return;
    };

    let any_visible = notes
        .iter()
        .any(|(_, win)| win.is_visible().unwrap_or(false));

    if *hidden || !any_visible {
        log::info!("hotkey: showing {} note(s)", notes.len());
        for (_, win) in &notes {
            let _ = win.show();
            let _ = win.unminimize();
        }
        // Focus one, so the set comes back ready to type into rather than
        // merely visible.
        // Whichever note is there: `main` is no longer guaranteed to exist,
        // since every note can be closed.
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

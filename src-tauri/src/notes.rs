//! Note windows — one per task list.
//!
//! The registry exists so the rest of the app can answer two questions that
//! nothing else can: "which of these HWNDs are ours?" and "which list is this
//! window showing?". Cross-window drag depends on both
//! (docs/cross-window-drag-phase0.md §0.1), and so does polling the right set
//! of lists.
//!
//! Labels are sequential (`note-0`, `note-1`), not derived from the list, so
//! several notes can show the same list. The window-state plugin keys geometry
//! by label, so restoring a session reuses the saved labels — that is what puts
//! each note back where it was.

use std::collections::HashMap;
use std::sync::Mutex;

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::settings::{AppState, WindowLayer};

/// The window declared in tauri.conf.json. It shows whichever list is selected
/// rather than a fixed one, so it behaves as the "default note".
pub const MAIN_LABEL: &str = "main";

const NOTE_PREFIX: &str = "note-";

/// Which list each open note is showing.
///
/// Labels are sequential rather than derived from the list id, because several
/// notes may show the *same* list — the `+` button duplicates the current note
/// on purpose. The registry, not the label, is the authority on what a window
/// is displaying.
#[derive(Default)]
pub struct NoteRegistry {
    by_label: Mutex<HashMap<String, String>>,
    next_id: std::sync::atomic::AtomicU32,
}

impl NoteRegistry {
    pub fn set(&self, label: &str, list_id: &str) {
        if let Ok(mut map) = self.by_label.lock() {
            map.insert(label.to_string(), list_id.to_string());
        }
    }

    pub fn remove(&self, label: &str) {
        if let Ok(mut map) = self.by_label.lock() {
            map.remove(label);
        }
    }

    pub fn list_for(&self, label: &str) -> Option<String> {
        self.by_label.lock().ok()?.get(label).cloned()
    }

    /// Every list with a window open, deduplicated — what the scheduler polls.
    pub fn active_lists(&self) -> Vec<String> {
        let Ok(map) = self.by_label.lock() else {
            return Vec::new();
        };
        let mut lists: Vec<String> = map.values().cloned().collect();
        lists.sort();
        lists.dedup();
        lists
    }

    pub fn labels(&self) -> Vec<String> {
        self.by_label
            .lock()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

/// Always opens a *new* note, even for a list that already has one.
///
/// Duplicates are deliberate: the `+` button means "another note like this one",
/// and two views of the same list is a reasonable thing to want.
pub fn open_note(app: &AppHandle, list_id: &str, near: Option<&str>) -> Result<String, String> {
    open_note_labelled(app, list_id, near, None)
}

/// `label` is supplied only when restoring a saved session, so the window-state
/// plugin matches the geometry it stored for that note. New notes get the next
/// sequential label.
pub fn open_note_labelled(
    app: &AppHandle,
    list_id: &str,
    near: Option<&str>,
    label: Option<&str>,
) -> Result<String, String> {
    let registry = app.state::<NoteRegistry>();
    let label = match label {
        Some(explicit) => explicit.to_string(),
        None => {
            let n = registry
                .next_id
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            format!("{NOTE_PREFIX}{n}")
        }
    };

    log::info!("open_note: requested list={list_id} label={label} from={near:?}");

    // A new note starts at whatever the user last chose, then owns that setting
    // outright. Recording it now rather than leaving the entry empty is what
    // makes it independent: an unrecorded note falls back to the global default,
    // so pinning the main note would silently drag every never-pinned note with
    // it.
    let layer = {
        let state = app.state::<AppState>();
        let mut settings = match state.settings.lock() {
            Ok(settings) => settings,
            Err(_) => return Err("settings lock poisoned".to_string()),
        };
        let layer = settings.layer_for(&label);
        settings.set_layer_for(&label, layer);
        crate::settings::save(app, &settings);
        layer
    };

    let mut builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("Sticky Widget")
        .inner_size(340.0, 480.0)
        .min_inner_size(260.0, 200.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(true)
        .skip_taskbar(true)
        .always_on_top(layer == WindowLayer::Top);

    // Place it *beside* the window that asked, not on top of it.
    //
    // A small cascade offset looks like nothing happened: both notes are
    // always-on-top and the same size, so a 32px overlap leaves only a sliver
    // showing and the new note reads as missing.
    if let Some(origin) = near.and_then(|l| app.get_webview_window(l)) {
        if let (Ok(pos), Ok(size)) = (origin.outer_position(), origin.outer_size()) {
            let gap = 12i32;
            let width = size.width as i32;

            // Prefer to the right; fall back to the left; cascade only if
            // neither fits on this monitor.
            let (mon_left, mon_right) = origin
                .current_monitor()
                .ok()
                .flatten()
                .map(|m| {
                    let mp = m.position();
                    (mp.x, mp.x + m.size().width as i32)
                })
                .unwrap_or((i32::MIN, i32::MAX));

            let right = pos.x + width + gap;
            let left = pos.x - width - gap;

            let (x, y) = if right + width <= mon_right {
                (right, pos.y)
            } else if left >= mon_left {
                (left, pos.y)
            } else {
                (pos.x + 32, pos.y + 32)
            };

            log::info!(
                "open_note: placing at ({x},{y}); origin at ({},{}) size {}x{}; monitor x {mon_left}..{mon_right}",
                pos.x,
                pos.y,
                size.width,
                size.height
            );
            builder = builder.position(x as f64, y as f64);
        }
    } else {
        log::warn!("open_note: no origin window to place beside; using default position");
    }

    let window = builder.build().map_err(|e| {
        log::error!("open_note: window creation failed: {e}");
        format!("Could not open a new note: {e}")
    })?;

    // Without this the new note can sit behind the one that opened it — both
    // are always-on-top, so z-order between them is otherwise arbitrary.
    let _ = window.set_focus();

    log::info!("opened note window for list {list_id}");

    app.state::<NoteRegistry>().set(&label, list_id);
    persist_open_notes(app);

    // The scheduler polls only lists with a window open; a new note must start
    // syncing without waiting for the next tick.
    let sync = app.state::<crate::sync::SyncManager>();
    sync.refresh_lists(app);
    sync.wake();

    if layer == WindowLayer::Bottom {
        let _ = window.set_always_on_bottom(true);
    }

    Ok(label)
}

/// Distinguishes our note windows from anything else the app might own.
///
/// Used by the global shortcut to act on every note, and later by cross-window
/// drag to answer "is this HWND one of ours?" (docs/cross-window-drag-phase0.md).
pub fn is_note_label(label: &str) -> bool {
    label == MAIN_LABEL || label.starts_with(NOTE_PREFIX)
}

/// Writes the open-note set into settings so the next launch can rebuild it.
///
/// Called after every open, close, and list change rather than only at exit: a
/// crash or a force-kill would otherwise lose the arrangement, and this is a
/// few bytes of JSON.
pub fn persist_open_notes(app: &AppHandle) {
    let notes = match app.state::<NoteRegistry>().by_label.lock() {
        Ok(map) => map.clone(),
        Err(_) => return,
    };

    let state = app.state::<AppState>();
    let Ok(mut settings) = state.settings.lock() else {
        return;
    };
    settings.open_notes = notes;
    crate::settings::save(app, &settings);
}

/// Reopens the notes that were showing when the app last exited.
///
/// Runs during setup, before the event loop starts, which is why it can build
/// windows synchronously — the deadlock that makes `open_note_window` async
/// only applies to commands invoked while the loop is running.
///
/// A note whose list no longer exists is dropped rather than opened empty.
pub fn restore_session(app: &AppHandle, known_lists: &[String]) {
    let saved = match app.state::<AppState>().settings.lock() {
        Ok(settings) => settings.open_notes.clone(),
        Err(_) => return,
    };

    if saved.is_empty() {
        return;
    }

    // Continue the label sequence past anything restored, so a new note cannot
    // collide with a restored one and inherit its geometry.
    let highest = saved
        .keys()
        .filter_map(|l| l.strip_prefix(NOTE_PREFIX))
        .filter_map(|n| n.parse::<u32>().ok())
        .max();
    if let Some(highest) = highest {
        app.state::<NoteRegistry>()
            .next_id
            .store(highest + 1, std::sync::atomic::Ordering::Relaxed);
    }

    let mut restored = 0;
    for (label, list_id) in saved {
        if !known_lists.is_empty() && !known_lists.contains(&list_id) {
            log::info!("restore: skipping note for a list that no longer exists");
            continue;
        }
        match open_note_labelled(app, &list_id, None, Some(&label)) {
            Ok(_) => restored += 1,
            Err(err) => log::warn!("restore: could not reopen {label}: {err}"),
        }
    }

    log::info!("restore: reopened {restored} note(s)");
}

/* -- Commands ------------------------------------------------------------- */

/// Async on purpose, and it must stay that way.
///
/// A synchronous Tauri command runs **on the main thread**, and building a
/// window needs the main event loop to pump — which cannot happen while it is
/// blocked inside the command. The result is a hard deadlock: the window never
/// appears, the whole app stops responding, and logging stops mid-operation
/// with no error to explain it.
///
/// Marking it `async` moves it to a worker thread, leaving the event loop free
/// to service the window creation it dispatches.
#[tauri::command]
pub async fn open_note_window(
    app: AppHandle,
    window: tauri::Window,
    list_id: String,
) -> Result<String, String> {
    let label = window.label().to_string();
    open_note(&app, &list_id, Some(&label))
}

/// Which list the calling window shows.
///
/// `main` follows the persisted selection so it behaves as the default note;
/// every other window is pinned to the list in its own label.
#[tauri::command]
pub fn note_list_id(app: AppHandle, window: tauri::Window) -> Result<Option<String>, String> {
    let label = window.label().to_string();

    if label == MAIN_LABEL {
        return app
            .state::<AppState>()
            .settings
            .lock()
            .map(|s| s.selected_task_list_id.clone())
            .map_err(|_| "settings lock poisoned".to_string());
    }

    Ok(app.state::<NoteRegistry>().list_for(&label))
}

/// Closes the calling note. The main window hides instead — closing it would
/// leave the tray as the only way back, which is what `main` exists to avoid.
/// Async for the same reason as `open_note_window`: closing a window also goes
/// through the event loop, and blocking it from a command risks the same
/// deadlock.
#[tauri::command]
pub async fn close_note_window(app: AppHandle, window: tauri::Window) -> Result<(), String> {
    let label = window.label().to_string();

    if label == MAIN_LABEL {
        if let Some(win) = crate::window::main_window(&app) {
            crate::window::hide(&win);
        }
        return Ok(());
    }

    app.state::<NoteRegistry>().remove(&label);
    persist_open_notes(&app);
    let _ = window.close();

    let sync = app.state::<crate::sync::SyncManager>();
    sync.refresh_lists(&app);
    Ok(())
}

#[tauri::command]
pub fn open_note_labels(app: AppHandle) -> Vec<String> {
    app.state::<NoteRegistry>().labels()
}

/// Records which list the calling window is showing, and repoints the scheduler.
///
/// Replaces the old single-list `set_sync_list`, which set the *entire* polling
/// set and so would silently stop syncing every other open note. The scheduler
/// derives its list set from the registry instead, so it can only ever be the
/// union of what is actually on screen.
///
/// A note's label carries the list it was opened for, but the user can switch
/// lists inside it — the registry, not the label, is what the scheduler reads.
#[tauri::command]
pub fn register_note_list(
    app: AppHandle,
    window: tauri::Window,
    list_id: String,
) -> Result<(), String> {
    let label = window.label().to_string();

    if label != MAIN_LABEL {
        app.state::<NoteRegistry>().set(&label, &list_id);
        persist_open_notes(&app);
    }

    let sync = app.state::<crate::sync::SyncManager>();
    sync.refresh_lists(&app);
    sync.wake();
    Ok(())
}

/// Which note window sits under a point on the desktop, in physical pixels.
///
/// The answer has to come from here because a webview knows only its own
/// insides: it cannot see the note next to it, let alone where that note is on
/// screen. This is what lets a drag that leaves one note find another.
///
/// Only note windows are considered, so the desktop, other applications and any
/// non-note window of our own all fall out as `None` with nothing special
/// written for them.
///
/// Hit-testing by bounds rather than by Win32 `WindowFromPoint`: it needs no
/// native dependency, and notes are normally laid out side by side. The
/// difference shows only where two notes overlap, where this reports whichever
/// the registry lists first rather than whichever is on top.
#[tauri::command]
pub fn note_window_at(app: AppHandle, x: i32, y: i32) -> Option<String> {
    let mut labels = app.state::<NoteRegistry>().labels();
    labels.push(MAIN_LABEL.to_string());

    labels.into_iter().find(|label| {
        let Some(window) = app.get_webview_window(label) else {
            return false;
        };
        // A hidden note is not a drop target, and `main` in particular hides
        // rather than closes.
        if !window.is_visible().unwrap_or(false) {
            return false;
        }
        let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
            return false;
        };
        x >= pos.x
            && x < pos.x + size.width as i32
            && y >= pos.y
            && y < pos.y + size.height as i32
    })
}

/// Where a note window sits and how its pixels are scaled.
///
/// A drag converts its own client coordinates into physical screen ones, and the
/// note underneath converts them back. Both directions need this.
#[tauri::command]
pub fn note_window_frame(window: tauri::Window) -> Result<(i32, i32, f64), String> {
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    Ok((pos.x, pos.y, scale))
}

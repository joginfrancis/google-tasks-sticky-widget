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
use tauri_plugin_window_state::{AppHandleExt, StateFlags, WindowExt};

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
    /// When each note was last focused, as a monotonic tick.
    ///
    /// Stands in for z-order, which Tauri does not expose. It is only consulted
    /// where notes overlap, and there the note you touched most recently is
    /// almost always the one actually on top.
    focus_seq: Mutex<HashMap<String, u64>>,
    focus_clock: std::sync::atomic::AtomicU64,
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

    /// Records that a note came to the front.
    pub fn note_focused(&self, label: &str) {
        // `fetch_add` returns the value *before* the add, so the first note ever
        // focused would be stored as 0 — indistinguishable from never having
        // been focused, and it would lose the tie it just earned.
        let tick = self
            .focus_clock
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            + 1;
        if let Ok(mut seq) = self.focus_seq.lock() {
            seq.insert(label.to_string(), tick);
        }
    }

    /// How recently a note was focused; never focused sorts oldest.
    fn focus_rank(&self, label: &str) -> u64 {
        self.focus_seq
            .lock()
            .ok()
            .and_then(|seq| seq.get(label).copied())
            .unwrap_or(0)
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

    // Put the note back where it was last time.
    //
    // The window-state plugin restores windows declared in `tauri.conf.json`
    // by itself, but not windows built at run time — and every note is built
    // at run time now. Without this the saved position is written faithfully
    // on exit and then ignored on launch, which is why a restart stacked the
    // notes in a corner instead of rebuilding the arrangement.
    let _ = window.restore_state(StateFlags::POSITION | StateFlags::SIZE);

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
/// Set while a layout operation is moving windows itself.
///
/// Those moves raise the same `Moved`/`Resized` events a user drag does, and
/// answering every one of them with a disk write is what froze the app: five
/// notes being arranged produced hundreds of writes, each one asking the main
/// thread for every window's position while the main thread was busy moving
/// them.
static LAYOUT_BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// A move has happened that has not been written yet.
static GEOMETRY_DIRTY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

use std::sync::atomic::Ordering;

/// Notes that the arrangement has changed. The write happens later, once.
///
/// The plugin saves on exit, which is no help to a machine that is switched
/// off, a process that is killed, or an installer that replaces the binary
/// under a running app. So it is written while the app runs — but on a timer,
/// not on every pixel of a drag.
pub fn persist_geometry(_app: &AppHandle) {
    if LAYOUT_BUSY.load(Ordering::Relaxed) {
        return;
    }
    GEOMETRY_DIRTY.store(true, Ordering::Relaxed);
}

/// Writes the arrangement at most this often, and only when it has changed.
const GEOMETRY_SAVE_EVERY: std::time::Duration = std::time::Duration::from_secs(2);

/// Starts the one thread that writes window geometry.
pub fn start_geometry_writer(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(GEOMETRY_SAVE_EVERY);
        if GEOMETRY_DIRTY.swap(false, Ordering::Relaxed)
            && !LAYOUT_BUSY.load(Ordering::Relaxed)
        {
            let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);
        }
    });
}

/// Writes the arrangement right now — for the end of a layout operation.
fn save_geometry_now(app: &AppHandle) {
    GEOMETRY_DIRTY.store(false, Ordering::Relaxed);
    let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);
}

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

/// Every note window on screen, hidden or not.
pub fn note_windows(app: &AppHandle) -> Vec<(String, tauri::WebviewWindow)> {
    app.webview_windows()
        .into_iter()
        .filter(|(label, _)| is_note_label(label))
        .collect()
}

/// Opens the arrangement the user last had, or one note if there is none.
///
/// This is what "show my notes" means when nothing is on screen: the set that
/// was open before, minus anything already up, and a single note on the
/// default list when the arrangement is empty — a first run, or every note
/// having been closed.
///
/// Returns how many windows it opened.
pub fn open_layout(app: &AppHandle, known_lists: &[String]) -> usize {
    let saved = match app.state::<AppState>().settings.lock() {
        Ok(settings) => settings.open_notes.clone(),
        Err(_) => return 0,
    };

    // Labels continue past anything restored, so a new note cannot collide
    // with a restored one and inherit its geometry.
    if let Some(highest) = saved
        .keys()
        .filter_map(|l| l.strip_prefix(NOTE_PREFIX))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
    {
        app.state::<NoteRegistry>()
            .next_id
            .store(highest + 1, std::sync::atomic::Ordering::Relaxed);
    }

    let mut opened = 0;
    for (label, list_id) in saved {
        // Already on screen: show it rather than opening a second view of the
        // same list, which is how duplicate notes appeared.
        if let Some(existing) = app.get_webview_window(&label) {
            let _ = existing.show();
            continue;
        }
        if !known_lists.is_empty() && !known_lists.contains(&list_id) {
            log::info!("layout: skipping note for a list that no longer exists");
            continue;
        }
        match open_note_labelled(app, &list_id, None, Some(&label)) {
            Ok(_) => opened += 1,
            Err(err) => log::warn!("layout: could not open {label}: {err}"),
        }
    }

    if opened == 0 && note_windows(app).is_empty() {
        opened += open_default_note(app, known_lists);
    }

    log::info!("layout: opened {opened} note(s)");
    opened
}

/// One note on the list the user last chose, for when there is no arrangement
/// to restore. Takes the `main` label so it inherits whatever geometry that
/// note has had since the first run.
fn open_default_note(app: &AppHandle, known_lists: &[String]) -> usize {
    let chosen = {
        let state = app.state::<AppState>();
        let selected = state
            .settings
            .lock()
            .ok()
            .and_then(|s| s.selected_task_list_id.clone());

        match selected {
            Some(id) if known_lists.is_empty() || known_lists.contains(&id) => Some(id),
            _ => known_lists.first().cloned(),
        }
    };

    let Some(list_id) = chosen else {
        log::warn!("layout: no list to open a note on");
        return 0;
    };

    let label = if app.get_webview_window(MAIN_LABEL).is_none() {
        Some(MAIN_LABEL)
    } else {
        None
    };

    match open_note_labelled(app, &list_id, None, label) {
        Ok(_) => 1,
        Err(err) => {
            log::warn!("layout: could not open the default note: {err}");
            0
        }
    }
}

/// Reopens the notes that were showing when the app last exited.
///
/// Runs during setup, before the event loop starts, which is why it can build
/// windows synchronously — the deadlock that makes `open_note_window` async
/// only applies to commands invoked while the loop is running.
pub fn restore_session(app: &AppHandle, known_lists: &[String]) {
    open_layout(app, known_lists);
}

/// The list of lists this account has, for deciding what can be opened.
pub fn known_list_ids(app: &AppHandle) -> Vec<String> {
    app.try_state::<crate::store::Store>()
        .and_then(|store| store.task_lists().ok())
        .map(|lists| lists.into_iter().map(|l| l.id).collect())
        .unwrap_or_default()
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

    if let Some((_, list_id)) = page_target(window.clone()) {
        return Ok(Some(list_id));
    }

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
    // Every note closes the same way, `main` included: × takes a note out of
    // the arrangement. The list it was showing is untouched and can be opened
    // again from any note's list menu.
    let label = window.label().to_string();

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
/// native dependency, which matters on a machine where Smart App Control
/// already blocks unsigned build scripts.
///
/// Bounds alone cannot say which of two overlapping notes is in front, and the
/// registry is a `HashMap` whose iteration order Rust deliberately randomises —
/// so taking the first match would send the same drag to a different note each
/// time. Where several notes contain the point, the most recently focused one
/// wins: that is not the true z-order, but it is stable, and the note you
/// touched last is almost always the one on top.
#[tauri::command]
pub fn note_window_at(app: AppHandle, x: i32, y: i32) -> Option<String> {
    let registry = app.state::<NoteRegistry>();
    let mut labels = registry.labels();
    labels.push(MAIN_LABEL.to_string());

    let mut hits: Vec<String> = labels
        .into_iter()
        .filter(|label| {
            let Some(window) = app.get_webview_window(label) else {
                return false;
            };
            // A hidden note is not a drop target, and `main` in particular
            // hides rather than closes.
            if !window.is_visible().unwrap_or(false) {
                return false;
            }
            let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size())
            else {
                return false;
            };
            x >= pos.x
                && x < pos.x + size.width as i32
                && y >= pos.y
                && y < pos.y + size.height as i32
        })
        .collect();

    // Most recently focused first, then by label so the result never depends on
    // hash order even for two notes that have never been focused.
    hits.sort_by(|a, b| {
        registry
            .focus_rank(b)
            .cmp(&registry.focus_rank(a))
            .then_with(|| a.cmp(b))
    });

    hits.into_iter().next()
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

/// Geometry a note had before it was stretched to full height, by label.
static TALL_RESTORE: Mutex<Option<HashMap<String, (tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>)>>> =
    Mutex::new(None);

/// Stretches a note to the full height of its monitor's work area while a long
/// description is being read or edited, and puts it back afterwards.
///
/// The top edge stays where the user put it whenever the note can grow
/// downwards from there. A window that jumps to the top of the screen every
/// time a task is opened stops feeling like a note left somewhere on purpose —
/// so moving it is a last resort, taken only when the remaining height below
/// is not worth having.
#[tauri::command]
pub fn note_set_tall(window: tauri::Window, tall: bool) -> Result<(), String> {
    let label = window.label().to_string();

    if tall {
        {
            let guard = TALL_RESTORE.lock().map_err(|e| e.to_string())?;
            if guard.as_ref().is_some_and(|m| m.contains_key(&label)) {
                return Ok(());
            }
        }

        let pos = window.outer_position().map_err(|e| e.to_string())?;
        let size = window.outer_size().map_err(|e| e.to_string())?;
        let Some(monitor) = window.current_monitor().map_err(|e| e.to_string())? else {
            return Ok(());
        };
        let area = monitor.work_area();
        if size.height >= area.size.height {
            return Ok(());
        }

        let bottom = area.position.y + area.size.height as i32;
        let room_below = (bottom - pos.y).max(0) as u32;

        // Keep the top edge if two thirds of the screen is available below it;
        // that is plenty for a description, and staying put is worth more than
        // the last few hundred pixels.
        let (y, height) = if room_below * 3 >= area.size.height * 2 {
            (pos.y, room_below)
        } else {
            (area.position.y, area.size.height)
        };

        if height <= size.height {
            return Ok(());
        }

        TALL_RESTORE
            .lock()
            .map_err(|e| e.to_string())?
            .get_or_insert_with(HashMap::new)
            .insert(label, (pos, size));

        animate(&window, pos, size, tauri::PhysicalPosition::new(pos.x, y), tauri::PhysicalSize::new(size.width, height));
    } else {
        let saved = TALL_RESTORE
            .lock()
            .map_err(|e| e.to_string())?
            .as_mut()
            .and_then(|m| m.remove(&label));

        if let Some((pos, size)) = saved {
            let from_pos = window.outer_position().map_err(|e| e.to_string())?;
            let from_size = window.outer_size().map_err(|e| e.to_string())?;
            animate(&window, from_pos, from_size, pos, size);
        }
    }
    Ok(())
}

/// How long a note takes to change size. Long enough to be followed by eye,
/// short enough that nobody waits for it.
const ANIMATION_MS: u64 = 180;
const FRAMES: u64 = 12;

/// Moves and resizes a window over a fifth of a second rather than in one jump.
///
/// A note that snaps to a new size reads as a glitch — the eye cannot tell
/// whether the window moved or was replaced. Eased, the same change reads as
/// the note making room for itself.
fn animate(
    window: &tauri::Window,
    from_pos: tauri::PhysicalPosition<i32>,
    from_size: tauri::PhysicalSize<u32>,
    to_pos: tauri::PhysicalPosition<i32>,
    to_size: tauri::PhysicalSize<u32>,
) {
    let window = window.clone();
    std::thread::spawn(move || {
        for frame in 1..=FRAMES {
            // Ease-out: most of the movement happens early, which is what makes
            // it feel like a response rather than a transition.
            let t = frame as f64 / FRAMES as f64;
            let eased = 1.0 - (1.0 - t).powi(3);

            let lerp_i = |a: i32, b: i32| a + ((b - a) as f64 * eased).round() as i32;
            let lerp_u = |a: u32, b: u32| {
                (a as f64 + (b as f64 - a as f64) * eased).round().max(1.0) as u32
            };

            let _ = window.set_size(tauri::PhysicalSize::new(
                lerp_u(from_size.width, to_size.width),
                lerp_u(from_size.height, to_size.height),
            ));
            let _ = window.set_position(tauri::PhysicalPosition::new(
                lerp_i(from_pos.x, to_pos.x),
                lerp_i(from_pos.y, to_pos.y),
            ));

            std::thread::sleep(std::time::Duration::from_millis(ANIMATION_MS / FRAMES));
        }
        // Exactly the target, whatever the rounding did on the way.
        let _ = window.set_size(to_size);
        let _ = window.set_position(to_pos);
    });
}

/// Puts every stretched note back to its own size.
///
/// Called before the app exits, because the window-state plugin saves whatever
/// size a window has at that moment — and a note that was left tall for a long
/// description would otherwise reopen tall for ever after.
pub fn restore_all_sizes(app: &AppHandle) {
    let Ok(mut guard) = TALL_RESTORE.lock() else {
        return;
    };
    let Some(saved) = guard.as_mut() else {
        return;
    };

    for (label, (pos, size)) in saved.drain() {
        if let Some(window) = app.get_webview_window(&label) {
            let _ = window.set_size(size);
            let _ = window.set_position(pos);
        }
    }
}

/// Makes a note taller by `extra` physical pixels, so something that opened
/// inside it — a menu, a date picker — is not cut off by the window edge.
///
/// Unlike `note_set_tall` this takes only what is asked for: a six-item menu
/// needs a hundred pixels, and throwing the window to full screen height for
/// it would be a bigger surprise than the clipping it fixes. The original
/// geometry is remembered in the same place, so `note_set_tall(false)` is what
/// puts either kind of growth back.
#[tauri::command]
pub fn note_grow(window: tauri::Window, extra: u32) -> Result<(), String> {
    if extra == 0 {
        return Ok(());
    }

    let label = window.label().to_string();
    let pos = window.outer_position().map_err(|e| e.to_string())?;
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let Some(monitor) = window.current_monitor().map_err(|e| e.to_string())? else {
        return Ok(());
    };
    let area = monitor.work_area();

    let height = (size.height + extra).min(area.size.height);
    if height <= size.height {
        return Ok(());
    }

    let mut guard = TALL_RESTORE.lock().map_err(|e| e.to_string())?;
    let saved = guard.get_or_insert_with(HashMap::new);
    // Only the first growth records where the note started; a second one
    // would otherwise record the already-grown size as the way back.
    saved.entry(label).or_insert((pos, size));

    // Growing downwards off the bottom of the screen would hide the very thing
    // being made room for, so the note moves up instead when it has to.
    let bottom = area.position.y + area.size.height as i32;
    let y = if pos.y + height as i32 > bottom {
        (bottom - height as i32).max(area.position.y)
    } else {
        pos.y
    };

    animate(
        &window,
        pos,
        size,
        tauri::PhysicalPosition::new(pos.x, y),
        tauri::PhysicalSize::new(size.width, height),
    );
    Ok(())
}

/* -- Task pages ------------------------------------------------------------ */

/// Label prefix for the one-task page windows.
const PAGE_PREFIX: &str = "page-";

/// Which task each page window is showing, by label.
///
/// Carried here rather than in the window's URL. `WebviewUrl::App` is resolved
/// as a path into the bundled assets, so `index.html?x=1` and `index.html#x=1`
/// are both asking for a file that does not exist — the window opens, the
/// asset is never found, and all you get is a blank rectangle.
static PAGE_TARGETS: Mutex<Option<HashMap<String, (String, String)>>> = Mutex::new(None);

/// What task this page window is for: `(task_id, task_list_id)`.
#[tauri::command]
pub fn page_target(window: tauri::Window) -> Option<(String, String)> {
    PAGE_TARGETS
        .lock()
        .ok()?
        .as_ref()?
        .get(window.label())
        .cloned()
}
/// How much of the screen a page takes, and the least it is worth opening at.
const PAGE_FRACTION: f64 = 0.62;
const PAGE_MIN: (f64, f64) = (720.0, 560.0);

/// Opens a task in a window of its own, centred on the screen.
///
/// A sticky note is 340px wide because it is a note; a description you intend
/// to actually write in is not. This is the same task, in a window with room —
/// and a separate window rather than the note stretched, so the note stays
/// where it was put and the page can be dragged to another screen.
///
/// One page per task: asking twice brings the existing one forward instead of
/// opening a second view of the same text, which would let two editors fight
/// over one description.
/// Async for the same reason `open_note_window` is, and it must stay that way:
/// a synchronous command holds the main thread, and building a window needs the
/// event loop that thread is running. Sync, this deadlocked — the window
/// appeared as a blank rectangle that could not even be closed, and the command
/// never returned.
#[tauri::command]
pub async fn open_task_page(
    app: AppHandle,
    window: tauri::Window,
    task_list_id: String,
    task_id: String,
) -> Result<String, String> {
    let label = format!("{PAGE_PREFIX}{}", sanitise(&task_id));
    log::info!("open_task_page: asked for {label} (list {task_list_id})");

    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(label);
    }

    // Sized and placed against the screen the note is on, not the primary one.
    let monitor = window
        .current_monitor()
        .ok()
        .flatten()
        .or_else(|| app.primary_monitor().ok().flatten());

    let (width, height, x, y) = match &monitor {
        Some(monitor) => {
            let scale = monitor.scale_factor();
            let area = monitor.work_area();
            let logical_w = area.size.width as f64 / scale;
            let logical_h = area.size.height as f64 / scale;

            let width = (logical_w * PAGE_FRACTION).max(PAGE_MIN.0).min(logical_w - 40.0);
            let height = (logical_h * PAGE_FRACTION).max(PAGE_MIN.1).min(logical_h - 40.0);

            let x = area.position.x as f64 / scale + (logical_w - width) / 2.0;
            let y = area.position.y as f64 / scale + (logical_h - height) / 2.0;
            (width, height, Some(x), Some(y))
        }
        None => (PAGE_MIN.0, PAGE_MIN.1, None, None),
    };

    // Recorded before the window exists, so the page can ask for its task the
    // moment it loads.
    PAGE_TARGETS
        .lock()
        .map_err(|e| e.to_string())?
        .get_or_insert_with(HashMap::new)
        .insert(label.clone(), (task_id.clone(), task_list_id.clone()));

    let mut builder =
        WebviewWindowBuilder::new(&app, &label, WebviewUrl::App("index.html".into()))
        .title("Sticky Widget")
        .inner_size(width, height)
        .min_inner_size(420.0, 320.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(true)
        // A page is somewhere you work for a while: it belongs in Alt+Tab, and
        // it must not float above the windows you are reading from.
        .skip_taskbar(false)
        .always_on_top(false)
        .focused(true);

    if let (Some(x), Some(y)) = (x, y) {
        builder = builder.position(x, y);
    }

    builder.build().map_err(|e| e.to_string())?;
    log::info!("open_task_page: {label} for list {task_list_id}");
    Ok(label)
}

/// Closes the calling page window. Ordinary notes are left alone.
#[tauri::command]
pub fn close_task_page(window: tauri::Window) -> Result<(), String> {
    if !window.label().starts_with(PAGE_PREFIX) {
        return Err("Not a task page.".into());
    }
    window.close().map_err(|e| e.to_string())
}

/// Window labels must be alphanumeric-ish; Google's task ids are not.
fn sanitise(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}



/* -- Layouts --------------------------------------------------------------- */

/// Gap between notes when they are arranged, and from the edge of the screen.
const TILE_GAP: i32 = 10;

/// Where every open note is, right now.
fn capture_layout(app: &AppHandle) -> Vec<crate::settings::LayoutNote> {
    let registry = app.state::<NoteRegistry>();
    let mut layout = Vec::new();

    for (label, window) in note_windows(app) {
        let Some(list_id) = registry.list_for(&label) else {
            continue;
        };
        let (Ok(pos), Ok(size), Ok(scale)) = (
            window.outer_position(),
            window.outer_size(),
            window.scale_factor(),
        ) else {
            continue;
        };

        layout.push(crate::settings::LayoutNote {
            label,
            list_id,
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            scale,
        });
    }

    layout
}

/// Saves the current arrangement under a name, replacing one of that name.
///
/// Saving over an existing name is how a layout is *updated*: arrange the
/// notes, pick the preset, save again. There is no separate update command,
/// because "save as the name that is already selected" is the same act.
#[tauri::command]
pub async fn save_layout(app: AppHandle, name: String) -> Result<usize, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("Give the layout a name.".into());
    }

    let notes = capture_layout(&app);
    if notes.is_empty() {
        return Err("There are no notes open to save.".into());
    }

    let count = notes.len();
    {
        let state = app.state::<AppState>();
        let mut settings = state.settings.lock().map_err(|e| e.to_string())?;

        match settings.layouts.iter_mut().find(|l| l.name == name) {
            Some(existing) => existing.notes = notes,
            None => {
                if settings.layouts.len() >= crate::settings::MAX_LAYOUTS {
                    return Err(format!(
                        "That is {} layouts already — rename or delete one first.",
                        crate::settings::MAX_LAYOUTS
                    ));
                }
                settings
                    .layouts
                    .push(crate::settings::Layout { name: name.clone(), notes });
            }
        }

        crate::settings::save(&app, &settings);
    }

    log::info!("layout: saved {count} note(s) as {name}");
    Ok(count)
}

/// Every saved layout's name, in the order they were made.
#[tauri::command]
pub fn list_layouts(app: AppHandle) -> Vec<String> {
    app.state::<AppState>()
        .settings
        .lock()
        .map(|s| s.layouts.iter().map(|l| l.name.clone()).collect())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn delete_layout(app: AppHandle, name: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let mut settings = state.settings.lock().map_err(|e| e.to_string())?;
    settings.layouts.retain(|l| l.name != name);
    crate::settings::save(&app, &settings);
    log::info!("layout: deleted {name}");
    Ok(())
}

/// Puts a saved layout back: opens what is missing, moves what is there.
///
/// Notes not in the layout are left alone rather than closed. Closing windows
/// the user has opened since would be a surprise, and "restore my arrangement"
/// does not have to mean "and discard everything else".
#[tauri::command]
pub async fn apply_layout(app: AppHandle, name: String) -> Result<usize, String> {
    let saved = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings
            .layouts
            .iter()
            .find(|l| l.name == name)
            .map(|l| l.notes.clone())
    };

    let Some(saved) = saved else {
        return Err(format!("There is no layout called {name}."));
    };

    LAYOUT_BUSY.store(true, Ordering::Relaxed);
    let known = known_list_ids(&app);
    let mut placed = 0;

    for note in saved {
        if !known.is_empty() && !known.contains(&note.list_id) {
            log::info!("layout: skipping a note whose list is gone");
            continue;
        }

        let window = match app.get_webview_window(&note.label) {
            Some(existing) => existing,
            None => {
                // Reuse the label, so the note keeps its own colour and pin.
                match open_note_labelled(&app, &note.list_id, None, Some(&note.label)) {
                    Ok(_) => match app.get_webview_window(&note.label) {
                        Some(w) => w,
                        None => continue,
                    },
                    Err(err) => {
                        log::warn!("layout: could not open {}: {err}", note.label);
                        continue;
                    }
                }
            }
        };

        let _ = window.show();
        let _ = window.set_size(tauri::PhysicalSize::new(note.width, note.height));
        let _ = window.set_position(tauri::PhysicalPosition::new(note.x, note.y));
        placed += 1;
    }

    LAYOUT_BUSY.store(false, Ordering::Relaxed);
    persist_open_notes(&app);
    save_geometry_now(&app);
    log::info!("layout: applied {name} to {placed} note(s)");
    Ok(placed)
}


/// Arranges every open note in even columns down the work area.
///
/// Column-first, because a note is tall and narrow: filling downwards keeps
/// each one readable, where a grid of short wide notes would not. Notes are
/// arranged on the screen they are already on, so a second monitor is not
/// emptied onto the first.
///
/// Nothing is saved by arranging. Arrange, look at it, and save it as a preset
/// if it is worth keeping — that way the automatic version is a starting point
/// rather than a thing that overwrites what you had.
#[tauri::command]
pub async fn arrange_notes(app: AppHandle) -> Result<usize, String> {
    let notes = note_windows(&app);
    if notes.is_empty() {
        return Err("There are no notes to arrange.".into());
    }

    LAYOUT_BUSY.store(true, Ordering::Relaxed);

    // Grouped by monitor, in label order, so the result is stable: running it
    // twice gives the same arrangement.
    let mut by_monitor: HashMap<String, Vec<(String, tauri::WebviewWindow)>> = HashMap::new();
    for (label, window) in notes {
        let key = window
            .current_monitor()
            .ok()
            .flatten()
            .and_then(|m| m.name().cloned())
            .unwrap_or_else(|| "primary".to_string());
        by_monitor.entry(key).or_default().push((label, window));
    }

    let mut arranged = 0;

    for (_, mut group) in by_monitor {
        group.sort_by(|a, b| a.0.cmp(&b.0));

        let Some((_, first)) = group.first() else {
            continue;
        };
        let Some(monitor) = first.current_monitor().ok().flatten() else {
            continue;
        };
        let area = monitor.work_area();

        // Width comes from the notes themselves: how wide a note should be is
        // a preference, and arranging is not the place to overrule it.
        let width = group
            .iter()
            .filter_map(|(_, w)| w.outer_size().ok())
            .map(|s| s.width as i32)
            .max()
            .unwrap_or(340);

        let usable_h = area.size.height as i32 - TILE_GAP * 2;
        let columns = ((area.size.width as i32 - TILE_GAP) / (width + TILE_GAP)).max(1);
        let per_column = ((group.len() as i32 + columns - 1) / columns).max(1);
        let height = ((usable_h - TILE_GAP * (per_column - 1)) / per_column).max(200);

        for (index, (_, window)) in group.iter().enumerate() {
            let index = index as i32;
            let column = index / per_column;
            let row = index % per_column;

            let x = area.position.x + TILE_GAP + column * (width + TILE_GAP);
            let y = area.position.y + TILE_GAP + row * (height + TILE_GAP);

            let _ = window.show();
            let _ = window.set_size(tauri::PhysicalSize::new(width as u32, height as u32));
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
            arranged += 1;
        }
    }

    LAYOUT_BUSY.store(false, Ordering::Relaxed);
    save_geometry_now(&app);
    log::info!("layout: arranged {arranged} note(s)");
    Ok(arranged)
}

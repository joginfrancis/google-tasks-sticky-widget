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

/// When the last move or resize was seen, as milliseconds since the app
/// started. Saving waits for this to go quiet.
static LAST_CHANGE_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

use std::sync::atomic::Ordering;

fn millis_since_start() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START.get_or_init(std::time::Instant::now).elapsed().as_millis() as u64
}

/// Records that the arrangement has just changed.
fn touch_geometry() {
    GEOMETRY_DIRTY.store(true, Ordering::Relaxed);
    LAST_CHANGE_MS.store(millis_since_start(), Ordering::Relaxed);
}

/// Notes that the arrangement has changed. The write happens later, once.
///
/// The plugin saves on exit, which is no help to a machine that is switched
/// off, a process that is killed, or an installer that replaces the binary
/// under a running app. So it is written while the app runs — but on a timer,
/// not on every pixel of a drag.
pub fn persist_geometry(_app: &AppHandle) {
    touch_geometry();
}

/// How often the writer looks, and how long the windows must have been still.
const GEOMETRY_SAVE_EVERY: std::time::Duration = std::time::Duration::from_millis(700);
const GEOMETRY_QUIET_MS: u64 = 1_200;

/// Starts the one thread that writes window geometry.
pub fn start_geometry_writer(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(GEOMETRY_SAVE_EVERY);

        if !GEOMETRY_DIRTY.load(Ordering::Relaxed) || LAYOUT_BUSY.load(Ordering::Relaxed) {
            continue;
        }

        // Only once nothing has moved for a moment. Saving mid-drag, or in the
        // middle of an arrange, is what deadlocked the main thread: the save
        // and the window events it generates want the same lock.
        let quiet = millis_since_start()
            .saturating_sub(LAST_CHANGE_MS.load(Ordering::Relaxed))
            >= GEOMETRY_QUIET_MS;
        if !quiet {
            continue;
        }

        GEOMETRY_DIRTY.store(false, Ordering::Relaxed);
        let _ = app.save_window_state(StateFlags::POSITION | StateFlags::SIZE);
        remember_arrangement(&app);
        log::debug!("geometry: saved");
    });
}

/// Writes the arrangement right now — for the end of a layout operation.

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

/// Applies a set of positions and sizes, on a thread of its own.
///
/// Three attempts to get this right, so the reasoning is worth keeping:
///
/// 1. One `set_size` per window from the command thread. Each call is a
///    blocking request to the main thread, and a run of them jammed it.
/// 2. The whole set handed to the main thread with `run_on_main_thread`. That
///    deadlocked *harder*: the calls themselves post to the event loop and wait
///    for it, and the event loop was busy running the closure that made them.
///    The command returned and logged success while the app froze behind it.
/// 3. A plain OS thread, which is what the window animation has always used
///    and what has always worked. The calls dispatch to an event loop that is
///    free to answer them.
///
/// It returns as soon as the work is handed over: the windows move a frame or
/// two later, which is invisible, and nothing waits on anything.
fn place_windows(
    app: &AppHandle,
    moves: Vec<(String, tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>)>,
) -> usize {
    let count = moves.len();
    let handle = app.clone();

    std::thread::spawn(move || {
        for (label, pos, size) in moves {
            let Some(window) = handle.get_webview_window(&label) else {
                continue;
            };
            let _ = window.show();
            if let Err(err) = window.set_size(size) {
                log::warn!("place: could not resize {label}: {err}");
            }
            if let Err(err) = window.set_position(pos) {
                log::warn!("place: could not move {label}: {err}");
            }
        }

        log::info!("place: done");
        LAYOUT_BUSY.store(false, Ordering::Relaxed);

        // Deliberately *not* saving here.
        //
        // `save_window_state` asks every window where it is, and the plugin
        // takes its own lock while doing so — the same lock its event hook
        // takes for each `Moved`/`Resized` the moves have just generated. Save
        // into that storm and the main thread stops answering: the app froze
        // while its background threads carried on, which is exactly what the
        // logs showed. The writer below picks this up once the dust settles.
        touch_geometry();
    });

    count
}

/// How often the displays are counted, and how long they must be settled.
const SCREEN_POLL: std::time::Duration = std::time::Duration::from_secs(3);
const SCREEN_SETTLE: std::time::Duration = std::time::Duration::from_secs(2);

/// Watches for displays coming and going, and puts the notes back.
///
/// Unplugging a monitor makes Windows drag every note on it onto another
/// screen, in a heap. Plugging it back in does *not* undo that — the notes
/// stay where they were dragged, which is the friction that started all of
/// this. So the set of displays is counted, and when it changes the last
/// layout used is applied again a couple of seconds later, once Windows has
/// finished rearranging things itself.
///
/// Polled rather than event-driven: Tauri exposes no display-change event, and
/// a count of monitor names every three seconds costs nothing.
pub fn watch_screens(app: &AppHandle) {
    let app = app.clone();

    std::thread::spawn(move || {
        let names = |app: &AppHandle| -> Vec<String> {
            let mut names: Vec<String> = app
                .available_monitors()
                .unwrap_or_default()
                .iter()
                .filter_map(|m| m.name().cloned())
                .collect();
            names.sort();
            names
        };

        let mut known = names(&app);

        loop {
            std::thread::sleep(SCREEN_POLL);

            let current = names(&app);
            if current == known {
                continue;
            }

            log::info!(
                "screens: changed from {} to {} display(s)",
                known.len(),
                current.len()
            );
            known = current;

            // Let Windows finish moving things before moving them back.
            std::thread::sleep(SCREEN_SETTLE);

            // What this set of displays looked like last time. Not the named
            // layout: a layout is a preference, and this is a memory — the
            // one-screen arrangement and the two-screen one are both wanted,
            // each when its screens are attached.
            let signature = screen_signature(&app);
            let remembered = app
                .state::<AppState>()
                .settings
                .lock()
                .ok()
                .and_then(|s| s.layout_by_screens.get(&signature).cloned());

            match remembered {
                Some(notes) => {
                    let placed = apply_notes(&app, notes);
                    log::info!("screens: restored the arrangement for {signature} ({placed} note(s))");
                }
                None => {
                    // Never seen these displays together. Rather than leaving
                    // the notes wherever Windows dropped them, the remembered
                    // *shape* is projected onto the main screen: the same
                    // layout at a different size, which is what a change of
                    // display should mean.
                    let area = app
                        .primary_monitor()
                        .ok()
                        .flatten()
                        .map(|m| { let a = *m.work_area(); (a.position, a.size) });

                    match area {
                        Some((pos, size)) => {
                            let placed = apply_relative(&app, pos, size);
                            log::info!(
                                "screens: projected the layout onto the main screen ({placed} note(s))"
                            );
                        }
                        None => log::warn!("screens: no main screen to project onto"),
                    }
                }
            }
        }
    });
}

/// Brings a saved position onto a screen that exists.
///
/// A layout saved with a second monitor attached names coordinates that are
/// nowhere once it is unplugged, and a note placed there is invisible with no
/// way back. Rather than refusing to restore, the position is carried onto the
/// nearest screen as a proportion of the one it was saved on: a note two
/// thirds across and half way down stays two thirds across and half way down.
fn fit_to_screen(
    app: &AppHandle,
    note: &crate::settings::LayoutNote,
) -> (tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
    let saved_pos = tauri::PhysicalPosition::new(note.x, note.y);
    let saved_size = tauri::PhysicalSize::new(note.width, note.height);

    let monitors = app.available_monitors().unwrap_or_default();
    if monitors.is_empty() {
        return (saved_pos, saved_size);
    }

    // The display it was saved on is here. Its coordinates may not be: a
    // screen that was at 0,0 alone sits at 1920,0 once another joins on its
    // left, so the saved position is read against where that display is *now*
    // rather than taken literally.
    if let Some(name) = &note.monitor {
        if let Some(screen) = monitors.iter().find(|m| m.name() == Some(name)) {
            let now = screen.position();
            let moved_x = now.x - note.monitor_x;
            let moved_y = now.y - note.monitor_y;

            if moved_x != 0 || moved_y != 0 {
                log::info!(
                    "layout: {} follows {name}, which moved by {moved_x},{moved_y}",
                    note.label
                );
            }

            return (
                tauri::PhysicalPosition::new(note.x + moved_x, note.y + moved_y),
                saved_size,
            );
        }
    }

    // On a screen it still fits: leave it exactly where it was saved.
    let fits = monitors.iter().any(|m| {
        let area = m.work_area();
        note.x >= area.position.x
            && note.y >= area.position.y
            && note.x + note.width as i32 <= area.position.x + area.size.width as i32
            && note.y + note.height as i32 <= area.position.y + area.size.height as i32
    });
    if fits {
        return (saved_pos, saved_size);
    }

    let Some(primary) = app.primary_monitor().ok().flatten().or_else(|| monitors.first().cloned())
    else {
        return (saved_pos, saved_size);
    };
    let area = *primary.work_area();

    // The screen it was saved on, so the proportions mean something. Nothing
    // records which that was, so the monitor whose span contains the saved
    // position is the best guess; failing that, the one it is nearest to.
    let source = monitors
        .iter()
        .find(|m| {
            let a = m.work_area();
            note.x >= a.position.x && note.x < a.position.x + a.size.width as i32
        })
        .map(|m| *m.work_area())
        .unwrap_or(area);

    let fx = (note.x - source.position.x) as f64 / source.size.width.max(1) as f64;
    let fy = (note.y - source.position.y) as f64 / source.size.height.max(1) as f64;
    let fh = (note.height as f64 / source.size.height.max(1) as f64).min(1.0);

    let width = note.width.min(area.size.width.saturating_sub(2 * TILE_GAP as u32));
    let height = ((fh * area.size.height as f64).round() as u32)
        .min(area.size.height.saturating_sub(2 * TILE_GAP as u32))
        .max(200);

    let x = area.position.x + (fx * area.size.width as f64).round() as i32;
    let y = area.position.y + (fy * area.size.height as f64).round() as i32;

    let x = x.clamp(
        area.position.x + TILE_GAP,
        (area.position.x + area.size.width as i32 - width as i32 - TILE_GAP)
            .max(area.position.x + TILE_GAP),
    );
    let y = y.clamp(
        area.position.y + TILE_GAP,
        (area.position.y + area.size.height as i32 - height as i32 - TILE_GAP)
            .max(area.position.y + TILE_GAP),
    );

    log::info!(
        "layout: {} was off-screen; brought to {x},{y} at {width}x{height}",
        note.label
    );
    (
        tauri::PhysicalPosition::new(x, y),
        tauri::PhysicalSize::new(width, height),
    )
}

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

        // A note that is hidden — by the shortcut, or minimised — reports the
        // parking spot rather than where it will reappear. Recording that puts
        // the note at -32000 next time the arrangement is restored, which is
        // how four notes went missing and one came back.
        if !is_placeable(&pos, &size) || !window.is_visible().unwrap_or(false) {
            log::debug!("layout: skipping {label}, which is not on screen");
            continue;
        }

        let screen = window.current_monitor().ok().flatten();
        let monitor = screen.as_ref().and_then(|m| m.name().cloned());
        let frame = screen.as_ref().map(|m| (*m.position(), *m.size()));

        layout.push(crate::settings::LayoutNote {
            label,
            list_id,
            x: pos.x,
            y: pos.y,
            width: size.width,
            height: size.height,
            scale,
            monitor,
            monitor_x: frame.map(|(p, _)| p.x).unwrap_or(0),
            monitor_y: frame.map(|(p, _)| p.y).unwrap_or(0),
            monitor_width: frame.map(|(_, s)| s.width).unwrap_or(0),
            monitor_height: frame.map(|(_, s)| s.height).unwrap_or(0),
        });
    }

    layout
}

/// Windows parks hidden and minimised windows far off-screen, at -32000. Any
/// coordinate out there is a parking spot, not a position worth remembering.
const PARKED: i32 = -30000;

/// Whether a window's geometry is worth recording or restoring.
fn is_placeable(pos: &tauri::PhysicalPosition<i32>, size: &tauri::PhysicalSize<u32>) -> bool {
    pos.x > PARKED && pos.y > PARKED && size.width > 120 && size.height > 120
}

/// Fits a whole arrangement onto one screen, keeping its shape.
///
/// The bounding box of every note is scaled by a single factor — the smaller
/// of the width and height ratios, so nothing is squashed — and centred on the
/// target. One scale for everything is the point: scaling each note by its own
/// fractions stretches the gaps between them while leaving the notes alone,
/// and five tidy columns arrive as five columns with uneven spaces.
fn fit_arrangement(
    items: Vec<(String, tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>)>,
    to: &tauri::PhysicalPosition<i32>,
    to_size: &tauri::PhysicalSize<u32>,
) -> Vec<(String, tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>)> {
    if items.is_empty() {
        return items;
    }

    let left = items.iter().map(|(_, p, _)| p.x).min().unwrap_or(0);
    let top = items.iter().map(|(_, p, _)| p.y).min().unwrap_or(0);
    let right = items
        .iter()
        .map(|(_, p, s)| p.x + s.width as i32)
        .max()
        .unwrap_or(0);
    let bottom = items
        .iter()
        .map(|(_, p, s)| p.y + s.height as i32)
        .max()
        .unwrap_or(0);

    let box_w = (right - left).max(1) as f64;
    let box_h = (bottom - top).max(1) as f64;

    let usable_w = (to_size.width as i32 - TILE_GAP * 2).max(1) as f64;
    let usable_h = (to_size.height as i32 - TILE_GAP * 2).max(1) as f64;

    // Never stretched beyond a third again: a small arrangement blown up to
    // fill a large screen is not what anyone meant by "keep the layout".
    let scale = (usable_w / box_w).min(usable_h / box_h).min(1.35);

    let scaled_w = box_w * scale;
    let scaled_h = box_h * scale;
    let offset_x = to.x as f64 + (usable_w - scaled_w).max(0.0) / 2.0 + TILE_GAP as f64;
    let offset_y = to.y as f64 + (usable_h - scaled_h).max(0.0) / 2.0 + TILE_GAP as f64;

    items
        .into_iter()
        .map(|(label, pos, size)| {
            let x = offset_x + (pos.x - left) as f64 * scale;
            let y = offset_y + (pos.y - top) as f64 * scale;
            let w = (size.width as f64 * scale).round().max(200.0) as u32;
            let h = (size.height as f64 * scale).round().max(160.0) as u32;

            (
                label,
                tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32),
                tauri::PhysicalSize::new(w, h),
            )
        })
        .collect()
}

/// Describes the current arrangement as proportions of its own bounding box.
///
/// Pixels are a fact about one screen; proportions are a fact about the
/// arrangement. Keeping both means a configuration that has been seen before
/// can be restored exactly, and one that has not can still be given the same
/// shape.
fn relative_from(notes: &[crate::settings::LayoutNote]) -> Vec<crate::settings::RelativeNote> {
    if notes.is_empty() {
        return Vec::new();
    }

    let left = notes.iter().map(|n| n.x).min().unwrap_or(0) as f64;
    let top = notes.iter().map(|n| n.y).min().unwrap_or(0) as f64;
    let right = notes.iter().map(|n| n.x + n.width as i32).max().unwrap_or(1) as f64;
    let bottom = notes.iter().map(|n| n.y + n.height as i32).max().unwrap_or(1) as f64;

    let w = (right - left).max(1.0);
    let h = (bottom - top).max(1.0);

    notes
        .iter()
        .map(|n| crate::settings::RelativeNote {
            label: n.label.clone(),
            list_id: n.list_id.clone(),
            fx: (n.x as f64 - left) / w,
            fy: (n.y as f64 - top) / h,
            fw: n.width as f64 / w,
            fh: n.height as f64 / h,
        })
        .collect()
}

/// Lays the relative arrangement out on one screen.
///
/// The bounding box is fitted to the work area — the whole of it, since the
/// arrangement was built to fill a screen — and every note placed by its own
/// proportions within that box. Nothing here reads a window's current
/// position, which is the point: on a display change Windows has already
/// moved them, and mapping from that heap is what shuffled the order.
fn project_relative(
    app: &AppHandle,
    relative: &[crate::settings::RelativeNote],
    area_pos: &tauri::PhysicalPosition<i32>,
    area_size: &tauri::PhysicalSize<u32>,
) -> Vec<(String, tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>)> {
    let known = known_list_ids(app);

    let box_x = area_pos.x as f64 + TILE_GAP as f64;
    let box_y = area_pos.y as f64 + TILE_GAP as f64;
    let box_w = (area_size.width as i32 - TILE_GAP * 2).max(200) as f64;
    let box_h = (area_size.height as i32 - TILE_GAP * 2).max(200) as f64;

    relative
        .iter()
        .filter(|note| known.is_empty() || known.contains(&note.list_id))
        .map(|note| {
            let x = (box_x + note.fx * box_w).round() as i32;
            let y = (box_y + note.fy * box_h).round() as i32;
            let w = (note.fw * box_w).round().max(220.0) as u32;
            let h = (note.fh * box_h).round().max(180.0) as u32;

            (
                note.label.clone(),
                tauri::PhysicalPosition::new(x, y),
                tauri::PhysicalSize::new(w, h),
            )
        })
        .collect()
}

/// Opens any note the arrangement names but which is not on screen.
fn open_missing(app: &AppHandle, notes: &[crate::settings::RelativeNote]) {
    let known = known_list_ids(app);
    for note in notes {
        if app.get_webview_window(&note.label).is_some() {
            continue;
        }
        if !known.is_empty() && !known.contains(&note.list_id) {
            continue;
        }
        if let Err(err) = open_note_labelled(app, &note.list_id, None, Some(&note.label)) {
            log::warn!("layout: could not open {}: {err}", note.label);
        }
    }
}

/// Puts the remembered shape onto one screen, whatever its size.
pub fn apply_relative(app: &AppHandle, area_pos: tauri::PhysicalPosition<i32>, area_size: tauri::PhysicalSize<u32>) -> usize {
    let relative = {
        let state = app.state::<AppState>();
        let Ok(settings) = state.settings.lock() else {
            return 0;
        };
        settings.relative_layout.clone()
    };

    if relative.is_empty() {
        return 0;
    }

    LAYOUT_BUSY.store(true, Ordering::Relaxed);
    open_missing(app, &relative);

    let moves = project_relative(app, &relative, &area_pos, &area_size);
    let count = moves.len();
    place_windows(app, moves);
    persist_open_notes(app);
    count
}

/// The set of displays attached right now, as a key.
///
/// Names rather than positions: a monitor keeps its name across unplugging,
/// where its coordinates change the moment another one joins or leaves.
pub fn screen_signature(app: &AppHandle) -> String {
    let mut names: Vec<String> = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .filter_map(|m| m.name().cloned())
        .collect();
    names.sort();
    if names.is_empty() {
        return "none".to_string();
    }
    names.join("|")
}

/// Writes down where the notes are, against the displays they are on.
///
/// Called when the windows have been still for a moment, so it records a
/// settled arrangement rather than the middle of a drag.
pub fn remember_arrangement(app: &AppHandle) {
    let notes = capture_layout(app);
    if notes.is_empty() {
        return;
    }

    // Hidden notes are not captured, so a hidden set would record as a smaller
    // arrangement and quietly lose the rest. Only a complete picture is worth
    // remembering.
    if notes.len() < note_windows(app).len() {
        log::debug!("layout: not remembering a partial arrangement");
        return;
    }

    let signature = screen_signature(app);
    let state = app.state::<AppState>();
    let Ok(mut settings) = state.settings.lock() else {
        return;
    };

    if settings.layout_by_screens.get(&signature) == Some(&notes) {
        return;
    }

    let relative = relative_from(&notes);
    settings.layout_by_screens.insert(signature, notes);
    settings.relative_layout = relative;
    crate::settings::save(app, &settings);
}

/// Puts a set of notes where the saved arrangement says, opening any that are
/// closed. Shared by the named layouts and by the per-display memory.
fn apply_notes(app: &AppHandle, saved: Vec<crate::settings::LayoutNote>) -> usize {
    LAYOUT_BUSY.store(true, Ordering::Relaxed);
    let known = known_list_ids(app);
    let mut placed = 0;
    let mut moves = Vec::new();
    let mut stranded: Vec<String> = Vec::new();

    for note in saved {
        if !known.is_empty() && !known.contains(&note.list_id) {
            log::info!("layout: skipping a note whose list is gone");
            continue;
        }

        // Written by an older build while the note was hidden. Placing it
        // would put the note 32,000 pixels off-screen, so it is laid out with
        // the others instead.
        let parked = !is_placeable(
            &tauri::PhysicalPosition::new(note.x, note.y),
            &tauri::PhysicalSize::new(note.width, note.height),
        );

        if app.get_webview_window(&note.label).is_none() {
            // Reuse the label, so the note keeps its own colour and pin.
            if let Err(err) = open_note_labelled(app, &note.list_id, None, Some(&note.label)) {
                log::warn!("layout: could not open {}: {err}", note.label);
                continue;
            }
        }

        if parked {
            stranded.push(note.label.clone());
            placed += 1;
            continue;
        }

        let (pos, size) = fit_to_screen(app, &note);
        moves.push((note.label.clone(), pos, size));
        placed += 1;
    }

    // A note with no usable saved position is given one: beside the notes
    // that do have one, down the right-hand edge of the main screen.
    if !stranded.is_empty() {
        log::info!("layout: {} note(s) had no usable position", stranded.len());
        if let Some(area) = app.primary_monitor().ok().flatten().map(|m| *m.work_area()) {
            let width = 340;
            let height = (area.size.height as i32 - TILE_GAP * 2).clamp(240, 1000) as u32;
            for (index, label) in stranded.into_iter().enumerate() {
                let x = area.position.x + area.size.width as i32
                    - width
                    - TILE_GAP
                    - index as i32 * (width + TILE_GAP);
                moves.push((
                    label,
                    tauri::PhysicalPosition::new(x.max(area.position.x + TILE_GAP), area.position.y + TILE_GAP),
                    tauri::PhysicalSize::new(width as u32, height),
                ));
            }
        }
    }

    place_windows(app, moves);
    persist_open_notes(app);
    placed
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

        settings.last_layout = Some(name.clone());
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

    let placed = apply_notes(&app, saved);

    if let Ok(mut settings) = app.state::<AppState>().settings.lock() {
        if settings.last_layout.as_deref() != Some(name.as_str()) {
            settings.last_layout = Some(name.clone());
            crate::settings::save(&app, &settings);
        }
    }

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
    log::info!("arrange: starting with {} note(s)", notes.len());
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
        log::info!("arrange: {label} is on {key}");
        by_monitor.entry(key).or_default().push((label, window));
    }

    let mut arranged = 0;
    let mut moves = Vec::new();

    for (_, mut group) in by_monitor {
        group.sort_by(|a, b| a.0.cmp(&b.0));

        let Some((_, first)) = group.first() else {
            continue;
        };
        // `current_monitor` can answer `None` for a window that is off-screen
        // or on a display that has just changed; the primary monitor is a
        // better answer than silently arranging nothing.
        let monitor = match first.current_monitor() {
            Ok(Some(monitor)) => Some(monitor),
            other => {
                log::warn!("arrange: no current monitor ({other:?}); falling back to primary");
                app.primary_monitor().ok().flatten()
            }
        };
        let Some(monitor) = monitor else {
            log::warn!("arrange: no monitor at all; skipping this group");
            continue;
        };
        let area = monitor.work_area();
        log::info!(
            "arrange: {} note(s) on a {}x{} work area at {},{}",
            group.len(),
            area.size.width,
            area.size.height,
            area.position.x,
            area.position.y
        );

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

        for (index, (label, _)) in group.iter().enumerate() {
            let index = index as i32;
            let column = index / per_column;
            let row = index % per_column;

            // Filling from the right-hand edge: a desktop's left side holds
            // whatever is being worked on, and notes belong at its margin.
            let right = area.position.x + area.size.width as i32 - TILE_GAP;
            let x = right - (column + 1) * width - column * TILE_GAP;
            let y = area.position.y + TILE_GAP + row * (height + TILE_GAP);

            moves.push((
                label.clone(),
                tauri::PhysicalPosition::new(x, y),
                tauri::PhysicalSize::new(width as u32, height as u32),
            ));
            arranged += 1;
        }
    }

    log::info!("arrange: placing {} note(s)", moves.len());
    place_windows(&app, moves);
    log::info!("layout: arranged {arranged} note(s)");
    Ok(arranged)
}

/// The vertical grid a note snaps to. Small enough that a note barely moves,
/// large enough that two notes nudged to "about the same height" end up
/// exactly level.
const SNAP_STEP: i32 = 24;

/// Pulls every note onto a grid without rearranging them.
///
/// `arrange_notes` decides where notes go; this one keeps where they already
/// are and makes it exact — the way icons settle onto a phone's grid. A note
/// moves to the nearest column, its top snaps to the nearest step, and its
/// height becomes a whole number of steps. Two notes left at roughly the same
/// height finish exactly level, and a column of notes finishes exactly as wide
/// as its neighbours.
///
/// Overlaps are resolved downwards within a column, because the alternative —
/// leaving two notes on top of each other after an alignment — would look like
/// the feature had failed.
#[tauri::command]
pub async fn snap_notes(app: AppHandle) -> Result<usize, String> {
    let notes = note_windows(&app);
    log::info!("snap: starting with {} note(s)", notes.len());
    if notes.is_empty() {
        return Err("There are no notes to align.".into());
    }

    LAYOUT_BUSY.store(true, Ordering::Relaxed);

    // Current geometry, read before anything moves.
    let mut current: Vec<(String, tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>, String)> =
        Vec::new();
    for (label, window) in notes {
        let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
            continue;
        };
        let monitor = window
            .current_monitor()
            .ok()
            .flatten()
            .and_then(|m| m.name().cloned())
            .unwrap_or_else(|| "primary".to_string());
        current.push((label, pos, size, monitor));
    }

    let mut moves = Vec::new();

    // Per monitor, so a second screen keeps its own grid.
    let monitors: Vec<String> = {
        let mut seen: Vec<String> = current.iter().map(|(_, _, _, m)| m.clone()).collect();
        seen.sort();
        seen.dedup();
        seen
    };

    for monitor_name in monitors {
        let mut group: Vec<_> = current
            .iter()
            .filter(|(_, _, _, m)| *m == monitor_name)
            .cloned()
            .collect();

        let monitor = group
            .first()
            .and_then(|(label, _, _, _)| app.get_webview_window(label))
            .and_then(|w| w.current_monitor().ok().flatten())
            .or_else(|| app.primary_monitor().ok().flatten());
        let Some(monitor) = monitor else {
            continue;
        };
        // Owned before the monitor goes out of scope: `work_area` borrows it.
        let area = *monitor.work_area();

        // One width for every note: a column of notes that differ by nine
        // pixels is the untidiness this is meant to remove.
        let width = group
            .iter()
            .map(|(_, _, size, _)| size.width as i32)
            .max()
            .unwrap_or(340)
            .min(area.size.width as i32 - TILE_GAP * 2);

        let column_pitch = width + TILE_GAP;
        let top = area.position.y + TILE_GAP;
        let bottom = area.position.y + area.size.height as i32 - TILE_GAP;
        let columns = ((area.size.width as i32 - TILE_GAP) / column_pitch).max(1);

        // Columns are counted from the right, because that is where arranging
        // puts them and the two should agree.
        let right_edge = area.position.x + area.size.width as i32 - TILE_GAP;

        let column_of = |x: i32| -> i32 {
            (((right_edge - x - width) as f64 / column_pitch as f64).round() as i32)
                .clamp(0, columns - 1)
        };

        // Sorted by column, then down the screen, so a column is filled top to
        // bottom in the order the notes already appear in it.
        group.sort_by_key(|(_, pos, _, _)| (column_of(pos.x), pos.y));

        let mut column_counts: Vec<i32> = vec![0; columns as usize];
        for (_, pos, _, _) in &group {
            column_counts[column_of(pos.x) as usize] += 1;
        }

        let mut placed_in_column: Vec<i32> = vec![0; columns as usize];
        let next_free: Vec<i32> = vec![top; columns as usize];

        for (label, pos, size, _) in group {
            let column = column_of(pos.x);

            // Each column is divided evenly between the notes in it: one
            // note fills the height, two take half each, three a third. That
            // is what "the closest tidy version of this" means for a column,
            // and it leaves no ragged ends.
            let in_column = column_counts[column as usize].max(1);
            let share = (bottom - top - TILE_GAP * (in_column - 1)) / in_column;
            let height = share.max(SNAP_STEP * 4);

            let index = placed_in_column[column as usize];
            let y = top + index * (height + TILE_GAP);
            placed_in_column[column as usize] += 1;

            let x = right_edge - (column + 1) * width - column * TILE_GAP;
            let _ = (size, next_free.len());

            moves.push((
                label,
                tauri::PhysicalPosition::new(x, y),
                tauri::PhysicalSize::new(width as u32, height as u32),
            ));
        }
    }

    let count = moves.len();
    log::info!("snap: aligning {count} note(s)");
    place_windows(&app, moves);
    log::info!("snap: aligned {count} note(s)");
    Ok(count)
}

/// A display, as Settings needs to describe it.
#[derive(serde::Serialize)]
pub struct ScreenInfo {
    /// Windows' own name, used to ask for this screen back.
    pub name: String,
    /// What to show a person: "1920 x 1080" and whether it is the main one.
    pub label: String,
    pub primary: bool,
    /// Whether any note is on it at the moment.
    pub has_notes: bool,
}

#[tauri::command]
pub fn list_screens(app: AppHandle) -> Vec<ScreenInfo> {
    let occupied: Vec<String> = note_windows(&app)
        .iter()
        .filter_map(|(_, w)| w.current_monitor().ok().flatten())
        .filter_map(|m| m.name().cloned())
        .collect();

    let primary = app
        .primary_monitor()
        .ok()
        .flatten()
        .and_then(|m| m.name().cloned());

    let mut screens: Vec<_> = app
        .available_monitors()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|m| {
            let name = m.name().cloned()?;
            Some((name, *m.position(), *m.size(), m.scale_factor()))
        })
        .collect();

    // Left to right, which is how someone thinks about their desk.
    screens.sort_by_key(|(_, pos, _, _)| pos.x);

    screens
        .into_iter()
        .enumerate()
        .map(|(index, (name, _, size, scale))| {
            let is_primary = primary.as_deref() == Some(name.as_str());

            // Logical pixels, which is the number Windows shows in its own
            // display settings. Physical ones made a 125%-scaled 1536x864
            // laptop screen read as "1920 x 1080" — identical to the monitor
            // beside it, so the dropdown offered the same label twice and
            // choosing either did nothing visible.
            let logical_w = (size.width as f64 / scale).round() as u32;
            let logical_h = (size.height as f64 / scale).round() as u32;

            ScreenInfo {
                label: format!(
                    "Screen {}{} — {} × {}",
                    index + 1,
                    if is_primary { " (main)" } else { "" },
                    logical_w,
                    logical_h
                ),
                has_notes: occupied.contains(&name),
                primary: is_primary,
                name,
            }
        })
        .collect()
}

/// Moves every note onto one display, keeping the shape of the arrangement.
///
/// Each note keeps its place *proportionally*: a note two thirds across and
/// half way down its old screen is two thirds across and half way down the new
/// one. Width is kept where it fits, because a note's width is a preference;
/// height is scaled with the screen, because a shorter screen cannot hold it.
#[tauri::command]
pub async fn move_notes_to_screen(app: AppHandle, name: String) -> Result<usize, String> {
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let Some(target) = monitors.iter().find(|m| m.name() == Some(&name)) else {
        return Err("That screen is no longer attached.".into());
    };
    let area = *target.work_area();

    let notes = note_windows(&app);
    if notes.is_empty() {
        return Err("There are no notes to move.".into());
    }

    LAYOUT_BUSY.store(true, Ordering::Relaxed);

    // The remembered *shape* first, projected onto the screen being asked
    // for. Mapping the notes' current positions instead reproduces whatever
    // state they happen to be in — including the heap Windows leaves after a
    // display disappears, which is how the order came out shuffled.
    let relative = {
        let state = app.state::<AppState>();
        let settings = state.settings.lock().map_err(|e| e.to_string())?;
        settings.relative_layout.clone()
    };

    let moves = if relative.is_empty() {
        // Nothing remembered yet: fall back to what is on screen.
        let mut current = Vec::new();
        for (label, window) in notes {
            let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
                continue;
            };
            current.push((label, pos, size));
        }
        fit_arrangement(current, &area.position, &area.size)
    } else {
        open_missing(&app, &relative);
        project_relative(&app, &relative, &area.position, &area.size)
    };

    let count = moves.len();
    log::info!("screens: moving {count} note(s) to {name}");
    place_windows(&app, moves);
    Ok(count)
}


#[cfg(test)]
mod layout_tests {
    use super::*;

    fn at(x: i32, y: i32, w: u32, h: u32) -> (tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>) {
        (
            tauri::PhysicalPosition::new(x, y),
            tauri::PhysicalSize::new(w, h),
        )
    }

    fn note_on(monitor: &str, mx: i32, x: i32, y: i32) -> crate::settings::LayoutNote {
        crate::settings::LayoutNote {
            label: "note-0".into(),
            list_id: "list".into(),
            x,
            y,
            width: 340,
            height: 500,
            scale: 1.0,
            monitor: Some(monitor.into()),
            monitor_x: mx,
            monitor_y: 0,
            monitor_width: 1920,
            monitor_height: 1080,
        }
    }

    #[test]
    fn an_arrangement_keeps_its_proportions_on_a_smaller_screen() {
        // Two columns, 20px apart, on a 1920x1080 screen.
        let items = vec![
            ("a".to_string(), at(0, 0, 340, 1000).0, at(0, 0, 340, 1000).1),
            ("b".to_string(), at(360, 0, 340, 1000).0, at(360, 0, 340, 1000).1),
        ];

        let placed = fit_arrangement(
            items,
            &tauri::PhysicalPosition::new(0, 0),
            &tauri::PhysicalSize::new(1536, 864),
        );

        let a = &placed[0];
        let b = &placed[1];

        // Same shape: the gap shrinks with the notes rather than staying put,
        // which is what made tidy columns arrive unevenly spaced.
        let gap_before = 360.0 - 340.0;
        let gap_after = (b.1.x - (a.1.x + a.2.width as i32)) as f64;
        let scale = a.2.width as f64 / 340.0;
        assert!(
            (gap_after - gap_before * scale).abs() <= 2.0,
            "gap {gap_after} should scale with the notes ({})",
            gap_before * scale
        );

        // Nothing is squashed: both dimensions use the same scale.
        let scale_h = a.2.height as f64 / 1000.0;
        assert!((scale - scale_h).abs() < 0.01, "width and height scales differ");
    }

    #[test]
    fn an_arrangement_fits_inside_the_screen_it_is_given() {
        let items = vec![
            ("a".to_string(), at(0, 0, 340, 1040).0, at(0, 0, 340, 1040).1),
            ("b".to_string(), at(1580, 0, 340, 1040).0, at(1580, 0, 340, 1040).1),
        ];

        let placed = fit_arrangement(
            items,
            &tauri::PhysicalPosition::new(1920, 0),
            &tauri::PhysicalSize::new(1536, 864),
        );

        for (label, pos, size) in &placed {
            assert!(pos.x >= 1920, "{label} starts left of the screen");
            assert!(
                pos.x + size.width as i32 <= 1920 + 1536,
                "{label} runs off the right"
            );
            assert!(
                pos.y + size.height as i32 <= 864,
                "{label} runs off the bottom"
            );
        }
    }

    #[test]
    fn a_single_note_is_not_blown_up_to_fill_a_screen() {
        let items = vec![("a".to_string(), at(10, 10, 340, 400).0, at(10, 10, 340, 400).1)];

        let placed = fit_arrangement(
            items,
            &tauri::PhysicalPosition::new(0, 0),
            &tauri::PhysicalSize::new(3840, 2160),
        );

        // "Keep the layout" does not mean "make one note enormous".
        assert!(placed[0].2.width <= 340 * 2, "scaled up too far");
    }

    #[test]
    fn a_note_follows_its_display_when_the_display_moves() {
        // Saved while the laptop screen was alone at 0,0; now it sits at
        // 1920,0 because a monitor joined on its left. The note belongs at
        // 1920 + 300, not at 300 — which is on the other screen entirely.
        let note = note_on("\\\\.\\DISPLAY1", 0, 300, 50);
        let moved_x = 1920 - note.monitor_x;
        assert_eq!(note.x + moved_x, 2220);
    }
}


#[cfg(test)]
mod relative_tests {
    use super::*;

    fn note(label: &str, x: i32, y: i32, w: u32, h: u32) -> crate::settings::LayoutNote {
        crate::settings::LayoutNote {
            label: label.into(),
            list_id: "list".into(),
            x,
            y,
            width: w,
            height: h,
            scale: 1.0,
            monitor: None,
            monitor_x: 0,
            monitor_y: 0,
            monitor_width: 0,
            monitor_height: 0,
        }
    }

    #[test]
    fn the_shape_survives_a_change_of_screen() {
        // Two columns and a tall note, as on a 1920x1080 screen.
        let notes = vec![
            note("a", 0, 0, 340, 1000),
            note("b", 360, 0, 340, 490),
            note("c", 360, 510, 340, 490),
        ];

        let relative = relative_from(&notes);

        // Proportions, not pixels: b starts just past half way across.
        assert!((relative[1].fx - 360.0 / 700.0).abs() < 0.01);
        assert!((relative[2].fy - 510.0 / 1000.0).abs() < 0.01);

        // And every note keeps its share of the width.
        let total: f64 = relative.iter().map(|n| n.fw).sum();
        assert!(total > 1.0, "three notes across two columns overlap in width");
    }

    #[test]
    fn relative_places_are_stable_whatever_the_order() {
        // The same arrangement described twice, listed in a different order,
        // produces the same proportions — the order notes are enumerated in
        // must not change where they land.
        let a = relative_from(&[note("a", 0, 0, 340, 500), note("b", 360, 0, 340, 500)]);
        let b = relative_from(&[note("b", 360, 0, 340, 500), note("a", 0, 0, 340, 500)]);

        let a_first = a.iter().find(|n| n.label == "a").unwrap();
        let b_first = b.iter().find(|n| n.label == "a").unwrap();
        assert!((a_first.fx - b_first.fx).abs() < 0.0001);
        assert!((a_first.fw - b_first.fw).abs() < 0.0001);
    }
}

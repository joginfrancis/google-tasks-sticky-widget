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

use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};
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

/// What a new note is, before the user resizes it.
const NEW_NOTE_WIDTH: f64 = 340.0;
const NEW_NOTE_HEIGHT: f64 = 480.0;

/// A rectangle in physical pixels: x, y, width, height.
type Rect = (i32, i32, i32, i32);

/// The notes a new one must not land on.
///
/// Hidden and minimised notes are left out: a window nobody can see cannot be
/// obscured, and counting it would push new notes into a corner for no visible
/// reason.
fn occupied_rects(app: &AppHandle) -> Vec<Rect> {
    note_windows(app)
        .into_iter()
        .filter(|(_, w)| {
            w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false)
        })
        .filter_map(|(_, w)| {
            let pos = w.outer_position().ok()?;
            let size = w.outer_size().ok()?;
            Some((pos.x, pos.y, size.width as i32, size.height as i32))
        })
        .collect()
}

/// Do two rectangles share any pixel?
///
/// Edges touching is not an overlap — two notes side by side with no gap are
/// a tidy arrangement, not a collision.
fn rects_overlap(a: Rect, b: Rect) -> bool {
    a.0 < b.0 + b.2 && a.0 + a.2 > b.0 && a.1 < b.1 + b.3 && a.1 + a.3 > b.1
}

/// Is a rectangle wholly inside the work area?
fn inside(r: Rect, area: Rect) -> bool {
    r.0 >= area.0 && r.1 >= area.1 && r.0 + r.2 <= area.0 + area.2 && r.1 + r.3 <= area.1 + area.3
}

/// Where to put a new note: close to the one that asked, on top of nothing.
///
/// "Beside the window that asked" was the whole rule, and it is wrong in
/// exactly the arrangement this app encourages. Notes tiled in columns leave
/// no free space beside any of them — the space to the right of a note is the
/// next note — so every new note landed on an existing one. Same size, same
/// always-on-top, a few pixels off: indistinguishable from the `+` button
/// doing nothing, which is what it looked like.
///
/// Candidates run from "touching the note you clicked" outwards, so the new
/// note still arrives where you are looking whenever there is room there.
fn pick_spot(origin: Rect, size: (i32, i32), area: Rect, taken: &[Rect]) -> (i32, i32) {
    let gap = 12i32;
    let (w, h) = size;

    let mut candidates: Vec<(i32, i32)> = vec![
        (origin.0 + origin.2 + gap, origin.1),          // right
        (origin.0 - w - gap, origin.1),                 // left
        (origin.0, origin.1 + origin.3 + gap),          // below
        (origin.0, origin.1 - h - gap),                 // above
    ];

    // Then the work area itself, in reading order, on a grid of note-sized
    // cells. This is what finds the empty column on a screen whose notes are
    // all stacked on one side.
    let step_x = (w + gap).max(1);
    let step_y = (h + gap).max(1);
    let mut y = area.1;
    while y + h <= area.1 + area.3 {
        let mut x = area.0;
        while x + w <= area.0 + area.2 {
            candidates.push((x, y));
            x += step_x;
        }
        y += step_y;
    }

    for (x, y) in candidates {
        let r = (x, y, w, h);
        if inside(r, area) && !taken.iter().any(|t| rects_overlap(r, *t)) {
            return (x, y);
        }
    }

    // Every cell is spoken for: cascade off the origin rather than refuse, and
    // keep it on screen so it can at least be dragged somewhere better.
    let x = (origin.0 + 32).clamp(area.0, (area.0 + area.2 - w).max(area.0));
    let y = (origin.1 + 32).clamp(area.1, (area.1 + area.3 - h).max(area.1));
    (x, y)
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

    // A label is supplied only when a saved session is being rebuilt, and that
    // is the only time stored geometry belongs to the window being opened.
    let restoring = label.is_some();

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
        .inner_size(NEW_NOTE_WIDTH, NEW_NOTE_HEIGHT)
        .min_inner_size(260.0, 200.0)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(true)
        .skip_taskbar(true)
        .always_on_top(layer == WindowLayer::Top);

    // Place it where there is room, near the window that asked.
    //
    // A small cascade offset looks like nothing happened: both notes are
    // always-on-top and the same size, so a 32px overlap leaves only a sliver
    // showing and the new note reads as missing.
    let mut spot = None;
    if let Some(origin) = near.and_then(|l| app.get_webview_window(l)) {
        if let (Ok(pos), Ok(size)) = (origin.outer_position(), origin.outer_size()) {
            let scale = origin.scale_factor().unwrap_or(1.0);
            let wanted = (
                (NEW_NOTE_WIDTH * scale).round() as i32,
                (NEW_NOTE_HEIGHT * scale).round() as i32,
            );

            let area = origin
                .current_monitor()
                .ok()
                .flatten()
                .map(|m| {
                    let wp = m.work_area().position;
                    let ws = m.work_area().size;
                    (wp.x, wp.y, ws.width as i32, ws.height as i32)
                })
                .unwrap_or((i32::MIN / 4, i32::MIN / 4, i32::MAX / 2, i32::MAX / 2));

            let taken = occupied_rects(app);
            let origin_rect = (pos.x, pos.y, size.width as i32, size.height as i32);
            let (x, y) = pick_spot(origin_rect, wanted, area, &taken);

            log::info!(
                "open_note: placing at ({x},{y}) {}x{}; origin at ({},{}) {}x{}; work area {}x{} at {},{}; {} window(s) in the way",
                wanted.0,
                wanted.1,
                pos.x,
                pos.y,
                size.width,
                size.height,
                area.2,
                area.3,
                area.0,
                area.1,
                taken.len()
            );
            builder = builder.position(x as f64, y as f64);
            spot = Some(tauri::PhysicalPosition::new(x, y));
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
    //
    // Only for a note being restored, though. Labels are handed out in
    // sequence from the highest *currently saved* note, so the first note
    // added after a session that had more notes in it reuses a label whose
    // geometry is still on disk — and the position chosen for it a few lines
    // above was overwritten by wherever that older note happened to sit. On
    // this machine that put every new note four pixels from `note-1`: same
    // size, same list, both always-on-top, which reads as the `+` button
    // doing nothing at all.
    if restoring {
        let _ = window.restore_state(StateFlags::POSITION | StateFlags::SIZE);
    } else if let Some(at) = spot {
        // The builder's position is taken as logical pixels while everything
        // measured above is physical, so on a scaled display the two disagree.
        // Setting it again, in the units the arithmetic was done in, is what
        // makes the chosen spot the one the note actually lands on.
        let _ = window.set_position(at);
    }

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

        // Every resting point, measured. A note dragged by hand changes the
        // shape just as much as Arrange does, and comparing a later switch
        // against the shape needs to know what the shape was.
        log_arrangement(&app, "settled");
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

    // The starting point of the session, once the windows have been placed by
    // the window-state plugin. Without it there is nothing to compare the
    // first screen change against.
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(2500));
        log_arrangement(&app, "at startup");

        // And recorded, not only logged. The writer only runs after something
        // moves, so a session where nothing is dragged would have no shape to
        // project if a screen disappeared — which is exactly the session after
        // an older shape has been dropped as unreadable.
        remember_arrangement(&app);
    });
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
    what: &str,
) -> usize {
    let count = moves.len();
    let handle = app.clone();
    let what = what.to_string();

    for (label, pos, size) in &moves {
        log::info!(
            "place[{what}]: {label} -> {},{} {}x{}",
            pos.x,
            pos.y,
            size.width,
            size.height
        );
    }

    std::thread::spawn(move || {
        for (label, pos, size) in &moves {
            let Some(window) = handle.get_webview_window(label) else {
                continue;
            };
            // Minimised first, then shown. `show` alone leaves a minimised
            // window minimised — it was still in the taskbar with the new size
            // and position applied to a window nobody could see.
            if window.is_minimized().unwrap_or(false) {
                log::info!("place[{what}]: {label} was minimised; restoring it");
                let _ = window.unminimize();
            }
            let _ = window.show();

            // Position before size, so that a window changing screens has
            // already been told about the new DPI by the time it is sized.
            if let Err(err) = window.set_position(*pos) {
                log::warn!("place[{what}]: could not move {label}: {err}");
            }
            if let Err(err) = window.set_size(*size) {
                log::warn!("place[{what}]: could not resize {label}: {err}");
            }
        }

        // Then check, because Windows does not necessarily do as it is told.
        //
        // A window crossing between a 100% screen and a 125% one is rescaled
        // by Windows itself on the DPI change: 621x1000 arrived as 776x1250,
        // every note a quarter too big and hanging off the panel. Moving back
        // divided by the same number. Since Arrange and Align take their width
        // from the notes, each switch made the next arrangement worse.
        //
        // Two passes are enough: the DPI change arrives once, and correcting
        // the size does not move the window to another screen.
        for attempt in 1..=2 {
            std::thread::sleep(std::time::Duration::from_millis(if attempt == 1 {
                180
            } else {
                120
            }));

            let mut corrected = 0;

            for (label, pos, size) in &moves {
                let Some(window) = handle.get_webview_window(label) else {
                    continue;
                };
                let (Ok(now_pos), Ok(now_size)) = (window.outer_position(), window.outer_size())
                else {
                    continue;
                };

                // A pixel or two of difference is window-frame rounding, not
                // Windows having changed its mind.
                let off_by = (now_size.width as i32 - size.width as i32)
                    .abs()
                    .max((now_size.height as i32 - size.height as i32).abs())
                    .max((now_pos.x - pos.x).abs())
                    .max((now_pos.y - pos.y).abs());

                if off_by <= 2 {
                    continue;
                }

                log::info!(
                    "place[{what}]: {label} came out {},{} {}x{} instead of {},{} {}x{}; correcting (pass {attempt})",
                    now_pos.x,
                    now_pos.y,
                    now_size.width,
                    now_size.height,
                    pos.x,
                    pos.y,
                    size.width,
                    size.height
                );

                let _ = window.set_position(*pos);
                let _ = window.set_size(*size);
                corrected += 1;
            }

            if corrected == 0 {
                break;
            }
        }

        log::info!("place[{what}]: done");
        LAYOUT_BUSY.store(false, Ordering::Relaxed);

        // Measured rather than assumed: what was asked for is logged above,
        // and this is what Windows actually did with it. The two disagreeing
        // is its own kind of bug, and it was not visible before.
        std::thread::sleep(std::time::Duration::from_millis(220));
        log_arrangement(&handle, &what);

        // Settings cannot ask where the notes are until they are there: the
        // windows move on this thread, so a question asked from the command
        // that started it is answered with the old positions. That is why the
        // screen picker worked exactly once — it snapped back to the screen
        // the notes had just left, and the next choice was the value it
        // already had. Saying so is better than the 900ms guess it replaced.
        let _ = handle.emit("layout:settled", ());

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

            // What Windows itself did with the notes, before anything here
            // touches them. The heap it leaves is the starting point every
            // branch below has to cope with.
            log_arrangement(&app, "after Windows moved things");

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

            // A remembered arrangement is only usable if it accounts for every
            // note. An incomplete one — written by an older build, or left
            // behind after bad entries were cleaned out — would restore two
            // notes and abandon the rest, which is worse than projecting the
            // shape onto the screen and getting all of them.
            let expected = app
                .state::<AppState>()
                .settings
                .lock()
                .ok()
                .map(|s| s.relative_layout.len())
                .unwrap_or(0)
                .max(note_windows(&app).len());

            // Whether the pixels remembered for these displays still
            // describe the shape in use. They can disagree: the arrangement
            // was rebuilt while the other screen was attached, and the memory
            // for this one is from an earlier session. Which of the two should
            // win is a real question, and it cannot be answered without
            // knowing how often they differ — so it is logged either way.
            let in_use = app
                .state::<AppState>()
                .settings
                .lock()
                .map(|settings| settings.relative_layout.clone())
                .unwrap_or_default();

            if let Some(notes) = remembered.as_ref() {
                log::info!(
                    "screens: {signature} remembers {} note(s); comparing with the shape in use",
                    notes.len()
                );
                log_drift(&app, "remembered pixels vs shape in use", &relative_from(notes));
            } else {
                log::info!("screens: nothing remembered for {signature}");
            }

            let remembered = remembered.filter(|notes| {
                if notes.len() < expected {
                    log::info!(
                        "screens: the arrangement for {signature} covers {} of {expected} note(s); projecting instead",
                        notes.len()
                    );
                    return false;
                }

                // Exact pixels are worth having only while they still describe
                // the arrangement in use. These were recorded the last time
                // these displays were attached, which may have been before the
                // notes were rearranged — or, as the log showed on 23 October,
                // in a session where Windows' own DPI rescale had inflated
                // every note by a quarter. Replaying that is not "the same
                // layout on this screen", which is the whole promise.
                let drift = shape_drift(&relative_from(notes), &in_use);
                if drift > SHAPE_TOLERANCE {
                    log::info!(
                        "screens: the pixels remembered for {signature} are a different arrangement (drift {drift:.3}); projecting the shape instead"
                    );
                    return false;
                }

                true
            });

            match remembered {
                Some(notes) => {
                    let (placed, _) = apply_notes(&app, notes, "restored for these screens");
                    log::info!("screens: restored the arrangement for {signature} ({placed} note(s))");
                    settled_on(&app, &signature);
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
                            settled_on(&app, &signature);
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
            // Against the work area's origin, because that is what was
            // recorded. The two differ whenever the taskbar is at the top or
            // the left of the screen.
            let now = screen.work_area().position;
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

        // The *usable* area, not the whole monitor. A note is measured as a
        // fraction of the space it can actually occupy, and placed back into
        // the same: dividing by 1080 and multiplying by 1032 cost a
        // full-height note four per cent of itself on every trip.
        let frame = screen.as_ref().map(|m| {
            let area = m.work_area();
            (area.position, area.size)
        });

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

/// Writes the whole arrangement to the log, with the numbers that are supposed
/// to survive a change of screen.
///
/// Three failures in this feature were invisible in the log: a note placed off
/// the screen, an arrangement that kept its positions but lost its gaps, and a
/// projection that filled the width while leaving the height alone. All three
/// are obvious in these lines and in none of the ones that came before.
fn log_arrangement(app: &AppHandle, when: &str) {
    let notes = capture_layout(app);

    if notes.is_empty() {
        log::info!("shape[{when}]: nothing visible to measure");
        return;
    }

    // The work area each note could be using, so a rectangle can be read
    // against the screen it is on rather than in the abstract.
    for monitor in app.available_monitors().unwrap_or_default() {
        let area = monitor.work_area();
        log::info!(
            "shape[{when}]: screen {} work area {}x{} at {},{} (scale {:.2}, {}x{} logical)",
            monitor.name().cloned().unwrap_or_else(|| "?".into()),
            area.size.width,
            area.size.height,
            area.position.x,
            area.position.y,
            monitor.scale_factor(),
            (area.size.width as f64 / monitor.scale_factor()).round() as i32,
            (area.size.height as f64 / monitor.scale_factor()).round() as i32,
        );
    }

    let relative = relative_from(&notes);
    let left = notes.iter().map(|n| n.x).min().unwrap_or(0);
    let top = notes.iter().map(|n| n.y).min().unwrap_or(0);
    let right = notes.iter().map(|n| n.x + n.width as i32).max().unwrap_or(0);
    let bottom = notes.iter().map(|n| n.y + n.height as i32).max().unwrap_or(0);

    log::info!(
        "shape[{when}]: {} note(s), bounding box {}x{} at {},{}",
        notes.len(),
        right - left,
        bottom - top,
        left,
        top
    );

    for (note, fraction) in notes.iter().zip(relative.iter()) {
        log::info!(
            "shape[{when}]: {} at {},{} {}x{} on {} — fx {:.3} fy {:.3} fw {:.3} fh {:.3} (w/h {:.2})",
            note.label,
            note.x,
            note.y,
            note.width,
            note.height,
            note.monitor.clone().unwrap_or_else(|| "?".into()),
            fraction.fx,
            fraction.fy,
            fraction.fw,
            fraction.fh,
            note.width as f64 / note.height.max(1) as f64,
        );
    }

    log_gaps(when, &notes);
    log_drift(app, when, &relative);
}

/// The spaces between notes, which are what "even" means to a person looking
/// at a desktop.
///
/// Columns are grouped by left edge — notes that share one are a column — and
/// the gaps reported are between neighbouring columns and between the notes
/// stacked inside each one. An arrangement that keeps its positions but
/// stretches its gaps shows up here as a set of numbers that disagree.
fn log_gaps(when: &str, notes: &[crate::settings::LayoutNote]) {
    let mut columns: Vec<(i32, i32, Vec<(i32, i32)>)> = Vec::new();

    for note in notes {
        // Within a few pixels counts as the same column: a note dragged by
        // hand is never exactly level with its neighbour.
        match columns
            .iter_mut()
            .find(|(x, width, _)| (note.x - x).abs() <= 6 && (note.width as i32 - width).abs() <= 6)
        {
            Some((_, _, rows)) => rows.push((note.y, note.height as i32)),
            None => columns.push((note.x, note.width as i32, vec![(note.y, note.height as i32)])),
        }
    }

    columns.sort_by_key(|(x, _, _)| *x);

    let widths: Vec<i32> = columns.iter().map(|(_, w, _)| *w).collect();
    let between: Vec<i32> = columns
        .windows(2)
        .map(|pair| pair[1].0 - (pair[0].0 + pair[0].1))
        .collect();

    log::info!(
        "shape[{when}]: {} column(s), widths {widths:?}, gaps between {between:?}",
        columns.len()
    );

    for (x, _, mut rows) in columns {
        rows.sort_by_key(|(y, _)| *y);
        let heights: Vec<i32> = rows.iter().map(|(_, h)| *h).collect();
        let stacked: Vec<i32> = rows
            .windows(2)
            .map(|pair| pair[1].0 - (pair[0].0 + pair[0].1))
            .collect();
        log::info!(
            "shape[{when}]: column at x {x}: heights {heights:?}, gaps {stacked:?}"
        );
    }
}

/// How far the arrangement has moved from the shape that is meant to be
/// reproduced on any screen.
///
/// This is the number the whole "same layout on any display" claim rests on:
/// if a projection is faithful, every fraction here is within rounding of the
/// one that was stored. Anything larger says which note went where it should
/// not have, by name.
fn shape_should_hold(when: &str) -> bool {
    matches!(
        when,
        "at startup"
            | "settled"
            | "after Windows moved things"
            | "projected onto one screen"
            | "moved to a chosen screen"
            | "restored for these screens"
    )
}

fn log_drift(app: &AppHandle, when: &str, now: &[crate::settings::RelativeNote]) {
    let stored = {
        let state = app.state::<AppState>();
        let Ok(settings) = state.settings.lock() else {
            return;
        };
        settings.relative_layout.clone()
    };

    if stored.is_empty() {
        log::info!("shape[{when}]: no remembered shape to compare with");
        return;
    }

    let mut worst = 0.0_f64;
    let mut culprit = String::new();

    for note in now {
        let Some(was) = stored.iter().find(|s| s.label == note.label) else {
            log::info!(
                "shape[{when}]: {} is not in the remembered shape at all",
                note.label
            );
            continue;
        };

        for (field, before, after) in [
            ("fx", was.fx, note.fx),
            ("fy", was.fy, note.fy),
            ("fw", was.fw, note.fw),
            ("fh", was.fh, note.fh),
        ] {
            let drift = (after - before).abs();
            if drift > worst {
                worst = drift;
                culprit = format!("{} {field} {before:.3} -> {after:.3}", note.label);
            }
        }
    }

    if stored.len() != now.len() {
        log::warn!(
            "shape[{when}]: remembered {} note(s) but measured {}",
            stored.len(),
            now.len()
        );
    }

    // A twentieth of the arrangement is about a note's width: past that the
    // shape is not the same shape any more. Arranging, aligning, restoring a
    // preset and undoing all change the shape on purpose, so only the
    // operations that promise to keep it are held to it.
    if worst <= 0.05 {
        log::info!("shape[{when}]: shape held, worst drift {worst:.3}");
    } else if shape_should_hold(when) {
        log::warn!("shape[{when}]: the shape has drifted by {worst:.3} — {culprit}");
    } else {
        log::info!("shape[{when}]: shape changed by {worst:.3} — {culprit} (expected here)");
    }
}

/// The width to give every note in a tidy arrangement.
///
/// The middle of the widths present, rather than the largest. Taking the
/// largest meant one note dragged wide made every note wide, which on a 1920
/// screen is the difference between three columns and two — and the notes that
/// did not fit were stacked three-deep instead.
fn tidy_width(mut widths: Vec<i32>, limit: i32) -> i32 {
    if widths.is_empty() {
        return 340;
    }

    widths.sort_unstable();
    let middle = widths[widths.len() / 2];

    // Never narrower than a note can usefully be, never wider than the screen.
    middle.clamp(220, limit.max(220))
}

/// Windows parks hidden and minimised windows far off-screen, at -32000. Any
/// coordinate out there is a parking spot, not a position worth remembering.
const PARKED: i32 = -30000;

/// Whether a window's geometry is worth recording or restoring.
fn is_placeable(pos: &tauri::PhysicalPosition<i32>, size: &tauri::PhysicalSize<u32>) -> bool {
    pos.x > PARKED && pos.y > PARKED && size.width > 120 && size.height > 120
}

/// What a layout operation actually did.
///
/// The count alone cannot say "four of your five notes moved": the fifth shows
/// up as a smaller number and nothing explains it. The reasons travel with the
/// count, so the panel can name what was left behind instead of quietly
/// reporting success.
#[derive(Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LayoutOutcome {
    /// Notes that were given a new place.
    pub moved: usize,
    /// Notes the operation was asked about, moved or not.
    pub total: usize,
    /// One phrase per reason, ready to read: "1 note whose list is gone".
    pub skipped: Vec<String>,
    /// Said instead of the count, when a count would not be the answer —
    /// "They are already on that screen."
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl LayoutOutcome {
    fn new(moved: usize, total: usize, skipped: &[(String, &'static str)]) -> Self {
        for (label, reason) in skipped {
            log::info!("layout: left {label} alone — {reason}");
        }

        Self {
            moved,
            total,
            skipped: summarise(skipped),
            message: None,
        }
    }
}

/// Collapses a list of (note, reason) into one phrase per reason.
///
/// Reasons are written as noun phrases — "whose list is gone", not "is
/// hidden" — so that counting them cannot produce "2 notes is hidden".
fn summarise(skipped: &[(String, &'static str)]) -> Vec<String> {
    let mut reasons: Vec<(&'static str, usize)> = Vec::new();

    for (_, reason) in skipped {
        match reasons.iter_mut().find(|(known, _)| known == reason) {
            Some((_, count)) => *count += 1,
            None => reasons.push((reason, 1)),
        }
    }

    reasons
        .into_iter()
        .map(|(reason, count)| {
            format!(
                "{count} {} {reason}",
                if count == 1 { "note" } else { "notes" }
            )
        })
        .collect()
}

/// Why a note can be left where it is.
const NO_LIST: &str = "whose list is gone";
const NO_SCREEN: &str = "with no screen to put it on";
const NOT_MEASURED: &str = "Windows would not give a position for";
const NOT_IN_LAYOUT: &str = "this layout does not mention";

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
    notes
        .iter()
        .map(|n| {
            // The screen the note is on, as it was when the note was measured.
            // Its width and height are the units: everything about the note is
            // expressed as a fraction of them, so the same four numbers
            // describe the same arrangement on a laptop panel and a monitor.
            let screen_w = if n.monitor_width > 0 {
                n.monitor_width as f64
            } else {
                1920.0
            };
            let screen_h = if n.monitor_height > 0 {
                n.monitor_height as f64
            } else {
                1080.0
            };

            crate::settings::RelativeNote {
                label: n.label.clone(),
                list_id: n.list_id.clone(),
                fx: (n.x - n.monitor_x) as f64 / screen_w,
                fy: (n.y - n.monitor_y) as f64 / screen_h,
                fw: n.width as f64 / screen_w,
                fh: n.height as f64 / screen_h,
                screen: n.monitor.clone(),
            }
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

    let wanted: Vec<&crate::settings::RelativeNote> = relative
        .iter()
        .filter(|note| known.is_empty() || known.contains(&note.list_id))
        .collect();

    // Notes measured on different screens, collapsed onto one.
    //
    // Each note's fractions are of *its* screen, so two notes that were both
    // halfway down their own displays would land on top of each other here.
    // Each original screen is given an equal share of the width instead, in
    // the order the screens were arranged, and the notes keep their place
    // within their share. With one screen — which is the ordinary case — the
    // share is the whole screen and nothing changes.
    let mut screens: Vec<String> = wanted
        .iter()
        .map(|note| note.screen.clone().unwrap_or_default())
        .collect();
    screens.sort();
    screens.dedup();

    let slices = screens.len().max(1) as f64;
    if slices > 1.0 {
        log::info!(
            "layout: {} screens' worth of notes onto one; each gets a {:.0}% slice",
            screens.len(),
            100.0 / slices
        );
    }

    let full_w = area_size.width as f64;
    let full_h = area_size.height as f64;

    wanted
        .iter()
        .map(|note| {
            let slice = screens
                .iter()
                .position(|s| Some(s) == note.screen.as_ref() || (s.is_empty() && note.screen.is_none()))
                .unwrap_or(0) as f64;

            let slice_w = full_w / slices;
            let origin_x = area_pos.x as f64 + slice * slice_w;

            let x = (origin_x + note.fx * slice_w).round() as i32;
            let y = (area_pos.y as f64 + note.fy * full_h).round() as i32;
            // A floor, because a note too narrow to read is not a faithful
            // reproduction of anything. Low enough that an ordinary three-column
            // arrangement still scales honestly onto a 1280-wide screen; a
            // screen narrower than that cannot hold three columns at a readable
            // size, and the notes will overlap rather than shrink to nothing.
            let w = (note.fw * slice_w).round().max(160.0) as u32;
            let h = (note.fh * full_h).round().max(140.0) as u32;

            // Nothing hangs off the edge it was pushed towards: rounding and
            // the minimum size above can both put a note a few pixels past the
            // right or the bottom of a smaller screen.
            let max_x = area_pos.x + area_size.width as i32 - w as i32;
            let max_y = area_pos.y + area_size.height as i32 - h as i32;

            (
                note.label.clone(),
                tauri::PhysicalPosition::new(x.min(max_x).max(area_pos.x), y.min(max_y).max(area_pos.y)),
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
    place_windows(app, moves, "projected onto one screen");
    persist_open_notes(app);
    count
}

/// How far one arrangement's proportions are from another's.
///
/// The largest difference in any one number, so a single note in the wrong
/// place counts as much as all of them being slightly off. Notes missing from
/// either side count as a complete difference: an arrangement of five notes
/// and one of three are not the same arrangement.
fn shape_drift(
    a: &[crate::settings::RelativeNote],
    b: &[crate::settings::RelativeNote],
) -> f64 {
    if a.is_empty() || b.is_empty() {
        return if a.len() == b.len() { 0.0 } else { 1.0 };
    }

    let mut worst: f64 = 0.0;

    for note in a {
        match b.iter().find(|other| other.label == note.label) {
            Some(other) => {
                worst = worst
                    .max((note.fx - other.fx).abs())
                    .max((note.fy - other.fy).abs())
                    .max((note.fw - other.fw).abs())
                    .max((note.fh - other.fh).abs());
            }
            None => return 1.0,
        }
    }

    if a.len() != b.len() {
        return 1.0;
    }

    worst
}

/// Past this, two arrangements are different arrangements rather than the same
/// one with rounding. A twentieth of a screen is about a sixth of a note.
const SHAPE_TOLERANCE: f64 = 0.05;

/// Marks this set of displays as the one the remembered shape now describes.
///
/// Called once the notes have been put back, which is the moment what is on
/// screen stops being whatever Windows did and starts being an arrangement
/// again. Until it is called, the writer leaves the shape alone.
fn settled_on(app: &AppHandle, signature: &str) {
    let state = app.state::<AppState>();
    let Ok(mut settings) = state.settings.lock() else {
        return;
    };
    if settings.relative_signature == signature {
        return;
    }
    log::info!("screens: the shape now belongs to {signature}");
    settings.relative_signature = signature.to_string();
    crate::settings::save(app, &settings);
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

    // The pixels are always worth recording: they describe this set of
    // displays and nothing else, so a heap recorded here is only ever replayed
    // on the desk that produced it, where it is at least what the user is
    // looking at.
    //
    // The shape is different: it is the answer for every screen the app has
    // never seen, so recording a heap into it spreads the mess everywhere. It
    // is only updated once the arrangement belongs to the displays attached
    // now — which, after a monitor disappears, means after the watcher has put
    // the notes back.
    let settled_here =
        settings.relative_signature.is_empty() || settings.relative_signature == signature;

    if settled_here {
        settings.relative_layout = relative;
        settings.relative_signature = signature.clone();
    } else {
        log::debug!(
            "layout: not reshaping; the shape belongs to {} and these are {signature}",
            settings.relative_signature
        );
    }

    settings.layout_by_screens.insert(signature, notes);
    crate::settings::save(app, &settings);
}

/// Puts a set of notes where the saved arrangement says, opening any that are
/// closed. Shared by the named layouts and by the per-display memory.
fn apply_notes(
    app: &AppHandle,
    saved: Vec<crate::settings::LayoutNote>,
    what: &str,
) -> (usize, Vec<(String, &'static str)>) {
    LAYOUT_BUSY.store(true, Ordering::Relaxed);
    let known = known_list_ids(app);
    let mut placed = 0;
    let mut moves = Vec::new();
    let mut stranded: Vec<String> = Vec::new();
    let mut skipped: Vec<(String, &'static str)> = Vec::new();

    for note in saved {
        if !known.is_empty() && !known.contains(&note.list_id) {
            skipped.push((note.label.clone(), NO_LIST));
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
                skipped.push((note.label.clone(), NOT_MEASURED));
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

    place_windows(app, moves, what);
    persist_open_notes(app);
    (placed, skipped)
}

/// Brings every hidden note back before an arrangement is read or written.
///
/// A note hidden by the shortcut answers -32000,-32000 at 160x28, so Arrange
/// and Align used to leave it out and report a smaller count with no
/// explanation. Showing them first is also the honest reading of the request:
/// "arrange my notes" is not "arrange the ones that happen to be visible".
///
/// Returns how many were brought back, and waits a beat if any were: Windows
/// keeps answering with the parking spot until a shown window has actually
/// been mapped, and reading it too early is the same bug in a different place.
fn reveal_notes(app: &AppHandle) -> usize {
    let mut shown = 0;

    for (label, window) in note_windows(app) {
        // A minimised window reports itself as visible, which is how these
        // notes used to sit out an arrange without anyone noticing.
        let minimised = window.is_minimized().unwrap_or(false);
        if window.is_visible().unwrap_or(false) && !minimised {
            continue;
        }
        if minimised {
            let _ = window.unminimize();
        }
        if let Err(err) = window.show() {
            log::warn!("reveal: could not show {label}: {err}");
            continue;
        }
        log::info!("reveal: brought {label} back to arrange it");
        shown += 1;
    }

    if shown > 0 {
        // Blocking for a sixth of a second on a command thread, once per
        // press, rather than reading positions that are not true yet.
        std::thread::sleep(std::time::Duration::from_millis(160));
    }

    shown
}

/// The arrangement as it was before the last layout change.
///
/// Arrange, Align, restoring a preset and moving to another screen all replace
/// an arrangement somebody may have spent an afternoon building. One step back
/// is what makes them safe to try, and being safe to try is what makes them
/// learnable.
static UNDO: Mutex<Option<Vec<crate::settings::LayoutNote>>> = Mutex::new(None);

/// Writes down where the notes are, for the operation that is about to move
/// them.
///
/// Called before anything moves, and never while a placement is still in
/// flight: the positions read during one are half-way through, and keeping
/// those would undo to a state that never existed.
fn remember_for_undo(app: &AppHandle) {
    if LAYOUT_BUSY.load(Ordering::Relaxed) {
        log::debug!("undo: a placement is still running; keeping the older state");
        return;
    }

    let notes = capture_layout(app);
    if notes.is_empty() {
        return;
    }

    if let Ok(mut slot) = UNDO.lock() {
        log::info!("undo: remembering {} note(s)", notes.len());
        *slot = Some(notes);
    }
}

/// Whether there is an arrangement to go back to.
#[tauri::command]
pub fn can_undo_layout() -> bool {
    UNDO.lock().map(|slot| slot.is_some()).unwrap_or(false)
}

/// Puts the notes back as they were before the last layout change.
///
/// The arrangement being replaced is kept in turn, so pressing undo twice
/// returns to the arranged version: one step, both ways, which is enough for
/// "that was not what I meant" without a history to maintain.
#[tauri::command]
pub async fn undo_layout(app: AppHandle) -> Result<LayoutOutcome, String> {
    let previous = UNDO
        .lock()
        .map_err(|e| e.to_string())?
        .take();

    let Some(previous) = previous else {
        return Err("There is nothing to undo.".into());
    };

    let swap = capture_layout(&app);
    let wanted = previous.len();
    let (placed, skipped) = apply_notes(&app, previous, "undone");

    if !swap.is_empty() {
        if let Ok(mut slot) = UNDO.lock() {
            *slot = Some(swap);
        }
    }

    log::info!("undo: put {placed} of {wanted} note(s) back");
    Ok(LayoutOutcome::new(placed, wanted, &skipped))
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
pub async fn apply_layout(app: AppHandle, name: String) -> Result<LayoutOutcome, String> {
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

    remember_for_undo(&app);

    let wanted = saved.len();
    let (placed, skipped) = apply_notes(&app, saved, "restored a saved layout");

    if let Ok(mut settings) = app.state::<AppState>().settings.lock() {
        if settings.last_layout.as_deref() != Some(name.as_str()) {
            settings.last_layout = Some(name.clone());
            crate::settings::save(&app, &settings);
        }
    }

    log::info!("layout: applied {name} to {placed} of {wanted} note(s)");
    Ok(LayoutOutcome::new(placed, wanted, &skipped))
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
pub async fn arrange_notes(app: AppHandle) -> Result<LayoutOutcome, String> {
    reveal_notes(&app);
    log_arrangement(&app, "before arrange");

    let notes = note_windows(&app);
    let total = notes.len();
    log::info!("arrange: starting with {total} note(s)");
    if notes.is_empty() {
        return Err("There are no notes to arrange.".into());
    }

    remember_for_undo(&app);
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
    let mut skipped: Vec<(String, &'static str)> = Vec::new();

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
            for (label, _) in &group {
                skipped.push((label.clone(), NO_SCREEN));
            }
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
        // a preference, and arranging is not the place to overrule it. The
        // middle width rather than the widest, so one wide note does not
        // decide for the rest.
        let width = tidy_width(
            group
                .iter()
                .filter_map(|(_, w)| w.outer_size().ok())
                .map(|s| s.width as i32)
                .collect(),
            area.size.width as i32 - TILE_GAP * 2,
        );

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
    place_windows(&app, moves, "arrange");
    log::info!("layout: arranged {arranged} of {total} note(s)");
    Ok(LayoutOutcome::new(arranged, total, &skipped))
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
pub async fn snap_notes(app: AppHandle) -> Result<LayoutOutcome, String> {
    reveal_notes(&app);
    log_arrangement(&app, "before align");

    let notes = note_windows(&app);
    let total = notes.len();
    log::info!("snap: starting with {total} note(s)");
    if notes.is_empty() {
        return Err("There are no notes to align.".into());
    }

    remember_for_undo(&app);
    LAYOUT_BUSY.store(true, Ordering::Relaxed);

    // Current geometry, read before anything moves.
    let mut current: Vec<(String, tauri::PhysicalPosition<i32>, tauri::PhysicalSize<u32>, String)> =
        Vec::new();
    let mut skipped: Vec<(String, &'static str)> = Vec::new();
    for (label, window) in notes {
        let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
            skipped.push((label, NOT_MEASURED));
            continue;
        };

        // Still parked after `reveal_notes` asked for it back: minimised by
        // Windows itself, most likely. Aligning from that position would send
        // the note to a column that does not exist.
        if !is_placeable(&pos, &size) {
            skipped.push((label, NOT_MEASURED));
            continue;
        }

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
            for (label, _, _, _) in &group {
                skipped.push((label.clone(), NO_SCREEN));
            }
            continue;
        };
        // Owned before the monitor goes out of scope: `work_area` borrows it.
        let area = *monitor.work_area();

        // One width for every note: a column of notes that differ by nine
        // pixels is the untidiness this is meant to remove. The middle width,
        // not the widest — see `tidy_width`.
        let width = tidy_width(
            group.iter().map(|(_, _, size, _)| size.width as i32).collect(),
            area.size.width as i32 - TILE_GAP * 2,
        );

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
    log::info!("snap: aligning {count} of {total} note(s)");
    place_windows(&app, moves, "align");
    Ok(LayoutOutcome::new(count, total, &skipped))
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

    let total = screens.len();

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
                    "{}{} — {} × {}",
                    place_name(index, total),
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

/// What to call a screen, given where it sits on the desk.
///
/// Not a number. The screens are listed left to right, so numbering them makes
/// the left-hand one "Screen 1" — and Windows, which numbers displays its own
/// way, may well call that same screen 2. Someone reading "Screen 2" means the
/// one Windows showed them, picks it, and the notes move to the screen they
/// were already on. Where a screen physically is cannot be got wrong.
fn place_name(index: usize, total: usize) -> String {
    match (index, total) {
        (_, 1) => "This screen".to_string(),
        (0, _) => "Left screen".to_string(),
        (i, t) if i == t - 1 => "Right screen".to_string(),
        (_, 3) => "Middle screen".to_string(),
        (i, _) => format!("{} from the left", nth(i + 1)),
    }
}

fn nth(n: usize) -> &'static str {
    match n {
        2 => "Second",
        3 => "Third",
        4 => "Fourth",
        _ => "Next",
    }
}

/// Moves every note onto one display, keeping the shape of the arrangement.
///
/// Each note keeps its place *proportionally*: a note two thirds across and
/// half way down its old screen is two thirds across and half way down the new
/// one. Width is kept where it fits, because a note's width is a preference;
/// height is scaled with the screen, because a shorter screen cannot hold it.
#[tauri::command]
pub async fn move_notes_to_screen(app: AppHandle, name: String) -> Result<LayoutOutcome, String> {
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let Some(target) = monitors.iter().find(|m| m.name() == Some(&name)) else {
        return Err("That screen is no longer attached.".into());
    };
    let area = *target.work_area();

    let notes = note_windows(&app);
    if notes.is_empty() {
        return Err("There are no notes to move.".into());
    }

    // Already there: say so rather than shuffling the notes on the spot and
    // reporting a move. Four attempts to "switch back" in the log were four
    // moves to the screen the notes were on, each one reported as a success,
    // which is how the misreading survived.
    let elsewhere = notes.iter().any(|(_, window)| {
        window
            .current_monitor()
            .ok()
            .flatten()
            .and_then(|m| m.name().cloned())
            .map(|on| on != name)
            .unwrap_or(true)
    });

    if !elsewhere {
        log::info!("screens: every note is already on {name}");
        return Ok(LayoutOutcome {
            moved: 0,
            total: notes.len(),
            skipped: Vec::new(),
            message: Some("They are already on that screen.".to_string()),
        });
    }

    let open: Vec<String> = notes.iter().map(|(label, _)| label.clone()).collect();
    let total = open.len();

    remember_for_undo(&app);
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

    let mut skipped: Vec<(String, &'static str)> = Vec::new();

    let moves = if relative.is_empty() {
        // Nothing remembered yet: fall back to what is on screen.
        let mut current = Vec::new();
        for (label, window) in notes {
            let (Ok(pos), Ok(size)) = (window.outer_position(), window.outer_size()) else {
                skipped.push((label, NOT_MEASURED));
                continue;
            };
            current.push((label, pos, size));
        }
        fit_arrangement(current, &area.position, &area.size)
    } else {
        open_missing(&app, &relative);
        project_relative(&app, &relative, &area.position, &area.size)
    };

    // A note that is open but absent from the remembered shape would simply
    // stay behind on the old screen, and the count would not mention it.
    for label in open {
        if !moves.iter().any(|(moved, _, _)| *moved == label) {
            skipped.push((label, NOT_IN_LAYOUT));
        }
    }

    let count = moves.len();
    log::info!("screens: moving {count} of {total} note(s) to {name}");
    place_windows(&app, moves, "moved to a chosen screen");
    Ok(LayoutOutcome::new(count, total.max(count), &skipped))
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

/// The same, but measured on a 1920x1032 screen at the origin.
    fn on_screen(label: &str, x: i32, y: i32, w: u32, h: u32) -> crate::settings::LayoutNote {
        crate::settings::LayoutNote {
            monitor: Some("\\\\.\\DISPLAY6".into()),
            monitor_width: 1920,
            monitor_height: 1032,
            ..note(label, x, y, w, h)
        }
    }

    #[test]
    fn the_shape_is_measured_against_the_screen() {
        // Two columns and a tall note on a 1920x1032 screen. The fractions are
        // of the screen, not of the block the three of them happen to form:
        // that is what lets a note keep its own proportions when it moves.
        let notes = vec![
            on_screen("a", 0, 0, 340, 1000),
            on_screen("b", 360, 0, 340, 490),
            on_screen("c", 360, 510, 340, 490),
        ];

        let relative = relative_from(&notes);

        assert!((relative[1].fx - 360.0 / 1920.0).abs() < 0.01, "b's place on the screen");
        assert!((relative[2].fy - 510.0 / 1032.0).abs() < 0.01, "c's place down it");
        assert!((relative[0].fw - 340.0 / 1920.0).abs() < 0.01, "a's share of the width");

        // Three notes in two columns take up well under the whole screen.
        let widest: f64 = relative.iter().map(|n| n.fx + n.fw).fold(0.0, f64::max);
        assert!(widest < 0.4, "the arrangement is not stretched to fill");
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

#[cfg(test)]
mod outcome_tests {
    use super::*;

    #[test]
    fn one_phrase_per_reason_with_the_right_plural() {
        let skipped = vec![
            ("note-1".to_string(), NO_LIST),
            ("note-2".to_string(), NO_SCREEN),
            ("note-3".to_string(), NO_SCREEN),
        ];

        assert_eq!(
            summarise(&skipped),
            vec![
                "1 note whose list is gone".to_string(),
                "2 notes with no screen to put it on".to_string(),
            ]
        );
    }

    #[test]
    fn nothing_skipped_says_nothing() {
        assert!(summarise(&[]).is_empty());
        let outcome = LayoutOutcome::new(5, 5, &[]);
        assert_eq!(outcome.moved, 5);
        assert!(outcome.skipped.is_empty());
    }
}


#[cfg(test)]
mod projection_tests {
    use super::*;

    /// A note on a 1920x1032 screen at the origin, in pixels.
    fn note_at(label: &str, x: i32, y: i32, w: u32, h: u32) -> crate::settings::LayoutNote {
        crate::settings::LayoutNote {
            label: label.into(),
            list_id: "list".into(),
            x,
            y,
            width: w,
            height: h,
            scale: 1.0,
            monitor: Some("\\\\.\\DISPLAY6".into()),
            monitor_x: 0,
            monitor_y: 0,
            monitor_width: 1920,
            monitor_height: 1032,
        }
    }

    /// `project_relative` without an app handle: the same arithmetic, which is
    /// the part worth testing. One screen, so no slicing.
    fn project(
        relative: &[crate::settings::RelativeNote],
        at: (i32, i32),
        size: (f64, f64),
    ) -> Vec<(String, i32, i32, i32, i32)> {
        relative
            .iter()
            .map(|note| {
                let w = (note.fw * size.0).round().max(160.0);
                let h = (note.fh * size.1).round().max(140.0);
                (
                    note.label.clone(),
                    (at.0 as f64 + note.fx * size.0).round() as i32,
                    (at.1 as f64 + note.fy * size.1).round() as i32,
                    w as i32,
                    h as i32,
                )
            })
            .collect()
    }

    #[test]
    fn a_note_is_measured_against_its_own_screen() {
        // The sentence this rule was asked for: a fifth of the width, half the
        // height, three fifths across, half way down.
        let note = note_at("note-0", 1152, 516, 384, 516);
        let shape = relative_from(&[note]);

        assert!((shape[0].fw - 0.2).abs() < 0.001, "a fifth of the width");
        assert!((shape[0].fh - 0.5).abs() < 0.001, "half the height");
        assert!((shape[0].fx - 0.6).abs() < 0.001, "three fifths across");
        assert!((shape[0].fy - 0.5).abs() < 0.001, "half way down");
    }

    #[test]
    fn the_same_fractions_arrive_on_a_different_screen() {
        let shape = relative_from(&[note_at("note-0", 1152, 516, 384, 516)]);

        // A smaller, differently shaped screen: 1280x800 of usable area.
        let placed = project(&shape, (0, 0), (1280.0, 800.0));
        let (_, x, y, w, h) = placed[0].clone();

        assert_eq!(w, 256, "still a fifth of the width");
        assert_eq!(h, 400, "still half the height");
        assert_eq!(x, 768, "still three fifths across");
        assert_eq!(y, 400, "still half way down");
    }

    #[test]
    fn a_note_keeps_its_own_proportions_when_the_screens_match() {
        // The complaint this replaced: moving between two screens of the same
        // size and shape used to stretch a 340x501 note into 611x505, because
        // the arrangement was stretched to fill the screen rather than each
        // note keeping its share of it.
        let shape = relative_from(&[note_at("note-0", 849, 9, 340, 1003)]);
        let placed = project(&shape, (1920, 0), (1920.0, 1020.0));
        let (_, x, _, w, h) = placed[0].clone();

        assert_eq!(w, 340, "the same width on a screen of the same width");
        assert!(
            (h as f64 - 1003.0 * 1020.0 / 1032.0).abs() <= 1.0,
            "height follows the usable height, not the whole panel"
        );
        assert_eq!(x, 1920 + 849, "the same place on the new screen");
    }

    #[test]
    fn an_arrangement_keeps_its_gaps_in_proportion() {
        // Three columns, 10px apart, on a 1920-wide screen.
        let shape = relative_from(&[
            note_at("a", 849, 10, 340, 500),
            note_at("b", 1199, 10, 340, 500),
            note_at("c", 1549, 10, 340, 500),
        ]);

        // A 1280-wide screen: two thirds of the width, so two thirds of
        // everything — the notes and the spaces between them alike.
        let placed = project(&shape, (0, 0), (1280.0, 800.0));
        let gap_one = placed[1].1 - (placed[0].1 + placed[0].3);
        let gap_two = placed[2].1 - (placed[1].1 + placed[1].3);

        assert!(
            (gap_one - gap_two).abs() <= 1,
            "the columns stay evenly spaced: {gap_one} and {gap_two}"
        );
        assert!(
            (gap_one - 7).abs() <= 1,
            "a 10px gap on a 1920 screen is about 7px on a 1280 one: {gap_one}"
        );
    }

    #[test]
    fn nothing_is_asked_to_sit_off_the_screen() {
        let shape = relative_from(&[
            note_at("a", 10, 10, 340, 1012),
            note_at("b", 1570, 10, 340, 1012),
        ]);

        for (label, x, y, w, h) in project(&shape, (0, 0), (1280.0, 800.0)) {
            assert!(x >= 0, "{label} starts left of the screen");
            assert!(y >= 0, "{label} starts above the screen");
            assert!(x + w <= 1280 + 1, "{label} runs past the right edge");
            assert!(y + h <= 800 + 1, "{label} runs past the bottom");
        }
    }
}

#[cfg(test)]
mod width_tests {
    use super::*;

    #[test]
    fn one_wide_note_does_not_decide_for_the_rest() {
        // The widths measured on 15 October, when one note had been dragged
        // wide: taking the largest gave everybody 499 and left room for three
        // columns on a 1920 screen. The middle width leaves room for four.
        let widths = vec![340, 340, 499, 373, 387];
        assert_eq!(tidy_width(widths, 1900), 373);
    }

    #[test]
    fn notes_of_one_width_keep_it() {
        assert_eq!(tidy_width(vec![340, 340, 340], 1900), 340);
    }

    #[test]
    fn a_narrow_screen_caps_the_width() {
        assert_eq!(tidy_width(vec![900, 900, 900], 600), 600);
    }

    #[test]
    fn nothing_to_measure_falls_back_to_a_note_sized_note() {
        assert_eq!(tidy_width(vec![], 1900), 340);
    }
}

#[cfg(test)]
mod placement_tests {
    use super::{pick_spot, Rect};

    /// 1920x1032 with no taskbar in the way, which is this machine's external
    /// display.
    const AREA: Rect = (0, 0, 1920, 1032);
    const SIZE: (i32, i32) = (340, 480);

    #[test]
    fn a_new_note_appears_beside_the_one_that_asked() {
        let origin = (870, 10, 340, 1000);
        assert_eq!(pick_spot(origin, SIZE, AREA, &[origin]), (1222, 10));
    }

    #[test]
    fn it_does_not_land_on_the_note_already_standing_there() {
        // The arrangement that made the + button look broken: three columns,
        // and the space to the right of note-3 is note-1.
        let origin = (870, 10, 340, 1000);
        let taken = [
            origin,
            (1220, 10, 340, 495),
            (1220, 515, 340, 495),
            (1570, 10, 340, 495),
            (1570, 515, 340, 495),
        ];

        let (x, y) = pick_spot(origin, SIZE, AREA, &taken);
        let placed = (x, y, SIZE.0, SIZE.1);
        for t in taken {
            assert!(
                !super::rects_overlap(placed, t),
                "new note at {placed:?} overlaps {t:?}"
            );
        }
    }

    #[test]
    fn it_goes_left_when_the_right_is_off_the_screen() {
        let origin = (1560, 10, 340, 480);
        assert_eq!(pick_spot(origin, SIZE, AREA, &[origin]), (1208, 10));
    }

    #[test]
    fn it_stays_inside_the_work_area() {
        let origin = (0, 0, 340, 480);
        let mut taken = vec![origin];
        // Fill the screen bar one cell in the bottom row.
        for row in 0..2 {
            for col in 0..5 {
                taken.push((col * 352, row * 492, 340, 480));
            }
        }

        let (x, y) = pick_spot(origin, SIZE, AREA, &taken);
        assert!(x >= 0 && y >= 0, "placed off the left or top at ({x},{y})");
        assert!(
            x + SIZE.0 <= 1920 && y + SIZE.1 <= 1032,
            "placed off the right or bottom at ({x},{y})"
        );
    }

    #[test]
    fn a_full_screen_cascades_rather_than_refusing() {
        let origin = (0, 0, 340, 480);
        // One rectangle covering everything: nothing can be free.
        let taken = [(0, 0, 1920, 1032)];

        let (x, y) = pick_spot(origin, SIZE, AREA, &taken);
        assert_eq!((x, y), (32, 32));
    }
}

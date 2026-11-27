//! Evidence for the freezes.
//!
//! A hang is the one fault that cannot report itself. The main thread is the
//! thread that writes most of this log, so when it stops, the log stops too —
//! and what reaches the file is an ordinary line from some ordinary moment,
//! with nothing after it. That is exactly what the 7 November freeze left
//! behind, and from it nothing can be concluded: a deadlock, a crash and a
//! power cut all look the same.
//!
//! Everything here runs off the main thread, on purpose, so that it keeps
//! writing while the main thread is stuck.
//!
//! Three things are recorded.
//!
//! The **watchdog** asks the event loop a question it cannot get wrong — a
//! closure that sends an empty message back — and gives it a deadline. A miss
//! is logged as it happens, with how long the stall has lasted, and the
//! recovery is logged too. The freeze stops being invisible.
//!
//! A **gap in the wall clock** between two beats means the machine was
//! suspended. A watchdog that beats every two seconds and finds that eight
//! hours have passed was asleep, not slow. This matters here because the
//! freezes follow waking the machine, and a log that says "asleep for 8h"
//! immediately before "the event loop has not answered for 30s" is a
//! different kind of evidence from either line alone.
//!
//! A **session mark** says whether the last run ended by being asked to. A
//! process that is killed, or that is still wedged when the user gives up on
//! it, never writes its clean marker — so the next launch can say so, instead
//! of the whole event leaving no trace at all.
//!
//! Nothing here records task content. The report is behaviour, versions and
//! geometry; what the notes say stays out of it, as it does out of the log
//! (ARCHITECTURE §7.3).

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// How often the event loop is asked whether it is alive.
const BEAT: Duration = Duration::from_secs(2);

/// How long it may take to answer before the beat counts as missed.
///
/// Generous. A busy event loop painting five webviews can be slow without
/// being stuck, and a watchdog that cries at every slow frame teaches you to
/// ignore it.
const BEAT_DEADLINE: Duration = Duration::from_secs(5);

/// A gap between beats longer than this means the machine was suspended
/// rather than merely busy. Nothing short of sleep loses a whole minute.
const SLEEP_GAP: Duration = Duration::from_secs(60);

/* -- what the watchdog concludes ------------------------------------------ */

/// The state of the event loop as of one beat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Health {
    /// Answered, and was answering before.
    Fine,
    /// Did not answer. Carries how long it has now been silent.
    Stalled(Duration),
    /// Answered, after a run of misses. Carries how long the stall lasted.
    Recovered(Duration),
}

/// What one beat means, given what the last ones meant.
///
/// Pure, so the interesting part can be tested without an event loop to
/// starve: `silent_for` is how long the loop has been failing to answer, and
/// is `None` when the previous beat was answered.
pub fn judge_beat(answered: bool, silent_for: Option<Duration>) -> Health {
    match (answered, silent_for) {
        (true, Some(stalled)) => Health::Recovered(stalled),
        (true, None) => Health::Fine,
        (false, Some(silent)) => Health::Stalled(silent),
        // The first miss: silent for one deadline's worth, by definition.
        (false, None) => Health::Stalled(BEAT_DEADLINE),
    }
}

/// Whether the time between two beats is better explained by sleep than load.
///
/// The beat interval is a parameter rather than the constant so the boundary
/// can be tested; a gap is only suspicious relative to how often we expected
/// to run.
pub fn sleep_suspected(gap: Duration, threshold: Duration) -> bool {
    gap >= threshold
}

/* -- how the last session ended ------------------------------------------- */

/// Written at startup, rewritten on a clean exit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMark {
    pub version: String,
    /// Unix seconds. A timestamp, not a date string: this is read by code.
    pub started_at: u64,
    /// False until the app is asked to quit and gets as far as saying so.
    pub ended_cleanly: bool,
}

/// What to say about the run before this one.
#[derive(Debug, Clone, PartialEq)]
pub enum LastSession {
    /// No mark on disk: a first run, or a version that did not write one.
    Unknown,
    /// It was asked to quit, and it did.
    Clean,
    /// It was not. Killed, cut off, or still wedged when the user gave up.
    Unclean { version: String, started_at: u64 },
}

/// Reads the mark left by the previous run.
pub fn judge_previous(previous: Option<SessionMark>) -> LastSession {
    match previous {
        None => LastSession::Unknown,
        Some(mark) if mark.ended_cleanly => LastSession::Clean,
        Some(mark) => LastSession::Unclean {
            version: mark.version,
            started_at: mark.started_at,
        },
    }
}

impl LastSession {
    /// One line for the panel. Short, and not alarming about a first run.
    pub fn summary(&self) -> String {
        match self {
            LastSession::Unknown => "No record of a previous run.".to_string(),
            LastSession::Clean => "The last session closed normally.".to_string(),
            LastSession::Unclean { version, .. } => format!(
                "The last session ({version}) did not close normally — it was killed, \
                 cut off, or still frozen when it ended."
            ),
        }
    }
}

/* -- state the report reads ----------------------------------------------- */

/// Set once the previous mark has been read, so the panel can report it.
static LAST_SESSION: Mutex<Option<LastSession>> = Mutex::new(None);

/// Stalls seen this run, newest last. Capped: a wedged loop could otherwise
/// fill memory with its own complaints.
static STALLS: Mutex<Vec<String>> = Mutex::new(Vec::new());
const STALL_LIMIT: usize = 40;

/// Longest stall this run, in milliseconds. The single number worth having.
static WORST_STALL_MS: AtomicU64 = AtomicU64::new(0);

/// Whether a watchdog is already running, so a second call is harmless.
static WATCHING: AtomicBool = AtomicBool::new(false);

fn note_event(line: String) {
    if let Ok(mut stalls) = STALLS.lock() {
        if stalls.len() >= STALL_LIMIT {
            stalls.remove(0);
        }
        stalls.push(line);
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/* -- the session mark on disk --------------------------------------------- */

fn mark_path(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_config_dir()
        .ok()
        .map(|dir| dir.join("session.json"))
}

fn write_mark(app: &AppHandle, mark: &SessionMark) {
    let Some(path) = mark_path(app) else { return };
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(mark) {
        Ok(json) => {
            if let Err(err) = fs::write(&path, json) {
                log::warn!("diagnostics: could not write the session mark: {err}");
            }
        }
        Err(err) => log::warn!("diagnostics: could not serialise the session mark: {err}"),
    }
}

/// Reads the previous mark, says what it means, and claims the file for this
/// run. Call once, early.
pub fn open_session(app: &AppHandle) {
    let previous = mark_path(app)
        .and_then(|path| fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<SessionMark>(&text).ok());

    let verdict = judge_previous(previous);
    match &verdict {
        LastSession::Unclean {
            version,
            started_at,
        } => log::warn!(
            "diagnostics: the previous session ({version}, started at unix {started_at}) \
             did not shut down cleanly"
        ),
        LastSession::Clean => log::info!("diagnostics: the previous session closed normally"),
        LastSession::Unknown => log::info!("diagnostics: no previous session on record"),
    }
    if let Ok(mut slot) = LAST_SESSION.lock() {
        *slot = Some(verdict);
    }

    write_mark(
        app,
        &SessionMark {
            version: app.package_info().version.to_string(),
            started_at: now_secs(),
            ended_cleanly: false,
        },
    );
}

/// Records that this run is ending on purpose. Call on the way out.
pub fn close_session(app: &AppHandle) {
    write_mark(
        app,
        &SessionMark {
            version: app.package_info().version.to_string(),
            started_at: now_secs(),
            ended_cleanly: true,
        },
    );
}

/* -- what was in flight when it stalled ----------------------------------- */

/// Blocking calls currently waiting on the event loop.
///
/// "The loop stalled" is half a finding. "The loop stalled while the screen
/// watcher was asking for the monitor list" is the whole one, and it is the
/// difference between knowing there is a deadlock and knowing where it is.
/// Every call into the loop from a background thread marks itself here first.
static IN_FLIGHT: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

/// Marks a blocking call as in progress for as long as it is held.
///
/// Returned rather than taking a closure so it reads like a lock guard, which
/// is what it behaves like: the entry goes away when the call returns, by any
/// path including a panic.
pub struct Activity(&'static str);

impl Drop for Activity {
    fn drop(&mut self) {
        if let Ok(mut flight) = IN_FLIGHT.lock() {
            if let Some(at) = flight.iter().position(|name| *name == self.0) {
                flight.remove(at);
            }
        }
    }
}

/// Records that `what` is waiting on the event loop until the guard is dropped.
pub fn in_flight(what: &'static str) -> Activity {
    if let Ok(mut flight) = IN_FLIGHT.lock() {
        flight.push(what);
    }
    Activity(what)
}

/// What is waiting right now, for the stall line to name.
fn in_flight_now() -> String {
    match IN_FLIGHT.lock() {
        Ok(flight) if flight.is_empty() => "nothing waiting on it".to_string(),
        Ok(flight) => flight.join(", "),
        // The lock itself being unavailable is worth saying: it means a
        // thread died holding it, which is its own kind of answer.
        Err(_) => "unknown (the activity list is poisoned)".to_string(),
    }
}

/// What was concluded about the previous run, for the panel to show.
pub fn last_session() -> LastSession {
    LAST_SESSION
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or(LastSession::Unknown)
}

/* -- the watchdog --------------------------------------------------------- */

/// Starts the thread that notices when the main thread stops.
///
/// The probe is `run_on_main_thread` with a closure that sends an empty
/// message back. It needs no window, touches no state, and cannot fail for
/// any reason except the one being tested: that the event loop is not running
/// the work handed to it.
pub fn watch_event_loop(app: &AppHandle) {
    if WATCHING.swap(true, Ordering::Relaxed) {
        return;
    }
    let app = app.clone();

    std::thread::spawn(move || {
        let mut silent_since: Option<std::time::Instant> = None;
        let mut last_beat = std::time::Instant::now();

        loop {
            std::thread::sleep(BEAT);

            // Before anything else: did the machine sleep through that pause?
            // A suspended process is not a stalled one, and the next beat will
            // very likely miss for reasons that have nothing to do with a bug.
            let gap = last_beat.elapsed();
            last_beat = std::time::Instant::now();
            if sleep_suspected(gap, SLEEP_GAP) {
                let line = format!(
                    "watchdog: {}s passed between beats — the machine was suspended",
                    gap.as_secs()
                );
                log::warn!("{line}");
                note_event(line);
                // The stall clock restarts: anything measured across a sleep
                // is measuring the sleep.
                silent_since = None;
                continue;
            }

            let (tx, rx) = std::sync::mpsc::channel();
            let posted = app.run_on_main_thread(move || {
                let _ = tx.send(());
            });
            if posted.is_err() {
                // The runtime is gone; so is the point of watching.
                return;
            }

            let answered = rx.recv_timeout(BEAT_DEADLINE).is_ok();
            let silent_for = silent_since.map(|since| since.elapsed());

            match judge_beat(answered, silent_for) {
                Health::Fine => {}
                Health::Stalled(silent) => {
                    if silent_since.is_none() {
                        silent_since = Some(std::time::Instant::now());
                    }
                    let line = format!(
                        "watchdog: the event loop has not answered for {}s; waiting on it: {}",
                        silent.as_secs(),
                        in_flight_now()
                    );
                    log::error!("{line}");
                    note_event(line);
                }
                Health::Recovered(stalled) => {
                    silent_since = None;
                    let ms = stalled.as_millis() as u64;
                    WORST_STALL_MS.fetch_max(ms, Ordering::Relaxed);
                    let line = format!(
                        "watchdog: the event loop answered again after {}s",
                        stalled.as_secs()
                    );
                    log::warn!("{line}");
                    note_event(line);
                }
            }
        }
    });
}

/* -- the report ----------------------------------------------------------- */

/// Where the log files live, for the button that opens the folder.
pub fn log_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_log_dir().ok()
}

/// The newest log file in the folder, by modified time.
fn newest_log(app: &AppHandle) -> Option<PathBuf> {
    let dir = log_dir(app)?;
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = fs::read_dir(dir)
        .ok()?
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            if path.extension()?.to_str()? != "log" {
                return None;
            }
            Some((path.metadata().ok()?.modified().ok()?, path))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files.pop().map(|(_, path)| path)
}

/// The last `lines` lines of a file, without reading the whole thing into
/// memory twice. Log files are capped at 2MB, so this stays honest.
fn tail(path: &PathBuf, lines: usize) -> String {
    let Ok(text) = fs::read_to_string(path) else {
        return "(the log file could not be read)".to_string();
    };
    let all: Vec<&str> = text.lines().collect();
    let start = all.len().saturating_sub(lines);
    all[start..].join("\n")
}

/// Everything worth having when something has gone wrong, as plain text.
///
/// Deliberately a single block the user can read before sending it anywhere.
/// A report you cannot inspect is one you cannot consent to.
pub fn report(app: &AppHandle, log_lines: usize) -> String {
    let mut out = String::new();
    out.push_str("Sticky Widget — diagnostic report\n");
    out.push_str(&format!("version: {}\n", app.package_info().version));
    out.push_str(&format!("generated: unix {}\n", now_secs()));
    out.push_str(&format!(
        "os: {} {}\n",
        std::env::consts::OS,
        std::env::consts::ARCH
    ));

    let last = LAST_SESSION
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
        .unwrap_or(LastSession::Unknown);
    out.push_str(&format!("previous session: {}\n", last.summary()));

    let worst = WORST_STALL_MS.load(Ordering::Relaxed);
    out.push_str(&format!(
        "longest stall this run: {}\n",
        if worst == 0 {
            "none recorded".to_string()
        } else {
            format!("{worst}ms")
        }
    ));

    out.push_str("\n-- displays --\n");
    match app.available_monitors() {
        Ok(monitors) if !monitors.is_empty() => {
            for monitor in monitors {
                let size = monitor.size();
                let pos = monitor.position();
                out.push_str(&format!(
                    "{}  {}x{} at {},{}  scale {:.2}\n",
                    monitor.name().cloned().unwrap_or_else(|| "?".into()),
                    size.width,
                    size.height,
                    pos.x,
                    pos.y,
                    monitor.scale_factor()
                ));
            }
        }
        Ok(_) => out.push_str("(none reported)\n"),
        // Worth saying out loud: this call is one of the ones that hangs, and
        // a report that reaches this line having failed is itself a finding.
        Err(err) => out.push_str(&format!("(could not be read: {err})\n")),
    }

    out.push_str("\n-- windows --\n");
    for (label, window) in app.webview_windows() {
        let pos = window
            .outer_position()
            .map(|p| format!("{},{}", p.x, p.y))
            .unwrap_or_else(|_| "?".into());
        let size = window
            .outer_size()
            .map(|s| format!("{}x{}", s.width, s.height))
            .unwrap_or_else(|_| "?".into());
        let visible = window
            .is_visible()
            .map(|v| v.to_string())
            .unwrap_or_else(|_| "?".into());
        out.push_str(&format!("{label}  at {pos}  {size}  visible {visible}\n"));
    }

    out.push_str(&format!(
        "waiting on the event loop now: {}\n",
        in_flight_now()
    ));

    out.push_str("\n-- watchdog events this run --\n");
    match STALLS.lock() {
        Ok(stalls) if stalls.is_empty() => out.push_str("(none)\n"),
        Ok(stalls) => {
            for line in stalls.iter() {
                out.push_str(line);
                out.push('\n');
            }
        }
        Err(_) => out.push_str("(unavailable)\n"),
    }

    out.push_str(&format!("\n-- last {log_lines} log lines --\n"));
    match newest_log(app) {
        Some(path) => {
            out.push_str(&format!("from {}\n", path.display()));
            out.push_str(&tail(&path, log_lines));
            out.push('\n');
        }
        None => out.push_str("(no log file found)\n"),
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_loop_that_answers_is_fine() {
        assert_eq!(judge_beat(true, None), Health::Fine);
    }

    /// The first miss has nothing to measure from, and must still be reported
    /// rather than waiting for a second one to establish a baseline.
    #[test]
    fn the_first_miss_is_reported_at_the_deadline() {
        assert_eq!(judge_beat(false, None), Health::Stalled(BEAT_DEADLINE));
    }

    #[test]
    fn a_continuing_stall_reports_how_long_it_has_lasted() {
        let silent = Duration::from_secs(30);
        assert_eq!(judge_beat(false, Some(silent)), Health::Stalled(silent));
    }

    /// The recovery line is the one that distinguishes a deadlock from a
    /// pause, so it must not be swallowed as an ordinary good beat.
    #[test]
    fn coming_back_is_reported_with_the_length_of_the_stall() {
        let stalled = Duration::from_secs(12);
        assert_eq!(judge_beat(true, Some(stalled)), Health::Recovered(stalled));
    }

    #[test]
    fn a_long_gap_between_beats_reads_as_sleep() {
        assert!(sleep_suspected(Duration::from_secs(3600), SLEEP_GAP));
        assert!(sleep_suspected(SLEEP_GAP, SLEEP_GAP));
    }

    /// A slow beat is not a suspended machine. Confusing the two would blame
    /// sleep for every busy moment and hide the stalls worth seeing.
    #[test]
    fn an_ordinary_slow_beat_does_not(
    ) {
        assert!(!sleep_suspected(Duration::from_secs(6), SLEEP_GAP));
    }

    #[test]
    fn no_mark_means_nothing_is_claimed_about_the_last_run() {
        assert_eq!(judge_previous(None), LastSession::Unknown);
        assert!(LastSession::Unknown.summary().contains("No record"));
    }

    #[test]
    fn a_mark_left_unfinished_means_it_did_not_close() {
        let mark = SessionMark {
            version: "0.1.9".into(),
            started_at: 1_700_000_000,
            ended_cleanly: false,
        };
        assert_eq!(
            judge_previous(Some(mark)),
            LastSession::Unclean {
                version: "0.1.9".into(),
                started_at: 1_700_000_000
            }
        );
    }

    #[test]
    fn a_finished_mark_means_it_closed() {
        let mark = SessionMark {
            version: "0.1.9".into(),
            started_at: 1_700_000_000,
            ended_cleanly: true,
        };
        assert_eq!(judge_previous(Some(mark)), LastSession::Clean);
    }

    /// The version is in the sentence because "which build froze" is the
    /// first question asked of an unclean exit.
    #[test]
    fn an_unclean_exit_names_the_version_that_froze() {
        let verdict = LastSession::Unclean {
            version: "0.1.6".into(),
            started_at: 1,
        };
        assert!(verdict.summary().contains("0.1.6"));
    }
}

#[cfg(test)]
mod activity_tests {
    use super::*;

    /// The point of the guard: a blocking call that returns stops being
    /// blamed for the next stall.
    #[test]
    fn a_finished_call_stops_being_in_flight() {
        let name = "test: a finished call";
        {
            let _busy = in_flight(name);
            assert!(in_flight_now().contains(name));
        }
        assert!(!in_flight_now().contains(name));
    }

    /// Several threads can be waiting at once, and the stall line should name
    /// all of them rather than whichever happened to be first.
    #[test]
    fn concurrent_calls_are_all_named() {
        let _one = in_flight("test: first call");
        let _two = in_flight("test: second call");
        let now = in_flight_now();
        assert!(now.contains("test: first call"));
        assert!(now.contains("test: second call"));
    }

    /// Nested guards for the same call must not remove each other's entry
    /// early, or a long outer call would look finished.
    #[test]
    fn the_same_call_twice_is_tracked_twice() {
        let name = "test: repeated call";
        let outer = in_flight(name);
        {
            let _inner = in_flight(name);
        }
        assert!(in_flight_now().contains(name));
        drop(outer);
        assert!(!in_flight_now().contains(name));
    }
}

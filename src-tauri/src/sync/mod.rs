//! Background sync.
//!
//! One thread owns the polling loop. Everything else — window focus, a manual
//! refresh, a list change, a completed write — pokes it through a channel
//! rather than syncing directly, so there is exactly one place that decides
//! when to talk to Google.

pub mod backoff;
pub mod schedule;

use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use crate::google::errors::ApiError;
use crate::tasks_commands;
use backoff::Backoff;
use schedule::cadence_for;

#[derive(Debug)]
enum Command {
    /// Sync as soon as possible — focus, manual refresh, list switch.
    Wake,
    Stop,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    /// "synced" | "syncing" | "offline" | "error"
    pub state: String,
    pub last_synced_at: Option<String>,
    pub message: Option<String>,
}

#[derive(Debug)]
struct Shared {
    /// Every list with a note window open. Polling is per-list, so two notes on
    /// the same list cost one request, not two.
    lists: Vec<String>,
    /// True when *any* note is visible. The cadence describes the app, not one
    /// window — a single visible note is reason enough to stay responsive.
    visible: bool,
    last_activity: Option<Instant>,
    last_synced_at: Option<String>,
}

pub struct SyncManager {
    tx: Mutex<Option<Sender<Command>>>,
    shared: std::sync::Arc<Mutex<Shared>>,
}

impl SyncManager {
    pub fn new() -> Self {
        Self {
            tx: Mutex::new(None),
            shared: std::sync::Arc::new(Mutex::new(Shared {
                lists: Vec::new(),
                visible: true,
                last_activity: Some(Instant::now()),
                last_synced_at: None,
            })),
        }
    }

    pub fn set_list(&self, list_id: Option<String>) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.lists = list_id.into_iter().collect();
        }
        self.wake();
    }

    /// Re-reads the set of lists to poll from the open notes.
    ///
    /// Called whenever a note opens or closes, so a closed note stops costing
    /// requests and a new one starts syncing immediately.
    pub fn refresh_lists(&self, app: &AppHandle) {
        let mut lists = app.state::<crate::notes::NoteRegistry>().active_lists();

        // `main` is not in the registry — it follows the persisted selection
        // rather than being pinned to a list.
        if let Ok(settings) = app.state::<crate::settings::AppState>().settings.lock() {
            if let Some(selected) = settings.selected_task_list_id.clone() {
                lists.push(selected);
            }
        }
        lists.sort();
        lists.dedup();

        if let Ok(mut shared) = self.shared.lock() {
            shared.lists = lists;
        }
    }

    pub fn set_visible(&self, visible: bool) {
        let changed = match self.shared.lock() {
            Ok(mut shared) => {
                let was = shared.visible;
                shared.visible = visible;
                if visible {
                    shared.last_activity = Some(Instant::now());
                }
                was != visible
            }
            Err(_) => false,
        };

        // Becoming visible is the moment perceived lag matters most — sync now
        // rather than waiting out whatever remains of the hidden interval.
        if changed && visible {
            self.wake();
        }
    }

    pub fn note_activity(&self) {
        if let Ok(mut shared) = self.shared.lock() {
            shared.last_activity = Some(Instant::now());
        }
    }

    pub fn wake(&self) {
        if let Ok(guard) = self.tx.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(Command::Wake);
            }
        }
    }

    pub fn stop(&self) {
        if let Ok(guard) = self.tx.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(Command::Stop);
            }
        }
    }

    /// Starts the polling thread. Safe to call once, from setup.
    pub fn start(&self, app: AppHandle) {
        let (tx, rx) = mpsc::channel();
        if let Ok(mut guard) = self.tx.lock() {
            *guard = Some(tx);
        }

        let shared = self.shared.clone();

        std::thread::spawn(move || {
            let mut backoff = Backoff::default();
            let mut retry_after: Option<u64> = None;

            loop {
                let (lists, base_interval) = {
                    let Ok(state) = shared.lock() else { break };
                    let cadence =
                        cadence_for(state.visible, state.last_activity, Instant::now());
                    (state.lists.clone(), cadence.interval())
                };

                let wait = backoff.delay(base_interval, retry_after);

                match rx.recv_timeout(wait) {
                    Ok(Command::Stop) | Err(RecvTimeoutError::Disconnected) => break,
                    // Both a wake and a timeout mean the same thing: sync now.
                    Ok(Command::Wake) | Err(RecvTimeoutError::Timeout) => {}
                }

                if lists.is_empty() {
                    // No note open on any list; nothing to poll for.
                    continue;
                }

                emit(&app, "syncing", &shared, None);

                // One failure must not hide the others: sync every list, then
                // report the first problem. Stopping at the first error would
                // let one broken list starve the rest.
                let mut first_error: Option<ApiError> = None;
                for list_id in &lists {
                    if let Err(err) = tasks_commands::sync_list(&app, list_id, false) {
                        log::warn!("sync failed for list {list_id}: {err:?}");
                        if first_error.is_none() {
                            first_error = Some(err);
                        }
                    }
                }

                match first_error {
                    None => {
                        backoff.record_success();
                        retry_after = None;

                        let stamp = iso_now();
                        if let Ok(mut state) = shared.lock() {
                            state.last_synced_at = Some(stamp);
                        }
                        emit(&app, "synced", &shared, None);
                    }
                    Some(err) => {
                        backoff.record_failure();
                        retry_after = match &err {
                            ApiError::RateLimited { retry_after_secs } => *retry_after_secs,
                            _ => None,
                        };

                        let state_name = match err {
                            ApiError::Network => "offline",
                            _ => "error",
                        };
                        emit(&app, state_name, &shared, Some(err.user_message()));
                    }
                }
            }

            log::info!("sync loop stopped");
        });
    }
}

impl Default for SyncManager {
    fn default() -> Self {
        Self::new()
    }
}

fn emit(
    app: &AppHandle,
    state: &str,
    shared: &std::sync::Arc<Mutex<Shared>>,
    message: Option<String>,
) {
    let last_synced_at = shared.lock().ok().and_then(|s| s.last_synced_at.clone());
    let _ = app.emit(
        "sync:status",
        SyncStatus {
            state: state.to_string(),
            last_synced_at,
            message,
        },
    );
}

fn iso_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    tasks_commands::format_epoch_utc_pub(secs)
}

/* -- Commands ------------------------------------------------------------- */

/// Called by the UI whenever the user does something in the window, so the
/// scheduler can tell an actively-used widget from one sitting open all day.
#[tauri::command]
pub fn note_activity(app: AppHandle) {
    app.state::<SyncManager>().note_activity();
}

#[tauri::command]
pub fn request_sync(app: AppHandle) {
    app.state::<SyncManager>().wake();
}

/// Polling interval in seconds for the current state, so the UI can explain
/// itself rather than leaving the user guessing how fresh the panel is.
#[tauri::command]
pub fn sync_interval_secs(app: AppHandle) -> u64 {
    let manager = app.state::<SyncManager>();
    let Ok(state) = manager.shared.lock() else {
        return schedule::IDLE.as_secs();
    };
    cadence_for(state.visible, state.last_activity, Instant::now())
        .interval()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_manager_polls_nothing_and_assumes_visible() {
        let manager = SyncManager::new();
        let state = manager.shared.lock().unwrap();
        assert!(state.lists.is_empty());
        assert!(state.visible);
    }

    #[test]
    fn set_list_is_recorded() {
        let manager = SyncManager::new();
        manager.set_list(Some("list-1".into()));
        assert_eq!(manager.shared.lock().unwrap().lists, vec!["list-1".to_string()]);
    }

    #[test]
    fn hiding_and_showing_tracks_visibility_and_refreshes_activity() {
        let manager = SyncManager::new();

        manager.set_visible(false);
        assert!(!manager.shared.lock().unwrap().visible);

        manager.set_visible(true);
        let state = manager.shared.lock().unwrap();
        assert!(state.visible);
        assert!(
            state.last_activity.is_some(),
            "becoming visible counts as activity"
        );
    }

    #[test]
    fn waking_without_a_started_thread_is_harmless() {
        // Ordering during setup should never be able to panic the app.
        let manager = SyncManager::new();
        manager.wake();
        manager.stop();
    }
}

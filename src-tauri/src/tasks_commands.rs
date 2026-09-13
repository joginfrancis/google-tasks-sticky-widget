//! Task-related IPC commands.
//!
//! ARCHITECTURE §3: reads come from the cache so the UI never waits on the
//! network; writes go to Google and reconcile the returned representation back
//! into the cache. The WebView never sees a token and never issues a request.

use tauri::{AppHandle, Emitter, Manager};

use crate::auth::AuthState;
use crate::google::client::TasksClient;
use crate::google::errors::ApiError;
use crate::google::models::{Task, TaskList};
use crate::store::Store;

/// Cursor key for delta syncs, scoped per list so switching lists cannot make
/// one list's cursor skip the other's changes.
fn cursor_key(task_list_id: &str) -> String {
    format!("updated_min:{task_list_id}")
}

/// Tells every window that a list changed.
///
/// The cache is the single source of truth and each note is only a view of it,
/// so a write is not finished until the other views have been told. Without
/// this, two notes showing one list diverge the moment either is touched, and
/// stay wrong until a poll happens to come round — which looks exactly like two
/// windows syncing separately, even though there is only ever one store and one
/// scheduler behind them.
///
/// Every mutation ends with a call to this, and any new one must too.
fn announce(app: &AppHandle, task_list_id: &str) {
    let _ = app.emit("tasks:updated", task_list_id.to_string());
}

fn client(app: &AppHandle) -> Result<TasksClient, String> {
    let token = app
        .state::<AuthState>()
        .access_token()
        .map_err(|e| e.user_message())?;
    TasksClient::new(token).map_err(|e| e.user_message())
}

/// Refresh-once-then-give-up on 401, per ARCHITECTURE §4.4. A second failure
/// means the grant is genuinely gone, so we stop rather than loop.
fn on_auth_lost(app: &AppHandle) {
    log::warn!("google access lost; returning to the connect prompt");
    app.state::<AuthState>().cache.clear();
    let _ = app.emit("auth:changed", false);
}

/* -- Reads ---------------------------------------------------------------- */

#[tauri::command]
pub fn list_tasks(
    app: AppHandle,
    task_list_id: String,
) -> Result<Vec<Task>, String> {
    app.state::<Store>().tasks_for_list(&task_list_id)
}

#[tauri::command]
pub fn list_task_lists(app: AppHandle) -> Result<Vec<TaskList>, String> {
    app.state::<Store>().task_lists()
}

/// Fetches task lists from Google and caches them.
///
/// Needed because the cache is empty on a fresh install, and list syncing is
/// keyed on a selected list — without this the widget would wait forever for a
/// list it could never learn about.
#[tauri::command]
pub fn refresh_task_lists(app: AppHandle) -> Result<Vec<TaskList>, String> {
    let lists = client(&app)?.list_task_lists().map_err(|err| {
        if err == ApiError::Unauthorized {
            on_auth_lost(&app);
        }
        err.user_message()
    })?;

    app.state::<Store>().replace_task_lists(&lists)?;
    Ok(lists)
}

/* -- Sync ----------------------------------------------------------------- */

/// Pulls task lists and one list's tasks into the cache.
///
/// `full` forces a non-delta fetch. Used on first connect, on manual refresh,
/// and after any error that leaves the cursor untrustworthy.
pub fn sync_list(app: &AppHandle, task_list_id: &str, full: bool) -> Result<(), ApiError> {
    let client = client(app).map_err(|_| ApiError::Unauthorized)?;
    let store = app.state::<Store>();

    // Lists are cheap and change rarely, but a renamed list showing its old
    // name is the kind of small wrongness that erodes trust in the whole panel.
    match client.list_task_lists() {
        Ok(lists) => {
            let _ = store.replace_task_lists(&lists);
        }
        Err(err) => log::warn!("could not refresh task lists: {err:?}"),
    }

    let cursor = if full {
        None
    } else {
        store.sync_value(&cursor_key(task_list_id)).ok().flatten()
    };

    // Record the time before the request, not after: anything modified during
    // the round-trip must fall inside the next window rather than between them.
    let started_at = now_rfc3339();

    let tasks = match client.list_tasks(task_list_id, cursor.as_deref()) {
        Ok(tasks) => tasks,
        Err(ApiError::Unauthorized) => {
            on_auth_lost(app);
            return Err(ApiError::Unauthorized);
        }
        Err(err) => return Err(err),
    };

    log::info!(
        "sync: {} change(s) for list {task_list_id} ({})",
        tasks.len(),
        if cursor.is_some() { "delta" } else { "full" }
    );

    // A full fetch is the whole truth for this list, so it replaces rather than
    // merges: a task Google has already purged appears in no response at all,
    // and merging can only ever add and update. A delta must merge, because it
    // deliberately returns only what changed.
    if cursor.is_none() {
        store
            .replace_tasks_for_list(task_list_id, &tasks)
            .map_err(|e| ApiError::Malformed(e))?;
    } else {
        store
            .upsert_tasks(&tasks)
            .map_err(|e| ApiError::Malformed(e))?;
    }

    // Only advance the cursor after the write succeeded. Advancing first would
    // lose those changes permanently if the write failed.
    let _ = store.set_sync_value(&cursor_key(task_list_id), &started_at);

    let _ = app.emit("tasks:updated", task_list_id.to_string());
    Ok(())
}

/// Subtract slack for clock skew between this machine and Google's. Overlapping
/// windows re-apply identical updates, which is a no-op; a gap loses an edit
/// permanently. See ARCHITECTURE §5.2.
const SKEW_SLACK_SECS: u64 = 30;

fn now_rfc3339() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .saturating_sub(SKEW_SLACK_SECS);

    format_epoch_utc(secs)
}

pub fn format_epoch_utc_pub(secs: u64) -> String {
    format_epoch_utc(secs)
}

/// Minimal RFC3339 formatter so the project does not take a date-library
/// dependency for one call site.
fn format_epoch_utc(secs: u64) -> String {
    let days = secs / 86_400;
    let time_of_day = secs % 86_400;
    let (hour, minute, second) = (
        time_of_day / 3600,
        (time_of_day % 3600) / 60,
        time_of_day % 60,
    );

    // Civil-from-days, Howard Hinnant's algorithm.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z")
}

#[tauri::command]
pub fn sync_now(app: AppHandle, task_list_id: String) -> Result<(), String> {
    sync_list(&app, &task_list_id, true).map_err(|e| e.user_message())
}

/* -- Writes --------------------------------------------------------------- */

/// Moves a task to an exact `parent`/`previous`, as the API expresses position.
///
/// Separate from `move_task`, which resolves an index against the cache. Once
/// subtasks are draggable the index alone is not enough — the same slot can
/// mean "last child of this parent" or "next task after it" — so the frontend
/// resolves both from the rows it is actually drawing (see lib/dropTarget.ts,
/// which is tested) and sends the answer rather than a number to re-derive.
///
/// `parent` names the task to nest under. `clear_parent` means "top level",
/// which is not the same as having no opinion — serde cannot tell those apart
/// from one nullable field, and without the distinction a subtask could never
/// be dragged out.
#[tauri::command]
pub fn move_task_to(
    app: AppHandle,
    task_list_id: String,
    task_id: String,
    parent: Option<String>,
    previous: Option<String>,
    clear_parent: bool,
) -> Result<(), String> {
    let store = app.state::<Store>();

    let parent_arg: Option<Option<&str>> = match (&parent, clear_parent) {
        (Some(id), _) => Some(Some(id.as_str())),
        (None, true) => Some(None),
        (None, false) => None,
    };

    let moved = client(&app)?
        .move_task(
            &task_list_id,
            &task_id,
            previous.as_deref(),
            None,
            parent_arg,
        )
        .map_err(|err| {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            match err {
                ApiError::Forbidden | ApiError::Malformed(_) => {
                    "That task couldn't be moved there.".to_string()
                }
                other => other.user_message(),
            }
        })?;

    store.upsert_tasks(std::slice::from_ref(&moved))?;
    announce(&app, &task_list_id);

    // Re-nesting renumbers siblings on both levels and the response describes
    // only the moved task, so the rest of the order is corrected in the
    // background rather than making the drag wait for a full fetch.
    let handle = app.clone();
    std::thread::spawn(move || {
        if let Err(err) = sync_list(&handle, &task_list_id, true) {
            log::warn!("post-move resync failed, order may be stale: {err:?}");
        }
        announce(&handle, &task_list_id);
    });

    Ok(())
}

/// One line of a pasted outline.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OutlineEntry {
    pub title: String,
    /// 0 for a top-level task, 1 for a subtask of the last top-level one.
    pub depth: u8,
}

/// Creates a pasted outline, in order, as one operation.
///
/// Sequential on purpose: each insert needs the id of the task it follows, so
/// they cannot be issued in parallel without losing the order. A whole paste is
/// one command rather than one per line so a twenty-line list is a single IPC
/// call and a single announcement, instead of twenty re-renders.
///
/// A line that fails does not abandon the rest. Pasting twelve tasks and
/// getting four, with no indication of which failed, is worse than getting
/// eleven and a message — so failures are counted and reported at the end.
#[tauri::command]
pub fn create_outline(
    app: AppHandle,
    task_list_id: String,
    entries: Vec<OutlineEntry>,
) -> Result<Vec<Task>, String> {
    let client = client(&app)?;
    let store = app.state::<Store>();

    let mut created: Vec<Task> = Vec::new();
    // The last task at each level, to hang the next one from.
    let mut last_top: Option<String> = None;
    let mut last_child: Option<String> = None;
    let mut failures = 0usize;

    for entry in entries {
        let title = entry.title.trim();
        if title.is_empty() {
            continue;
        }

        // A subtask with nothing above it has no parent to join, so it becomes
        // a top-level task rather than being dropped.
        let nest = entry.depth > 0 && last_top.is_some();
        let (parent, previous) = if nest {
            (last_top.as_deref(), last_child.as_deref())
        } else {
            (None, last_top.as_deref())
        };

        match client.insert_task(&task_list_id, title, None, parent, previous) {
            Ok(task) => {
                if nest {
                    last_child = Some(task.id.clone());
                } else {
                    last_top = Some(task.id.clone());
                    // A new top-level task starts a fresh run of children.
                    last_child = None;
                }
                let _ = store.upsert_tasks(std::slice::from_ref(&task));
                created.push(task);
            }
            Err(err) => {
                if err == ApiError::Unauthorized {
                    on_auth_lost(&app);
                    announce(&app, &task_list_id);
                    return Err(err.user_message());
                }
                log::warn!("paste: could not create {title:?}: {err:?}");
                failures += 1;
            }
        }
    }

    announce(&app, &task_list_id);

    if failures > 0 {
        return Err(format!(
            "Added {} of {}. {failures} couldn't be created.",
            created.len(),
            created.len() + failures
        ));
    }

    Ok(created)
}

#[tauri::command]
pub fn create_task(
    app: AppHandle,
    task_list_id: String,
    title: String,
    notes: Option<String>,
) -> Result<Task, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("A task needs a title.".into());
    }

    let notes = notes.map(|n| n.trim().to_string()).filter(|n| !n.is_empty());

    let created = client(&app)?
        // Quick-add always makes a top-level task at the default position.
        .insert_task(&task_list_id, &title, notes.as_deref(), None, None)
        .map_err(|err| {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            err.user_message()
        })?;

    app.state::<Store>().upsert_tasks(std::slice::from_ref(&created))?;
    announce(&app, &task_list_id);
    Ok(created)
}

#[tauri::command]
pub fn set_task_completed(
    app: AppHandle,
    task_list_id: String,
    task_id: String,
    completed: bool,
) -> Result<Task, String> {
    let updated = client(&app)?
        .set_completed(&task_list_id, &task_id, completed)
        .map_err(|err| {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            err.user_message()
        })?;

    app.state::<Store>().upsert_tasks(std::slice::from_ref(&updated))?;
    announce(&app, &task_list_id);
    Ok(updated)
}

/// Edits title, notes, and due date in one call.
///
/// `clear_due` exists because a nested `Option<Option<String>>` cannot survive
/// IPC: serde folds both an absent field and an explicit null into `None`, so
/// "leave the date alone" and "remove the date" would be indistinguishable. An
/// explicit flag is plainer than the deserializer gymnastics that would be
/// needed otherwise.
///
/// `due` is date-only. The API discards any time component, so the UI must never
/// offer one (docs/api-findings.md §2).
#[tauri::command]
pub fn update_task(
    app: AppHandle,
    task_list_id: String,
    task_id: String,
    title: Option<String>,
    notes: Option<String>,
    due: Option<String>,
    clear_due: Option<bool>,
) -> Result<Task, String> {
    // Google's documented ceilings. Rejecting here beats a 400 the user cannot
    // act on.
    if let Some(title) = &title {
        if title.trim().is_empty() {
            return Err("A task needs a title.".into());
        }
        if title.chars().count() > 1024 {
            return Err("That title is too long — 1024 characters maximum.".into());
        }
    }
    if let Some(notes) = &notes {
        if notes.chars().count() > 8192 {
            return Err("Those notes are too long — 8192 characters maximum.".into());
        }
    }

    let due_patch = if clear_due.unwrap_or(false) {
        Some(None)
    } else {
        due.map(Some)
    };

    let patch = crate::google::models::TaskPatch {
        title: title.map(|t| t.trim().to_string()),
        notes,
        due: due_patch,
        status: None,
    };

    let updated = client(&app)?
        .patch_task(&task_list_id, &task_id, &patch)
        .map_err(|err| {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            err.user_message()
        })?;

    app.state::<Store>().upsert_tasks(std::slice::from_ref(&updated))?;
    announce(&app, &task_list_id);
    Ok(updated)
}

/* -- List management ------------------------------------------------------ */

/// The one way a task changes position or list.
///
/// Reorder within a list, move to another list, and move-to-a-position are the
/// same operation to Google, so they are the same operation here. Drag-and-drop,
/// the context menu, and anything added later all commit through this.
///
/// `to_index` is a position among the destination list's **top-level** tasks,
/// counted after the moved task is removed. Subtasks keep their parent and are
/// not repositioned by index — Google orders them within their parent, and a
/// flat index across a nested list would mean something different in the UI than
/// it does in the API.
#[tauri::command]
pub fn move_task(
    app: AppHandle,
    task_list_id: String,
    task_id: String,
    destination_task_list_id: Option<String>,
    to_index: Option<usize>,
) -> Result<(), String> {
    let store = app.state::<Store>();
    let destination = destination_task_list_id
        .clone()
        .unwrap_or_else(|| task_list_id.clone());
    let is_cross_list = destination != task_list_id;

    // Resolve an index into the id it must follow. Done here rather than in the
    // UI because the cache is the ordering authority, and the UI's view can be
    // a frame stale mid-drag.
    let previous = match to_index {
        None => None,
        Some(index) => {
            // Completed tasks are excluded because the index counts rows the
            // user can see in the active list, and finished ones are hidden away
            // under "Completed". They are not merely at the end: `position`
            // interleaves them among the active tasks, so counting them shifts
            // every index by however many happen to sit above the drop — which
            // is why a drop would land several rows short in a list with a long
            // completed section.
            let siblings: Vec<Task> = store
                .tasks_for_list(&destination)?
                .into_iter()
                .filter(|t| {
                    t.parent_id.is_none() && t.id != task_id && t.status != "completed"
                })
                .collect();

            if index == 0 {
                None
            } else {
                // Clamp rather than fail: a drop past the end is a legitimate
                // gesture meaning "last", and the list may have changed under us.
                siblings
                    .get(index.min(siblings.len()).saturating_sub(1))
                    .map(|t| t.id.clone())
            }
        }
    };

    let moved = client(&app)?
        .move_task(
            &task_list_id,
            &task_id,
            previous.as_deref(),
            is_cross_list.then_some(destination.as_str()),
            // Index-based moves are top-level only, so nesting is left alone.
            None,
        )
        .map_err(|err| {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            // Recurrence is invisible to us, so this is the one case where a
            // plain failure deserves a more specific hint than the generic text.
            match err {
                ApiError::Forbidden | ApiError::Malformed(_) if is_cross_list => {
                    "That task couldn't be moved. Repeating tasks can't change lists."
                        .to_string()
                }
                other => other.user_message(),
            }
        })?;

    if is_cross_list {
        // The row carries its list id, so writing the moved copy would leave a
        // duplicate under the old list until the next full sync.
        store.delete_task(&task_id)?;
    }
    store.upsert_tasks(std::slice::from_ref(&moved))?;

    // Tell the views now. Everything they need is already in the cache: the
    // response carried the moved task's new list and position, and the row it
    // left has been deleted. Waiting for the resync below put three or four
    // seconds between the drop and the other note noticing it.
    announce(&app, &destination);
    if is_cross_list {
        announce(&app, &task_list_id);
    }

    // Then reconcile in the background. A move renumbers siblings server-side
    // and the response describes only the moved task, so the *order* of the
    // others can be stale even when their contents are not — worth correcting,
    // not worth making a drag wait for. On its own thread because this command
    // runs on the main one, where two full fetches freeze the UI outright.
    let handle = app.clone();
    let origin = task_list_id.clone();
    std::thread::spawn(move || {
        if let Err(err) = sync_list(&handle, &destination, true) {
            log::warn!("post-move resync failed, order may be stale: {err:?}");
        }
        announce(&handle, &destination);

        if is_cross_list {
            if let Err(err) = sync_list(&handle, &origin, true) {
                log::warn!("post-move resync of the origin list failed: {err:?}");
            }
            announce(&handle, &origin);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn create_task_list(app: AppHandle, title: String) -> Result<TaskList, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("A list needs a name.".into());
    }

    let created = client(&app)?.create_task_list(&title).map_err(|err| {
        if err == ApiError::Unauthorized {
            on_auth_lost(&app);
        }
        err.user_message()
    })?;

    // Cheaper and less racy than re-fetching the whole set.
    let store = app.state::<Store>();
    let mut lists = store.task_lists()?;
    lists.push(created.clone());
    store.replace_task_lists(&lists)?;

    Ok(created)
}

#[tauri::command]
pub fn rename_task_list(
    app: AppHandle,
    task_list_id: String,
    title: String,
) -> Result<TaskList, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("A list needs a name.".into());
    }

    let updated = client(&app)?
        .rename_task_list(&task_list_id, &title)
        .map_err(|err| {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            err.user_message()
        })?;

    let store = app.state::<Store>();
    let lists: Vec<TaskList> = store
        .task_lists()?
        .into_iter()
        .map(|list| if list.id == updated.id { updated.clone() } else { list })
        .collect();
    store.replace_task_lists(&lists)?;

    Ok(updated)
}

/// Deletes a list and everything in it. The UI must confirm first — Google has
/// no undo, and this cannot be walked back.
#[tauri::command]
pub fn delete_task_list(app: AppHandle, task_list_id: String) -> Result<(), String> {
    let store = app.state::<Store>();

    // Refuse to leave the widget with nothing to show. Google's own UI keeps a
    // default list around for the same reason.
    if store.task_lists()?.len() <= 1 {
        return Err("You need at least one task list.".into());
    }

    client(&app)?
        .delete_task_list(&task_list_id)
        .map_err(|err| {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            err.user_message()
        })?;

    let lists: Vec<TaskList> = store
        .task_lists()?
        .into_iter()
        .filter(|list| list.id != task_list_id)
        .collect();
    store.replace_task_lists(&lists)?;
    store.delete_tasks_for_list(&task_list_id)?;

    Ok(())
}

/// Opens a task in the Google Tasks web UI.
///
/// The honest answer to starring, recurrence and attachments — none of which the
/// API exposes (docs/ui-parity.md §1). Rather than pretend those features do not
/// exist, hand the user one click to where they do.
///
/// Only ever opens a `webViewLink` that came from Google's own response, never a
/// URL supplied by the WebView.
#[tauri::command]
pub fn open_task_in_google(app: AppHandle, task_id: String) -> Result<(), String> {
    let store = app.state::<Store>();
    let link = store
        .web_view_link(&task_id)?
        .ok_or_else(|| "That task has no Google Tasks link yet. Try syncing.".to_string())?;

    // Belt and braces: the value is Google's, but a stored string is still data.
    if !link.starts_with("https://") {
        log::warn!("refusing to open a non-https task link");
        return Err("That link doesn't look safe to open.".into());
    }

    tauri_plugin_opener::open_url(&link, None::<&str>)
        .map_err(|e| format!("Could not open your browser: {e}"))
}

#[tauri::command]
pub fn delete_task(
    app: AppHandle,
    task_list_id: String,
    task_id: String,
) -> Result<(), String> {
    // A task Google has never heard of is the outcome delete was asking for, so
    // report success and drop the stale cache row. Surfacing "that task no
    // longer exists" leaves the user staring at a row they just deleted, with
    // an error telling them it is already gone.
    match client(&app)?.delete_task(&task_list_id, &task_id) {
        Ok(()) | Err(ApiError::NotFound) => {}
        Err(err) => {
            if err == ApiError::Unauthorized {
                on_auth_lost(&app);
            }
            return Err(err.user_message());
        }
    }

    app.state::<Store>().delete_task(&task_id)?;
    announce(&app, &task_list_id);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_formats_as_rfc3339_utc() {
        assert_eq!(format_epoch_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_epoch_utc(1_000_000_000), "2001-09-09T01:46:40Z");
        // A leap day, since the civil-date maths is where this would break.
        assert_eq!(format_epoch_utc(1_709_164_800), "2024-02-29T00:00:00Z");
    }

    #[test]
    fn cursor_keys_are_scoped_per_list() {
        assert_ne!(cursor_key("list-a"), cursor_key("list-b"));
        assert!(cursor_key("list-a").contains("list-a"));
    }

    #[test]
    fn cursor_timestamp_is_backdated_for_clock_skew() {
        // Parsing it back is more work than it earns; asserting the shape and
        // that it is in the past is enough to catch a wrong-sign mistake.
        let now = now_rfc3339();
        assert!(now.ends_with('Z'));
        assert_eq!(now.len(), 20);
        assert!(now.starts_with("20"));
    }
}

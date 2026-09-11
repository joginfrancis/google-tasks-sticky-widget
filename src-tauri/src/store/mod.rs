//! Local cache.
//!
//! ARCHITECTURE §8: this is a cache, not a source of truth. Google is
//! authoritative; the database exists so the widget paints instantly on launch
//! and survives an outage. Deleting the file loses nothing.
//!
//! No authentication material is ever stored here.

use std::path::PathBuf;
use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension};

use crate::google::models::{Task, TaskList};

mod migrations;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("could not create data directory: {e}"))?;
        }

        let conn = Connection::open(&path)
            .map_err(|e| format!("could not open local cache: {e}"))?;

        // WAL keeps reads from blocking behind a sync write, which is what
        // makes the instant-paint-on-launch promise hold.
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|e| format!("could not configure local cache: {e}"))?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(|e| format!("could not configure local cache: {e}"))?;

        migrations::run(&conn)?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| e.to_string())?;
        migrations::run(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn with_conn<T>(
        &self,
        f: impl FnOnce(&Connection) -> rusqlite::Result<T>,
    ) -> Result<T, String> {
        let guard = self
            .conn
            .lock()
            .map_err(|_| "local cache lock poisoned".to_string())?;
        f(&guard).map_err(|e| format!("local cache error: {e}"))
    }

    /* -- Task lists -------------------------------------------------------- */

    pub fn replace_task_lists(&self, lists: &[TaskList]) -> Result<(), String> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM task_lists", [])?;
            {
                let mut stmt = tx.prepare(
                    "INSERT INTO task_lists (id, title, updated) VALUES (?1, ?2, ?3)",
                )?;
                for list in lists {
                    stmt.execute(params![list.id, list.title, list.updated])?;
                }
            }
            tx.commit()
        })
    }

    pub fn task_lists(&self) -> Result<Vec<TaskList>, String> {
        self.with_conn(|conn| {
            let mut stmt =
                conn.prepare("SELECT id, title, updated FROM task_lists ORDER BY title")?;
            let rows = stmt.query_map([], |row| {
                Ok(TaskList {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    updated: row.get(2)?,
                })
            })?;
            rows.collect()
        })
    }

    /* -- Tasks ------------------------------------------------------------- */

    /// Applies a batch from the API. Deleted tasks are removed outright rather
    /// than kept as tombstones — Google is authoritative, so there is nothing
    /// to reconcile them against later.
    pub fn upsert_tasks(&self, tasks: &[Task]) -> Result<(), String> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                let mut delete = tx.prepare("DELETE FROM tasks WHERE id = ?1")?;
                let mut upsert = tx.prepare(
                    "INSERT INTO tasks (
                        id, task_list_id, parent_id, title, notes, due,
                        status, completed_at, position, updated, deleted,
                        web_view_link
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11)
                     ON CONFLICT(id) DO UPDATE SET
                        task_list_id  = excluded.task_list_id,
                        parent_id     = excluded.parent_id,
                        title         = excluded.title,
                        notes         = excluded.notes,
                        due           = excluded.due,
                        status        = excluded.status,
                        completed_at  = excluded.completed_at,
                        position      = excluded.position,
                        updated       = excluded.updated,
                        deleted       = 0,
                        web_view_link = excluded.web_view_link",
                )?;

                for task in tasks {
                    if task.deleted {
                        delete.execute(params![task.id])?;
                    } else {
                        upsert.execute(params![
                            task.id,
                            task.task_list_id,
                            task.parent_id,
                            task.title,
                            task.notes,
                            task.due,
                            task.status,
                            task.completed_at,
                            task.position,
                            task.updated,
                            task.web_view_link,
                        ])?;
                    }
                }
            }
            tx.commit()
        })
    }

    /// Applies a **full** fetch, treating it as the complete truth for the list.
    ///
    /// `upsert_tasks` can only remove what Google explicitly reports as
    /// `deleted`, and Google stops reporting a deletion once it purges the task
    /// for good — after which no response mentions the row ever again and the
    /// cached copy becomes immortal. The widget goes on showing a task that
    /// exists nowhere else, while reporting itself perfectly synced.
    ///
    /// Only sound for a full fetch: a delta returns a subset by design, so
    /// applying this to one would delete every task that merely had not changed.
    pub fn replace_tasks_for_list(
        &self,
        task_list_id: &str,
        tasks: &[Task],
    ) -> Result<(), String> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            {
                // Scoped to the list: a task moved to another list is still a
                // real task, and its new list owns it now.
                tx.execute(
                    "DELETE FROM tasks WHERE task_list_id = ?1",
                    params![task_list_id],
                )?;

                let mut insert = tx.prepare(
                    "INSERT OR REPLACE INTO tasks (
                        id, task_list_id, parent_id, title, notes, due,
                        status, completed_at, position, updated, deleted,
                        web_view_link
                     ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11)",
                )?;

                for task in tasks.iter().filter(|t| !t.deleted) {
                    insert.execute(params![
                        task.id,
                        task.task_list_id,
                        task.parent_id,
                        task.title,
                        task.notes,
                        task.due,
                        task.status,
                        task.completed_at,
                        task.position,
                        task.updated,
                        task.web_view_link,
                    ])?;
                }
            }
            tx.commit()
        })
    }

    pub fn tasks_for_list(&self, task_list_id: &str) -> Result<Vec<Task>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, task_list_id, parent_id, title, notes, due,
                        status, completed_at, position, updated, deleted,
                        web_view_link
                 FROM tasks
                 WHERE task_list_id = ?1 AND deleted = 0
                 ORDER BY position",
            )?;
            let rows = stmt.query_map(params![task_list_id], |row| {
                Ok(Task {
                    id: row.get(0)?,
                    task_list_id: row.get(1)?,
                    parent_id: row.get(2)?,
                    title: row.get(3)?,
                    notes: row.get(4)?,
                    due: row.get(5)?,
                    status: row.get(6)?,
                    completed_at: row.get(7)?,
                    position: row.get(8)?,
                    updated: row.get(9)?,
                    deleted: row.get::<_, i64>(10)? != 0,
                    web_view_link: row.get(11)?,
                })
            })?;
            rows.collect()
        })
    }

    /// Drops one list's cached tasks and its sync cursor, after the list itself
    /// has gone. Leaving them would strand rows no view can ever reach.
    pub fn delete_tasks_for_list(&self, task_list_id: &str) -> Result<(), String> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM tasks WHERE task_list_id = ?1",
                params![task_list_id],
            )?;
            tx.execute(
                "DELETE FROM sync_state WHERE key = ?1",
                params![format!("updated_min:{task_list_id}")],
            )?;
            tx.commit()
        })
    }

    pub fn web_view_link(&self, task_id: &str) -> Result<Option<String>, String> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT web_view_link FROM tasks WHERE id = ?1",
                params![task_id],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map(Option::flatten)
        })
    }

    pub fn delete_task(&self, task_id: &str) -> Result<(), String> {
        self.with_conn(|conn| {
            conn.execute("DELETE FROM tasks WHERE id = ?1", params![task_id])
                .map(|_| ())
        })
    }

    /// Clears cached content without touching settings. Used when the account
    /// is disconnected — leaving another account's tasks on screen would be
    /// both confusing and a small privacy leak.
    pub fn clear_tasks(&self) -> Result<(), String> {
        self.with_conn(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute("DELETE FROM tasks", [])?;
            tx.execute("DELETE FROM task_lists", [])?;
            tx.execute("DELETE FROM sync_state", [])?;
            tx.commit()
        })
    }

    /* -- Sync cursor ------------------------------------------------------- */

    pub fn sync_value(&self, key: &str) -> Result<Option<String>, String> {
        self.with_conn(|conn| {
            conn.query_row(
                "SELECT value FROM sync_state WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()
        })
    }

    pub fn set_sync_value(&self, key: &str, value: &str) -> Result<(), String> {
        self.with_conn(|conn| {
            conn.execute(
                "INSERT INTO sync_state (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
            .map(|_| ())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, status: &str) -> Task {
        Task {
            id: id.into(),
            task_list_id: "list1".into(),
            parent_id: None,
            title: format!("Task {id}"),
            notes: None,
            due: None,
            status: status.into(),
            completed_at: None,
            position: format!("pos-{id}"),
            updated: "2026-09-07T10:00:00.000Z".into(),
            deleted: false,
            web_view_link: None,
        }
    }

    #[test]
    fn upsert_inserts_then_updates_in_place() {
        let store = Store::in_memory().unwrap();
        store.upsert_tasks(&[task("a", "needsAction")]).unwrap();
        assert_eq!(store.tasks_for_list("list1").unwrap().len(), 1);

        let mut updated = task("a", "completed");
        updated.title = "Renamed".into();
        store.upsert_tasks(&[updated]).unwrap();

        let rows = store.tasks_for_list("list1").unwrap();
        assert_eq!(rows.len(), 1, "upsert must not duplicate");
        assert_eq!(rows[0].title, "Renamed");
        assert!(rows[0].is_completed());
    }

    #[test]
    fn deleted_tasks_are_removed_not_retained() {
        let store = Store::in_memory().unwrap();
        store.upsert_tasks(&[task("a", "needsAction")]).unwrap();

        let mut gone = task("a", "needsAction");
        gone.deleted = true;
        store.upsert_tasks(&[gone]).unwrap();

        assert!(store.tasks_for_list("list1").unwrap().is_empty());
    }

    /// The failure this guards against reported itself as a successful sync:
    /// Google purges a deleted task eventually and then names it in no response
    /// at all, so a merge — which can only add and update — left the row cached
    /// forever.
    #[test]
    fn a_full_sync_drops_rows_google_no_longer_returns() {
        let store = Store::in_memory().unwrap();
        store
            .upsert_tasks(&[task("a", "needsAction"), task("d", "needsAction")])
            .unwrap();

        // A full fetch that simply does not mention `d`.
        store
            .replace_tasks_for_list("list1", &[task("a", "needsAction")])
            .unwrap();

        let ids: Vec<String> = store
            .tasks_for_list("list1")
            .unwrap()
            .into_iter()
            .map(|t| t.id)
            .collect();
        assert_eq!(ids, vec!["a".to_string()]);
    }

    /// Replacing is scoped, or syncing one list would empty every other.
    #[test]
    fn a_full_sync_leaves_other_lists_alone() {
        let store = Store::in_memory().unwrap();
        let mut elsewhere = task("x", "needsAction");
        elsewhere.task_list_id = "list2".into();
        store
            .upsert_tasks(&[task("a", "needsAction"), elsewhere])
            .unwrap();

        store.replace_tasks_for_list("list1", &[]).unwrap();

        assert!(store.tasks_for_list("list1").unwrap().is_empty());
        assert_eq!(store.tasks_for_list("list2").unwrap().len(), 1);
    }

    #[test]
    fn a_task_completed_elsewhere_updates_rather_than_disappearing() {
        // The showHidden case end to end: the delta returns it as completed,
        // and the cached row must flip rather than linger as incomplete.
        let store = Store::in_memory().unwrap();
        store.upsert_tasks(&[task("a", "needsAction")]).unwrap();

        let mut done = task("a", "completed");
        done.completed_at = Some("2026-09-07T11:00:00.000Z".into());
        store.upsert_tasks(&[done]).unwrap();

        let rows = store.tasks_for_list("list1").unwrap();
        assert_eq!(rows.len(), 1);
        assert!(rows[0].is_completed());
        assert!(rows[0].completed_at.is_some());
    }

    #[test]
    fn tasks_are_scoped_to_their_list() {
        let store = Store::in_memory().unwrap();
        let mut other = task("b", "needsAction");
        other.task_list_id = "list2".into();
        store.upsert_tasks(&[task("a", "needsAction"), other]).unwrap();

        assert_eq!(store.tasks_for_list("list1").unwrap().len(), 1);
        assert_eq!(store.tasks_for_list("list2").unwrap().len(), 1);
    }

    #[test]
    fn deleting_a_list_removes_its_tasks_and_cursor_only() {
        let store = Store::in_memory().unwrap();
        let mut other = task("b", "needsAction");
        other.task_list_id = "list2".into();
        store.upsert_tasks(&[task("a", "needsAction"), other]).unwrap();
        store.set_sync_value("updated_min:list1", "x").unwrap();
        store.set_sync_value("updated_min:list2", "y").unwrap();

        store.delete_tasks_for_list("list1").unwrap();

        assert!(store.tasks_for_list("list1").unwrap().is_empty());
        assert_eq!(store.sync_value("updated_min:list1").unwrap(), None);

        // The other list must be untouched.
        assert_eq!(store.tasks_for_list("list2").unwrap().len(), 1);
        assert_eq!(
            store.sync_value("updated_min:list2").unwrap().as_deref(),
            Some("y")
        );
    }

    #[test]
    fn sync_cursor_round_trips_and_overwrites() {
        let store = Store::in_memory().unwrap();
        assert_eq!(store.sync_value("cursor").unwrap(), None);

        store.set_sync_value("cursor", "2026-09-07T10:00:00Z").unwrap();
        assert_eq!(
            store.sync_value("cursor").unwrap().as_deref(),
            Some("2026-09-07T10:00:00Z")
        );

        store.set_sync_value("cursor", "2026-09-07T11:00:00Z").unwrap();
        assert_eq!(
            store.sync_value("cursor").unwrap().as_deref(),
            Some("2026-09-07T11:00:00Z")
        );
    }

    #[test]
    fn clearing_removes_tasks_lists_and_cursor() {
        let store = Store::in_memory().unwrap();
        store.upsert_tasks(&[task("a", "needsAction")]).unwrap();
        store
            .replace_task_lists(&[TaskList {
                id: "list1".into(),
                title: "My Tasks".into(),
                updated: "2026-09-07T10:00:00.000Z".into(),
            }])
            .unwrap();
        store.set_sync_value("cursor", "x").unwrap();

        store.clear_tasks().unwrap();

        assert!(store.tasks_for_list("list1").unwrap().is_empty());
        assert!(store.task_lists().unwrap().is_empty());
        assert_eq!(store.sync_value("cursor").unwrap(), None);
    }
}

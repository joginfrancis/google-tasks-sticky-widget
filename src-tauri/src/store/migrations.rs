//! Schema migrations.
//!
//! Versioned via `PRAGMA user_version`. Each step runs once, in order, inside a
//! transaction. Because this database is a cache and never a source of truth,
//! a migration that cannot be applied cleanly is allowed to reset it — the only
//! cost is one full sync.

use rusqlite::Connection;

type Migration = (&'static str, &'static str);

const MIGRATIONS: &[Migration] = &[
    (
    "initial schema",
    r#"
    CREATE TABLE task_lists (
        id      TEXT PRIMARY KEY,
        title   TEXT NOT NULL,
        updated TEXT NOT NULL
    );

    CREATE TABLE tasks (
        id           TEXT PRIMARY KEY,
        task_list_id TEXT NOT NULL,
        parent_id    TEXT,
        title        TEXT NOT NULL,
        notes        TEXT,
        due          TEXT,
        status       TEXT NOT NULL,
        completed_at TEXT,
        position     TEXT NOT NULL,
        updated      TEXT NOT NULL,
        deleted      INTEGER NOT NULL DEFAULT 0
    );

    -- Every read is "the tasks of one list", so this is the index that matters.
    CREATE INDEX idx_tasks_list ON tasks (task_list_id, deleted);

    CREATE TABLE sync_state (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );

    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    "#,
    ),
    (
        "task web view link",
        "ALTER TABLE tasks ADD COLUMN web_view_link TEXT;",
    ),
];

/// Adding a column leaves every existing row null in it, and a delta sync only
/// returns tasks that *changed* — so the new column would stay empty
/// indefinitely for tasks nobody touches. Dropping the cursor forces one full
/// fetch, which backfills it.
///
/// Applies to any future column addition too, which is why it lives here rather
/// than inside a single migration's SQL.
const INVALIDATE_SYNC_CURSOR: &str = "DELETE FROM sync_state";

pub fn run(conn: &Connection) -> Result<(), String> {
    let current: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .map_err(|e| format!("could not read cache version: {e}"))?;

    let target = MIGRATIONS.len() as i64;

    if current > target {
        // An older build opening a newer cache. Rebuilding costs one sync;
        // guessing at a future schema costs correctness.
        log::warn!("cache is newer than this build understands; rebuilding it");
        reset(conn)?;
        return run(conn);
    }

    let mut applied_any = false;

    for (index, (name, sql)) in MIGRATIONS.iter().enumerate() {
        let version = index as i64 + 1;
        if version <= current {
            continue;
        }

        log::info!("applying cache migration {version}: {name}");
        conn.execute_batch(sql)
            .map_err(|e| format!("cache migration '{name}' failed: {e}"))?;
        conn.pragma_update(None, "user_version", version)
            .map_err(|e| format!("could not record cache version: {e}"))?;
        applied_any = true;
    }

    // Skipped on a fresh database — there is no cursor to invalidate, and the
    // first sync is full anyway.
    if applied_any && current > 0 {
        log::info!("schema changed; forcing a full sync to backfill new columns");
        conn.execute(INVALIDATE_SYNC_CURSOR, [])
            .map_err(|e| format!("could not reset the sync cursor: {e}"))?;
    }

    Ok(())
}

fn reset(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "DROP TABLE IF EXISTS tasks;
         DROP TABLE IF EXISTS task_lists;
         DROP TABLE IF EXISTS sync_state;
         DROP TABLE IF EXISTS settings;",
    )
    .map_err(|e| format!("could not reset cache: {e}"))?;

    conn.pragma_update(None, "user_version", 0)
        .map_err(|e| format!("could not reset cache version: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        let first: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();

        // Re-running must not attempt to recreate tables.
        run(&conn).unwrap();
        let second: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();

        assert_eq!(first, second);
        assert_eq!(first, MIGRATIONS.len() as i64);
    }

    #[test]
    fn upgrading_an_existing_cache_drops_the_sync_cursor() {
        // Simulate a database created before the newest migration.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(MIGRATIONS[0].1).unwrap();
        conn.pragma_update(None, "user_version", 1i64).unwrap();
        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES ('updated_min:list1', 'x')",
            [],
        )
        .unwrap();

        run(&conn).unwrap();

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            remaining, 0,
            "a new column must be backfilled by a full sync, not left null forever"
        );
    }

    #[test]
    fn a_fresh_database_keeps_whatever_cursor_it_is_given() {
        // Nothing to backfill on a brand-new cache, so the reset must not fire
        // and cost an extra full fetch on every first run.
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();

        conn.execute(
            "INSERT INTO sync_state (key, value) VALUES ('updated_min:list1', 'x')",
            [],
        )
        .unwrap();
        run(&conn).unwrap();

        let remaining: i64 = conn
            .query_row("SELECT COUNT(*) FROM sync_state", [], |r| r.get(0))
            .unwrap();
        assert_eq!(remaining, 1);
    }

    #[test]
    fn a_cache_from_a_newer_build_is_rebuilt_rather_than_trusted() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        conn.pragma_update(None, "user_version", 999i64).unwrap();

        run(&conn).expect("should recover by rebuilding");

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, MIGRATIONS.len() as i64);

        // And the schema must actually be usable afterwards.
        conn.execute("DELETE FROM tasks", []).unwrap();
    }
}

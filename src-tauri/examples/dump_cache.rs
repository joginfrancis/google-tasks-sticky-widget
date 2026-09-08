//! Diagnostic: print what the local cache actually holds.
//!
//! `cargo run --example dump_cache`
//!
//! Useful when the widget and Google disagree — it separates "we never fetched
//! it" from "we fetched it and stored it wrong".

use rusqlite::Connection;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var("USERPROFILE")
        .map(std::path::PathBuf::from)?
        .join("AppData/Roaming/in.startupmission.stickywidget/cache.sqlite");

    println!("cache: {}\n", path.display());
    let conn = Connection::open(&path)?;

    println!("-- task lists --");
    let mut stmt = conn.prepare("SELECT id, title FROM task_lists ORDER BY title")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (id, title) = row?;
        println!("  {title}  [{id}]");
    }

    println!("\n-- tasks --");
    let mut stmt = conn.prepare(
        "SELECT title, status, due, parent_id, position, updated
         FROM tasks WHERE deleted = 0 ORDER BY status, position",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    })?;

    for row in rows {
        let (title, status, due, parent, position, updated) = row?;
        let mark = if status == "completed" { "x" } else { " " };
        println!(
            "  [{mark}] {title:<45} due={:<26} parent={:<10} pos={position} upd={updated}",
            due.unwrap_or_else(|| "-".into()),
            parent.unwrap_or_else(|| "-".into()),
        );
    }

    println!("\n-- sync cursors --");
    let mut stmt = conn.prepare("SELECT key, value FROM sync_state")?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (key, value) = row?;
        println!("  {key} = {value}");
    }

    Ok(())
}

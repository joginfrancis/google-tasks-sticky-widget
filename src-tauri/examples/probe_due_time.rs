//! Empirical check: does the Tasks API keep a time on `due`?
//!
//! `cargo run --example probe_due_time`
//!
//! Creates a throwaway task in the first task list, patches it with an explicit
//! time, reads back exactly what the server stored, then deletes it. Prints the
//! raw JSON so the answer is observed rather than assumed.

use sticky_widget_lib::auth::{flow, tokens};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(refresh) = tokens::load_refresh_token()? else {
        eprintln!("Not connected — sign in through the widget first.");
        return Ok(());
    };

    let access = flow::refresh_access_token(&refresh)?;
    let token = access.expose().to_string();

    let http = reqwest::blocking::Client::new();
    let base = "https://tasks.googleapis.com/tasks/v1";

    // First list
    let lists: serde_json::Value = http
        .get(format!("{base}/users/@me/lists"))
        .bearer_auth(&token)
        .send()?
        .json()?;
    let list_id = lists["items"][0]["id"]
        .as_str()
        .ok_or("no task lists found")?
        .to_string();
    println!("list: {}\n", lists["items"][0]["title"]);

    // Create with a time already set
    let wanted = "2026-09-09T14:30:00.000Z";
    println!("SENDING   due = {wanted}");

    let created: serde_json::Value = http
        .post(format!("{base}/lists/{list_id}/tasks"))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "title": "[probe] delete me",
            "due": wanted,
        }))
        .send()?
        .json()?;

    let task_id = created["id"].as_str().ok_or("no id returned")?.to_string();
    println!("ON INSERT due = {}", created["due"]);

    // Patch it again, in case insert and patch behave differently
    let patched: serde_json::Value = http
        .patch(format!("{base}/lists/{list_id}/tasks/{task_id}"))
        .bearer_auth(&token)
        .json(&serde_json::json!({ "due": wanted }))
        .send()?
        .json()?;
    println!("ON PATCH  due = {}", patched["due"]);

    // Read it back fresh, in case the write echo differs from stored state
    let fetched: serde_json::Value = http
        .get(format!("{base}/lists/{list_id}/tasks/{task_id}"))
        .bearer_auth(&token)
        .send()?
        .json()?;
    println!("ON GET    due = {}", fetched["due"]);

    println!("\nfull stored resource:");
    println!("{}", serde_json::to_string_pretty(&fetched)?);

    // Clean up so the probe leaves nothing behind
    http.delete(format!("{base}/lists/{list_id}/tasks/{task_id}"))
        .bearer_auth(&token)
        .send()?;
    println!("\n(probe task deleted)");

    Ok(())
}

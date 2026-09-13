//! Google Tasks API client.
//!
//! Two rules apply to every list call and are enforced here rather than left to
//! callers, because forgetting either produces a convincing phantom bug:
//!
//!   1. `showHidden=true` — a task completed in Google's own web or mobile app
//!      is marked hidden and otherwise vanishes from results. During a delta
//!      sync the widget would keep showing it as incomplete forever.
//!   2. Pagination — `maxResults` defaults to 20, so a list of 25 silently
//!      truncates.
//!
//! Both were found in Phase 0; see docs/api-findings.md §6 and §7.

use reqwest::blocking::{Client, RequestBuilder};
use reqwest::StatusCode;

use super::errors::ApiError;
use super::models::{
    ApiTask, ApiTaskList, ApiTaskListsResponse, ApiTasksResponse, Task, TaskList, TaskPatch,
};

const BASE: &str = "https://tasks.googleapis.com/tasks/v1";

/// The API's ceiling. Fewer round-trips for the same data.
const PAGE_SIZE: u32 = 100;

/// Refuse to loop forever if a cursor never terminates. 100 pages is 10,000
/// tasks — far past anything a sticky note should be showing.
const MAX_PAGES: usize = 100;

pub struct TasksClient {
    http: Client,
    access_token: String,
}

impl TasksClient {
    pub fn new(access_token: String) -> Result<Self, ApiError> {
        let http = Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|_| ApiError::Network)?;

        Ok(Self { http, access_token })
    }

    fn get(&self, url: &str) -> RequestBuilder {
        self.http.get(url).bearer_auth(&self.access_token)
    }

    /// Shared response handling: status classification, `Retry-After`, parsing.
    fn send<T: serde::de::DeserializeOwned>(
        &self,
        request: RequestBuilder,
    ) -> Result<T, ApiError> {
        let response = request.send().map_err(|err| {
            // Timeouts and connect failures are "offline", not "broken".
            if err.is_timeout() || err.is_connect() || err.is_request() {
                ApiError::Network
            } else {
                ApiError::Malformed(err.without_url().to_string())
            }
        })?;

        let status = response.status();
        let retry_after = response
            .headers()
            .get("retry-after")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok());

        if status == StatusCode::NO_CONTENT {
            // DELETE returns an empty body; give serde something to chew on.
            return serde_json::from_str("null")
                .map_err(|e| ApiError::Malformed(e.to_string()));
        }

        let body = response
            .text()
            .map_err(|e| ApiError::Malformed(e.without_url().to_string()))?;

        if !status.is_success() {
            let error = ApiError::from_status(status.as_u16(), &body, retry_after);
            log::warn!("tasks api {} -> {error:?}", status.as_u16());
            return Err(error);
        }

        serde_json::from_str(&body).map_err(|e| {
            // Never log the body: task titles and notes are user content.
            log::warn!("could not parse tasks api response: {e}");
            ApiError::Malformed(e.to_string())
        })
    }

    /* -- Reads ------------------------------------------------------------ */

    pub fn list_task_lists(&self) -> Result<Vec<TaskList>, ApiError> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut request = self
                .get(&format!("{BASE}/users/@me/lists"))
                .query(&[("maxResults", PAGE_SIZE.to_string())]);

            if let Some(token) = &page_token {
                request = request.query(&[("pageToken", token)]);
            }

            let page: ApiTaskListsResponse = self.send(request)?;
            out.extend(page.items.into_iter().map(TaskList::from));

            match page.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => return Ok(out),
            }
        }

        log::warn!("task list pagination hit the page ceiling");
        Ok(out)
    }

    /// `updated_min` turns this into a delta query. Pass `None` for a full sync.
    pub fn list_tasks(
        &self,
        task_list_id: &str,
        updated_min: Option<&str>,
    ) -> Result<Vec<Task>, ApiError> {
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;

        for _ in 0..MAX_PAGES {
            let mut request = self
                .get(&format!("{BASE}/lists/{task_list_id}/tasks"))
                .query(&[
                    ("maxResults", PAGE_SIZE.to_string()),
                    // Both non-negotiable — see the module comment.
                    ("showHidden", "true".to_string()),
                    ("showDeleted", "true".to_string()),
                    ("showCompleted", "true".to_string()),
                ]);

            if let Some(since) = updated_min {
                request = request.query(&[("updatedMin", since)]);
            }
            if let Some(token) = &page_token {
                request = request.query(&[("pageToken", token)]);
            }

            let page: ApiTasksResponse = self.send(request)?;
            out.extend(
                page.items
                    .into_iter()
                    .map(|task| Task::from_api(task, task_list_id)),
            );

            match page.next_page_token {
                Some(token) if !token.is_empty() => page_token = Some(token),
                _ => return Ok(out),
            }
        }

        log::warn!("task pagination hit the page ceiling for list {task_list_id}");
        Ok(out)
    }

    /* -- Writes ----------------------------------------------------------- */

    /// `notes` is sent only when there are some. An empty string is a value
    /// like any other to the API, so writing one would give every quick-added
    /// task an empty description rather than no description.
    pub fn insert_task(
        &self,
        task_list_id: &str,
        title: &str,
        notes: Option<&str>,
    ) -> Result<Task, ApiError> {
        let mut body = serde_json::json!({ "title": title });
        if let Some(notes) = notes.filter(|n| !n.is_empty()) {
            body["notes"] = serde_json::Value::String(notes.to_string());
        }

        let request = self
            .http
            .post(format!("{BASE}/lists/{task_list_id}/tasks"))
            .bearer_auth(&self.access_token)
            .json(&body);

        let created: ApiTask = self.send(request)?;
        Ok(Task::from_api(created, task_list_id))
    }

    pub fn patch_task(
        &self,
        task_list_id: &str,
        task_id: &str,
        patch: &TaskPatch,
    ) -> Result<Task, ApiError> {
        let request = self
            .http
            .patch(format!("{BASE}/lists/{task_list_id}/tasks/{task_id}"))
            .bearer_auth(&self.access_token)
            .json(patch);

        let updated: ApiTask = self.send(request)?;
        Ok(Task::from_api(updated, task_list_id))
    }

    pub fn set_completed(
        &self,
        task_list_id: &str,
        task_id: &str,
        completed: bool,
    ) -> Result<Task, ApiError> {
        let patch = TaskPatch {
            status: Some(
                if completed { "completed" } else { "needsAction" }.to_string(),
            ),
            // Google sets `completed` itself; sending our own timestamp would
            // fight the server's clock.
            ..Default::default()
        };
        self.patch_task(task_list_id, task_id, &patch)
    }

    /// Moves a task: to another position, to another list, or both.
    ///
    /// One call covers reorder and cross-list move because Google models them as
    /// one operation. `position` is output-only, so ordering is expressed as
    /// `previous` — the id of the task this one should follow, or `None` to put
    /// it first.
    ///
    /// `destinationTasklist` was verified present on 2026-09-05
    /// (docs/api-findings.md §1). Without it a cross-list move would mean
    /// delete-and-recreate, losing the id, completion history and subtasks.
    ///
    /// Google's docs note that recurrent tasks cannot be moved between lists;
    /// that surfaces as an API error rather than something we can pre-empt,
    /// since recurrence is invisible to us.
    pub fn move_task(
        &self,
        task_list_id: &str,
        task_id: &str,
        previous: Option<&str>,
        destination: Option<&str>,
    ) -> Result<Task, ApiError> {
        let mut request = self
            .http
            .post(format!("{BASE}/lists/{task_list_id}/tasks/{task_id}/move"))
            .bearer_auth(&self.access_token)
            // Everything this call needs is in the query string, but Google
            // rejects a POST with no `Content-Length` — it answers 411 Length
            // Required, which surfaced as "Google sent something unexpected".
            // An empty body makes reqwest send `Content-Length: 0`.
            .body("");

        // Omitted rather than sent empty: an empty `previous` means "first",
        // which is a different instruction from "leave the position alone".
        if let Some(previous) = previous {
            request = request.query(&[("previous", previous)]);
        }
        if let Some(destination) = destination {
            request = request.query(&[("destinationTasklist", destination)]);
        }

        let moved: ApiTask = self.send(request)?;
        Ok(Task::from_api(moved, destination.unwrap_or(task_list_id)))
    }

    /* -- Task lists ------------------------------------------------------- */

    pub fn create_task_list(&self, title: &str) -> Result<TaskList, ApiError> {
        let request = self
            .http
            .post(format!("{BASE}/users/@me/lists"))
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "title": title }));

        let created: ApiTaskList = self.send(request)?;
        Ok(TaskList::from(created))
    }

    pub fn rename_task_list(
        &self,
        task_list_id: &str,
        title: &str,
    ) -> Result<TaskList, ApiError> {
        let request = self
            .http
            .patch(format!("{BASE}/users/@me/lists/{task_list_id}"))
            .bearer_auth(&self.access_token)
            .json(&serde_json::json!({ "title": title }));

        let updated: ApiTaskList = self.send(request)?;
        Ok(TaskList::from(updated))
    }

    /// Deletes a list **and every task in it**. Irreversible, and Google offers
    /// no undo — callers must confirm before reaching this.
    pub fn delete_task_list(&self, task_list_id: &str) -> Result<(), ApiError> {
        let request = self
            .http
            .delete(format!("{BASE}/users/@me/lists/{task_list_id}"))
            .bearer_auth(&self.access_token);

        match self.send::<serde_json::Value>(request) {
            Ok(_) => Ok(()),
            Err(ApiError::NotFound) => Ok(()),
            Err(other) => Err(other),
        }
    }

    /// A task already gone counts as success — the caller wanted it absent.
    pub fn delete_task(&self, task_list_id: &str, task_id: &str) -> Result<(), ApiError> {
        let request = self
            .http
            .delete(format!("{BASE}/lists/{task_list_id}/tasks/{task_id}"))
            .bearer_auth(&self.access_token);

        match self.send::<serde_json::Value>(request) {
            Ok(_) => Ok(()),
            Err(ApiError::NotFound) => Ok(()),
            Err(other) => Err(other),
        }
    }
}

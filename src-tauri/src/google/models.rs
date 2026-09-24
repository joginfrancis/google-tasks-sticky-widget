//! Wire types for the Google Tasks API, and the domain types the rest of the
//! app uses.
//!
//! Kept apart on purpose: the API's shape (string enums, optional everything,
//! `deleted`/`hidden` flags) should not leak into the UI, and the UI's shape
//! should not constrain what we can parse back from Google.

use serde::{Deserialize, Serialize};

/* -- Wire ----------------------------------------------------------------- */

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTaskList {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub updated: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTaskListsResponse {
    #[serde(default)]
    pub items: Vec<ApiTaskList>,
    pub next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTask {
    pub id: String,
    #[serde(default)]
    pub title: String,
    pub notes: Option<String>,
    pub due: Option<String>,
    #[serde(default)]
    pub status: String,
    pub completed: Option<String>,
    pub parent: Option<String>,
    #[serde(default)]
    pub position: String,
    #[serde(default)]
    pub updated: String,
    /// Present only because we pass `showDeleted=true`.
    #[serde(default)]
    pub deleted: bool,
    /// Set by Google when a task is completed in a first-party client. We only
    /// see these because we pass `showHidden=true` — see docs/api-findings.md §6.
    #[serde(default)]
    pub hidden: bool,
    /// Deep link into the Google Tasks web UI. The escape hatch for everything
    /// the API cannot do — starring, recurrence, attachments (docs/ui-parity.md §1).
    pub web_view_link: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiTasksResponse {
    #[serde(default)]
    pub items: Vec<ApiTask>,
    pub next_page_token: Option<String>,
}

/// Only the fields we ever write. `position` is output-only and `parent` is set
/// through `tasks.move`, so neither belongs here.
#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// `Some(None)` clears the description; `None` leaves it untouched.
    ///
    /// Clearing needs an explicit `null`: an empty string is a value Google
    /// stores rather than a removal, which is why emptying a description in
    /// the widget appeared to do nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<Option<String>>,
    /// `Some(None)` clears the date; `None` leaves it untouched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<Option<String>>,
}

/* -- Domain --------------------------------------------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskList {
    pub id: String,
    pub title: String,
    pub updated: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub task_list_id: String,
    pub parent_id: Option<String>,
    pub title: String,
    pub notes: Option<String>,
    /// Date only — the API discards any time (docs/api-findings.md §2).
    pub due: Option<String>,
    pub status: String,
    pub completed_at: Option<String>,
    pub position: String,
    pub updated: String,
    pub deleted: bool,
    pub web_view_link: Option<String>,
}

impl Task {
    pub fn is_completed(&self) -> bool {
        self.status == "completed"
    }

    pub fn from_api(api: ApiTask, task_list_id: &str) -> Self {
        Self {
            id: api.id,
            task_list_id: task_list_id.to_string(),
            parent_id: api.parent,
            title: api.title,
            notes: api.notes,
            due: api.due,
            status: if api.status.is_empty() {
                "needsAction".to_string()
            } else {
                api.status
            },
            completed_at: api.completed,
            position: api.position,
            updated: api.updated,
            // A hidden task is one completed elsewhere — still a real task, not
            // a deletion. Only `deleted` removes it from the cache.
            deleted: api.deleted,
            web_view_link: api.web_view_link,
        }
    }
}

impl From<ApiTaskList> for TaskList {
    fn from(api: ApiTaskList) -> Self {
        Self {
            id: api.id,
            title: api.title,
            updated: api.updated,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api_task() -> ApiTask {
        ApiTask {
            id: "abc".into(),
            title: "Buy filament".into(),
            notes: None,
            due: Some("2026-09-10T00:00:00.000Z".into()),
            status: "needsAction".into(),
            completed: None,
            parent: None,
            position: "0000".into(),
            updated: "2026-09-07T10:00:00.000Z".into(),
            deleted: false,
            hidden: false,
            web_view_link: Some("https://tasks.google.com/task/abc".into()),
        }
    }

    #[test]
    fn clearing_a_description_sends_null_and_leaving_it_sends_nothing() {
        // An empty string is a value Google keeps; only null removes the
        // description. Getting this wrong makes "delete the details" silently
        // do nothing at all.
        let cleared = TaskPatch {
            notes: Some(None),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&cleared).expect("serialises"),
            r#"{"notes":null}"#
        );

        let untouched = TaskPatch {
            title: Some("Renamed".into()),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&untouched).expect("serialises"),
            r#"{"title":"Renamed"}"#
        );

        let written = TaskPatch {
            notes: Some(Some("Details".into())),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&written).expect("serialises"),
            r#"{"notes":"Details"}"#
        );
    }

    #[test]
    fn hidden_tasks_are_kept_not_treated_as_deleted() {
        // The whole point of showHidden=true: a task completed in the Google
        // web UI comes back hidden, and dropping it would strand the local copy
        // as permanently incomplete.
        let mut api = api_task();
        api.hidden = true;
        api.status = "completed".into();

        let task = Task::from_api(api, "list1");
        assert!(!task.deleted, "hidden must not mean deleted");
        assert!(task.is_completed());
    }

    #[test]
    fn deleted_flag_survives_conversion() {
        let mut api = api_task();
        api.deleted = true;
        assert!(Task::from_api(api, "list1").deleted);
    }

    #[test]
    fn missing_status_defaults_to_needs_action() {
        let mut api = api_task();
        api.status = String::new();
        assert_eq!(Task::from_api(api, "list1").status, "needsAction");
    }

    #[test]
    fn patch_omits_untouched_fields() {
        let patch = TaskPatch {
            status: Some("completed".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&patch).unwrap();
        assert_eq!(json, r#"{"status":"completed"}"#);
    }

    #[test]
    fn patch_can_clear_a_due_date() {
        let patch = TaskPatch {
            due: Some(None),
            ..Default::default()
        };
        assert_eq!(serde_json::to_string(&patch).unwrap(), r#"{"due":null}"#);
    }
}

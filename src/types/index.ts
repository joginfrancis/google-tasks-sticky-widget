/**
 * Shapes mirroring the Google Tasks resources we actually use.
 *
 * These will be generated from the Rust structs once the real client exists
 * (ARCHITECTURE §9). Until then they are the contract the mock data honours,
 * so Phase 5 is a swap of the data source rather than a rewrite of the UI.
 */

export type TaskStatus = "needsAction" | "completed";

/** Mirrors `google::models::Task` in Rust, which serializes as camelCase. */
export interface Task {
  id: string;
  taskListId?: string;
  parentId: string | null;
  title: string;
  notes: string | null;
  /**
   * Date only. The API discards any time component (SPEC §4.1), so this is
   * always midnight UTC and must never be rendered with a time.
   */
  due: string | null;
  status: TaskStatus;
  completedAt?: string | null;
  /** Server-assigned ordering key. Output only — see SPEC §4.3. */
  position: string;
  updated: string;
  deleted?: boolean;
  /** Deep link into Google Tasks, for the things the API cannot do. */
  webViewLink?: string | null;
}

export interface TaskList {
  id: string;
  title: string;
  updated?: string;
}

export type SyncState = "synced" | "syncing" | "offline" | "error";

export interface SyncStatus {
  state: SyncState;
  /** ISO timestamp of the last successful sync, null if never. */
  lastSyncedAt: string | null;
  /** Human-readable, already translated out of protocol-speak (SPEC §5). */
  message: string | null;
}

export type ThemePreference = "light" | "dark" | "system";

/** Where the window sits relative to other applications. */
export type WindowLayer = "top" | "normal" | "bottom";

export interface Settings {
  windowLayer: WindowLayer;
  showInTaskbar: boolean;
  startWithWindows: boolean;
  startHidden: boolean;
  globalHotkey: string;
  globalHotkeyEnabled: boolean;
  theme: ThemePreference;
  selectedTaskListId: string | null;
}

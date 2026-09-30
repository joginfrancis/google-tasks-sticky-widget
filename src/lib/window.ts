import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * The window declared in tauri.conf.json.
 *
 * It behaves as the default note: it follows the persisted list selection, and
 * closing it hides to the tray rather than destroying it. Every other note is
 * pinned to one list and really closes.
 */
export const MAIN_LABEL = "main";

let cachedLabel: string | null = null;

/** Cheap and stable — the label never changes for the life of a window. */
export function windowLabel(): string {
  if (cachedLabel === null) {
    try {
      cachedLabel = getCurrentWindow().label;
    } catch {
      // Outside a Tauri context (a plain browser during development); treat it
      // as the main window so the UI still renders.
      cachedLabel = MAIN_LABEL;
    }
  }
  return cachedLabel;
}

export function isMainNote(): boolean {
  return windowLabel() === MAIN_LABEL;
}

/**
 * The task this window is a page for, if it is one.
 *
 * Page windows carry their task in the URL rather than in a registry, because
 * unlike a note there is nothing to restore later: a page is opened, used and
 * closed within one session.
 */
export function pageTarget(): { taskId: string; listId: string } | null {
  if (!windowLabel().startsWith("page-")) return null;
  // The fragment is where the window was opened with it; the query is kept as
  // a fallback so a page opened by an older build still works.
  const params = new URLSearchParams(
    window.location.hash.replace(/^#/, "") || window.location.search,
  );
  const taskId = params.get("page");
  const listId = params.get("list");
  return taskId && listId ? { taskId, listId } : null;
}

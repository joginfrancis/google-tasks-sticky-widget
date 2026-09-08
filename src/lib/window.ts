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

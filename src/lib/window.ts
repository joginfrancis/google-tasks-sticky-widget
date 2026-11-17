import { getCurrentWindow } from "@tauri-apps/api/window";

/**
 * The default note.
 *
 * Nothing is declared in tauri.conf any more, so this window exists only when
 * there is no saved arrangement to restore - a first run, or a session where
 * every note had been closed. It follows the persisted list selection where
 * every other note is pinned to one list.
 */
export const MAIN_LABEL = "main";

const NOTE_LABEL = /^note-(\d+)$/;

/** `main` sorts before every note; notes sort by number, not by string. */
function labelOrder(label: string): number {
  const match = NOTE_LABEL.exec(label);
  return match ? Number(match[1]) : -1;
}

/**
 * The one window that should check for a new version.
 *
 * Five notes are five webviews of the same app, so five simultaneous downloads
 * of one installer would be absurd - but nobody checking is worse, and that is
 * what the rule "only `main` checks" actually meant. A restored session has no
 * `main` window at all, because the arrangement is rebuilt from saved labels
 * and `main` is not one of them. On a desk with five notes open the check
 * therefore never ran, and 0.1.7 sat on GitHub unoffered for a fortnight.
 *
 * `main` still wins when it is there, since that is a fresh install. Otherwise
 * the lowest-numbered note takes the job. Every window runs this over the same
 * list and exactly one of them agrees it was chosen, which is why the answer
 * is computed rather than raced for.
 */
export function pickChecker(labels: string[]): string | null {
  const eligible = labels.filter((l) => l === MAIN_LABEL || NOTE_LABEL.test(l));
  if (eligible.length === 0) {
    return null;
  }
  return eligible.sort((a, b) => labelOrder(a) - labelOrder(b))[0];
}

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

/** Page windows are one task each, opened from a note and closed again. */
export function isPageWindow(): boolean {
  return windowLabel().startsWith("page-");
}

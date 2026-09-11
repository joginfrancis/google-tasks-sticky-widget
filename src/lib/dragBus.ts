/**
 * The channel a drag uses to reach the note it is passing over.
 *
 * A webview can only see its own insides: it does not know that another note
 * exists, where it sits, or what list it shows. So a drag that leaves its own
 * window asks Rust which note is under the cursor, then broadcasts what it is
 * carrying. Every note listens; the one named as the target draws the drop
 * indicator and, on release, performs the move.
 *
 * Nothing is serialised beyond these few primitives — there is no drag payload
 * and no native clipboard format, because the two windows are two views of one
 * process (docs/cross-window-drag-spec.md).
 *
 * Note that the *target* window does the committing, not the source. It is the
 * one that knows its own list and its own row geometry, and it survives the
 * source note being closed mid-drag.
 */

import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";

/** A drag is hovering a note. Sent on every throttled move. */
export const DRAG_OVER = "drag:over";
/** The pointer was released. Only the named target acts on it. */
export const DRAG_DROP = "drag:drop";
/** The drag ended without a drop — clear any indicator. */
export const DRAG_END = "drag:end";

export interface DragOverPayload {
  /**
   * Identifies one drag gesture.
   *
   * A drop must be acted on exactly once. Listeners can be registered more than
   * once for reasons that have nothing to do with this code — React's
   * StrictMode double-mounts every effect in development, which is enough to
   * move the same task twice, the second attempt failing on a task Google has
   * already moved. Carrying the gesture's identity makes a repeat recognisable
   * rather than something to be prevented by careful listener bookkeeping.
   */
  dragId: string;
  /** Window the drag started in, so a note can ignore its own broadcast. */
  originLabel: string;
  /** Note the cursor is over now, or null when it is over none. */
  targetLabel: string | null;
  taskId: string;
  fromListId: string;
  /** Physical screen pixels — the one frame both windows agree on. */
  screenX: number;
  screenY: number;
}

export type DragDropPayload = DragOverPayload;

/**
 * Window position and DPI scale, cached for the life of the drag.
 *
 * Re-fetching per pointer move would put an IPC round-trip on every frame, and
 * a note cannot be moved while a drag is holding the pointer.
 */
export interface WindowFrame {
  x: number;
  y: number;
  scale: number;
}

export async function windowFrame(): Promise<WindowFrame> {
  const [x, y, scale] = await invoke<[number, number, number]>(
    "note_window_frame",
  );
  return { x, y, scale };
}

/** Client (CSS) pixels in this window → physical pixels on the desktop. */
export function toScreen(
  frame: WindowFrame,
  clientX: number,
  clientY: number,
): { screenX: number; screenY: number } {
  return {
    screenX: Math.round(frame.x + clientX * frame.scale),
    screenY: Math.round(frame.y + clientY * frame.scale),
  };
}

/** Physical pixels on the desktop → client (CSS) pixels in this window. */
export function toClient(
  frame: WindowFrame,
  screenX: number,
  screenY: number,
): { x: number; y: number } {
  return {
    x: (screenX - frame.x) / frame.scale,
    y: (screenY - frame.y) / frame.scale,
  };
}

/** Which note window is under this desktop point, if any. */
export function noteWindowAt(
  screenX: number,
  screenY: number,
): Promise<string | null> {
  return invoke<string | null>("note_window_at", { x: screenX, y: screenY });
}

export function emitDragOver(payload: DragOverPayload) {
  void emit(DRAG_OVER, payload);
}

export function emitDragDrop(payload: DragDropPayload) {
  void emit(DRAG_DROP, payload);
}

export function emitDragEnd() {
  void emit(DRAG_END, {});
}

export function onDragOver(fn: (p: DragOverPayload) => void) {
  return listen<DragOverPayload>(DRAG_OVER, (e) => fn(e.payload));
}

export function onDragDrop(fn: (p: DragDropPayload) => void) {
  return listen<DragDropPayload>(DRAG_DROP, (e) => fn(e.payload));
}

export function onDragEnd(fn: () => void) {
  return listen(DRAG_END, () => fn());
}

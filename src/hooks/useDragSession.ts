/**
 * The state of an in-progress task drag.
 *
 * Deliberately shaped as a single record rather than scattered component state,
 * because cross-window drag (docs/cross-window-drag-spec.md) needs exactly this
 * record to live in Rust and be broadcast to every note window. Keeping the
 * shape now means Stage 3 swaps where it is *stored* without touching how it is
 * *read* — every consumer already renders from a session object it does not own.
 *
 * Coordinates are client-space for now. They become physical screen pixels when
 * the session moves to Rust; nothing here reads them, so that change is local to
 * the pointer handlers.
 */

import { useCallback, useEffect, useRef, useState } from "react";

/** Pixels the pointer must travel before a press becomes a drag. */
const DRAG_THRESHOLD_PX = 4;

export interface DragSession {
  taskId: string;
  fromListId: string;
  fromIndex: number;
  cursor: { x: number; y: number };
  /** Cursor→row top-left, so the row does not jump under the pointer on grab. */
  grabOffset: { x: number; y: number };
  /** Null while the pointer is over no valid drop position. */
  overListId: string | null;
  overIndex: number | null;
  /** Enough to draw the dragged row without re-querying the task. */
  ghost: { width: number; height: number; title: string };
}

interface StartArgs {
  taskId: string;
  fromListId: string;
  fromIndex: number;
  title: string;
  /** The row being grabbed, for size and grab offset. */
  row: HTMLElement;
  event: React.PointerEvent;
}

/**
 * Owns the drag lifecycle for one window.
 *
 * `resolveIndex` is supplied by the list, which is the only thing that knows its
 * own row geometry. Returning null means "not a valid drop here" and clears the
 * indicator rather than guessing at an index.
 */
export function useDragSession(options: {
  resolveIndex: (client: { x: number; y: number }) => number | null;
  onCommit: (taskId: string, toIndex: number) => void;
}) {
  const [session, setSession] = useState<DragSession | null>(null);

  // Handlers are installed once per drag and must see current values without
  // being torn down and rebuilt on every pointer move.
  const optionsRef = useRef(options);
  optionsRef.current = options;
  const sessionRef = useRef<DragSession | null>(null);
  sessionRef.current = session;

  // Set between pointerdown and the threshold being crossed. A press that never
  // travels far enough stays a click, so taps and menu opens are unaffected.
  const pendingRef = useRef<
    | null
    | (StartArgs & { origin: { x: number; y: number }; pointerId: number })
  >(null);

  const cancel = useCallback(() => {
    pendingRef.current = null;
    setSession(null);
  }, []);

  const beginPress = useCallback((args: StartArgs) => {
    const { event, row } = args;
    // Left button only: a right-press opens the context menu, and a middle
    // press scrolls.
    if (event.button !== 0) return;

    const rect = row.getBoundingClientRect();
    pendingRef.current = {
      ...args,
      pointerId: event.pointerId,
      origin: { x: event.clientX, y: event.clientY },
    };
    void rect;
  }, []);

  useEffect(() => {
    // One set of window-level listeners covers both the pre-threshold press and
    // the drag itself. They are attached always rather than per-drag: the cost
    // is a no-op branch, and it removes a whole class of missed-teardown bug.
    const onMove = (event: PointerEvent) => {
      const pending = pendingRef.current;

      if (pending && !sessionRef.current) {
        if (event.pointerId !== pending.pointerId) return;
        const dx = event.clientX - pending.origin.x;
        const dy = event.clientY - pending.origin.y;
        if (Math.hypot(dx, dy) < DRAG_THRESHOLD_PX) return;

        const rect = pending.row.getBoundingClientRect();
        setSession({
          taskId: pending.taskId,
          fromListId: pending.fromListId,
          fromIndex: pending.fromIndex,
          cursor: { x: event.clientX, y: event.clientY },
          grabOffset: {
            x: pending.origin.x - rect.left,
            y: pending.origin.y - rect.top,
          },
          overListId: pending.fromListId,
          overIndex: pending.fromIndex,
          ghost: {
            width: rect.width,
            height: rect.height,
            title: pending.title,
          },
        });
        return;
      }

      const current = sessionRef.current;
      if (!current) return;

      const client = { x: event.clientX, y: event.clientY };
      const index = optionsRef.current.resolveIndex(client);
      setSession({
        ...current,
        cursor: client,
        overListId: index === null ? null : current.fromListId,
        overIndex: index,
      });
    };

    const onUp = () => {
      const current = sessionRef.current;
      pendingRef.current = null;
      if (!current) return;

      // A drop on no valid target, or back where it started, is a cancel — not
      // a write. Reordering to the same place would still cost a network call.
      if (
        current.overIndex !== null &&
        current.overListId !== null &&
        current.overIndex !== current.fromIndex
      ) {
        // `resolveIndex` counts slots in the list *as drawn*, which still holds
        // the row being dragged. The move API positions a task among siblings
        // with itself removed, and pulling the row out shifts everything below
        // it up by one — so a downward drop is one slot too high in that frame.
        const toIndex =
          current.overIndex > current.fromIndex
            ? current.overIndex - 1
            : current.overIndex;
        optionsRef.current.onCommit(current.taskId, toIndex);
      }
      setSession(null);
    };

    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") cancel();
    };

    // Losing the window mid-drag must not strand the session. This matters more
    // once the ghost is its own always-on-top window, which would otherwise be
    // left floating over the desktop with nothing to dismiss it.
    const onBlur = () => cancel();

    window.addEventListener("pointermove", onMove);
    window.addEventListener("pointerup", onUp);
    window.addEventListener("pointercancel", onUp);
    window.addEventListener("keydown", onKey);
    window.addEventListener("blur", onBlur);
    return () => {
      window.removeEventListener("pointermove", onMove);
      window.removeEventListener("pointerup", onUp);
      window.removeEventListener("pointercancel", onUp);
      window.removeEventListener("keydown", onKey);
      window.removeEventListener("blur", onBlur);
    };
  }, [cancel]);

  return { session, beginPress, cancel };
}

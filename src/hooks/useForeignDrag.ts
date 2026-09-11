/**
 * A drag that started in another note, seen from this one.
 *
 * The source window holds the pointer for the whole gesture, so this window
 * receives **no mouse events at all** while a task is dragged across it. Every
 * bit of feedback here is therefore driven by the broadcast from the source
 * (docs/cross-window-drag-spec.md) — if you find yourself reaching for a
 * mousemove handler in this file, the design has drifted.
 *
 * This window resolves the drop index itself, because it is the only one that
 * knows its own row geometry, and it performs the move, because it is the only
 * one that knows which list it is showing — and because it outlives the source
 * note being closed mid-drag.
 */

import { useEffect, useRef, useState } from "react";
import {
  onDragDrop,
  onDragEnd,
  onDragOver,
  toClient,
  windowFrame,
  type WindowFrame,
} from "../lib/dragBus";
import { windowLabel } from "../lib/window";

export interface ForeignDrag {
  taskId: string;
  fromListId: string;
  /** Slot this window would drop into, or null when not over its list. */
  overIndex: number | null;
}

export function useForeignDrag(options: {
  /** Client point → insertion index in this window's list, or null. */
  resolveIndex: (client: { x: number; y: number }) => number | null;
  /** Take the task from another list into this one, at an index. */
  onAdopt: (taskId: string, fromListId: string, toIndex: number) => void;
}) {
  const [drag, setDrag] = useState<ForeignDrag | null>(null);

  const optionsRef = useRef(options);
  optionsRef.current = options;
  const dragRef = useRef<ForeignDrag | null>(null);
  dragRef.current = drag;

  // This window's own position and DPI. Refreshed on every drag rather than
  // cached for the session: unlike the source note, this one may well have been
  // moved since the last drag.
  const frameRef = useRef<WindowFrame | null>(null);

  useEffect(() => {
    const self = windowLabel();
    let alive = true;

    const unlisteners = [
      onDragOver((payload) => {
        if (!alive) return;

        // Not for us — either aimed at another note, or our own drag echoing
        // back, which this window already draws from its local session.
        if (payload.targetLabel !== self || payload.originLabel === self) {
          if (dragRef.current) setDrag(null);
          return;
        }

        const apply = (frame: WindowFrame) => {
          if (!alive) return;
          const client = toClient(frame, payload.screenX, payload.screenY);
          setDrag({
            taskId: payload.taskId,
            fromListId: payload.fromListId,
            overIndex: optionsRef.current.resolveIndex(client),
          });
        };

        const frame = frameRef.current;
        if (frame) apply(frame);
        else {
          void windowFrame().then((f) => {
            frameRef.current = f;
            apply(f);
          });
        }
      }),

      onDragDrop((payload) => {
        if (!alive) return;
        if (payload.targetLabel !== self || payload.originLabel === self) return;

        const frame = frameRef.current;
        const current = dragRef.current;
        setDrag(null);

        // Prefer the index from the drop's own coordinates; fall back to the
        // last one drawn, so a drop landing between broadcasts still works.
        const index = frame
          ? optionsRef.current.resolveIndex(
              toClient(frame, payload.screenX, payload.screenY),
            )
          : (current?.overIndex ?? null);

        // Released over this note but not over its list — its header, say.
        // Appending would put the task somewhere the user never pointed at.
        if (index === null) return;

        optionsRef.current.onAdopt(payload.taskId, payload.fromListId, index);
      }),

      onDragEnd(() => {
        if (alive && dragRef.current) setDrag(null);
      }),
    ];

    return () => {
      alive = false;
      unlisteners.forEach((p) => void p.then((fn) => fn()));
    };
  }, []);

  // A note moved between drags would otherwise convert coordinates against a
  // stale origin, putting the indicator in the wrong place.
  useEffect(() => {
    if (drag === null) frameRef.current = null;
  }, [drag]);

  return drag;
}

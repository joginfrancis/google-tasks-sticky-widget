/**
 * The row the user is actually moving.
 *
 * Without this, a drag only moved the insertion line: the list showed where the
 * task *would* land but nothing followed the pointer, so the gesture read as
 * broken. `.task-item.is-dragging` has always styled the source row as a hole
 * to make space for this — the hole existed, the thing that fills it did not.
 *
 * Renders from the session record alone (title and measured size), never from
 * the task. That is deliberate: in Stage 3 of docs/cross-window-drag-spec.md the
 * session moves to Rust and the ghost becomes its own always-on-top window,
 * where the originating list's task objects are not reachable. Keeping it to
 * primitives now means that swap changes where this is mounted, not what it
 * draws.
 */

import type { DragSession } from "../../hooks/useDragSession";

export function DragGhost({ session }: { session: DragSession }) {
  // Cursor minus the grab offset, so the row holds the same spot under the
  // pointer it had when grabbed rather than snapping its corner to the cursor.
  const x = session.cursor.x - session.grabOffset.x;
  const y = session.cursor.y - session.grabOffset.y;

  return (
    <div
      className="drag-ghost"
      aria-hidden="true"
      style={{
        // translate3d rather than left/top: this moves on every pointermove, and
        // a transform is composited instead of triggering layout.
        transform: `translate3d(${x}px, ${y}px, 0)`,
        width: session.ghost.width,
        minHeight: session.ghost.height,
      }}
    >
      <span className="drag-ghost-checkbox" />
      <span className="drag-ghost-title">{session.ghost.title}</span>
    </div>
  );
}

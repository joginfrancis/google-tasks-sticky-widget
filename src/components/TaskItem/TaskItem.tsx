import { useEffect, useRef, useState } from "react";
import type { Task } from "../../types";
import { dueDateInDays, formatDue, isOverdue } from "../../lib/date";
import { DueChip } from "./DueChip";
import "./TaskItem.css";

/**
 * How long a single click waits to see whether a second one is coming.
 *
 * Windows' own double-click threshold defaults to 500ms, but that is far too
 * long to sit on before opening a row — the delay reads as lag. 220ms catches
 * an ordinary double click while still feeling immediate.
 */
const DOUBLE_CLICK_GRACE_MS = 220;

interface Props {
  task: Task;
  isSubtask: boolean;
  /** Set while the completion animation plays, before the row moves sections. */
  isSettling: boolean;
  /** Message from a failed write, shown inline rather than in a dialog. */
  error: string | null;
  onToggle: (id: string) => void;
  onDelete: (id: string) => void;
  onEdit: (id: string, patch: { title?: string; notes?: string }) => void;
  onSetDue: (id: string, due: string | null) => void;
  onOpenInGoogle: (id: string) => void;
  onMoveToList: (id: string, destinationListId: string) => void;
  /** Every list except the one this task is already in. */
  otherLists: { id: string; title: string }[];
  /**
   * Begins a possible drag. Only supplied for rows that may be reordered —
   * top-level, incomplete tasks — so an absent handler is what makes a row
   * undraggable, not a check inside it.
   */
  onDragPress?: (event: React.PointerEvent, row: HTMLElement) => void;
  /** True while this row is the one being dragged; it renders as a gap. */
  isDragging?: boolean;
  /**
   * Expanded rows show notes, date and actions inline.
   *
   * Held by the list rather than the row so only one can be open at a time —
   * several expanded rows on a 340px panel push everything else off-screen.
   */
  isExpanded: boolean;
  onToggleExpand: (id: string) => void;
}

export function TaskItem({
  task,
  isSubtask,
  isSettling,
  error,
  onToggle,
  onDelete,
  onEdit,
  onSetDue,
  onOpenInGoogle,
  onMoveToList,
  otherLists,
  onDragPress,
  isDragging,
  isExpanded,
  onToggleExpand,
}: Props) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const [editing, setEditing] = useState<null | "title" | "notes">(null);
  const [draft, setDraft] = useState("");
  const [dueOpen, setDueOpen] = useState(false);
  const inputRef = useRef<HTMLTextAreaElement>(null);
  const rowRef = useRef<HTMLLIElement>(null);
  /**
   * Where the press started, so a drag that ends on the title does not also
   * register as a click and toggle the row open.
   */
  const pressOrigin = useRef<{ x: number; y: number } | null>(null);
  const expandTimer = useRef<number | null>(null);
  /** Tab's next stop after the description. */
  const dueButtonRef = useRef<HTMLButtonElement>(null);

  const done = task.status === "completed";
  const overdue = !done && isOverdue(task.due);
  /**
   * Written locally but not yet confirmed by Google. The id is the tell: a real
   * one comes from the API, so a `pending-` prefix can only be ours.
   */
  const pending = task.id.startsWith("pending-");
  /**
   * Whether a date chip is already rendered, and so already owns the calendar.
   *
   * One for a task that has a date, another offering "Add date" on an open row;
   * both draw the calendar when `dueOpen` is set, so anything else that might
   * draw one has to check first.
   */
  const hasVisibleDueChip = Boolean(task.due) || (isExpanded && !done);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

  // A row unmounted mid-gesture — completed, deleted, or dragged to another
  // list — must not expand itself a moment later.
  useEffect(
    () => () => {
      if (expandTimer.current !== null) window.clearTimeout(expandTimer.current);
    },
    [],
  );

  const beginEdit = (field: "title" | "notes") => {
    if (done) return; // Editing a finished task is almost always a misclick.
    setDraft(field === "title" ? task.title : (task.notes ?? ""));
    setEditing(field);
  };

  const commitEdit = () => {
    if (!editing) return;
    const value = draft.trim();
    const current = editing === "title" ? task.title : (task.notes ?? "");

    // An empty title would be rejected by the API anyway; treat it as a cancel
    // rather than surfacing an error the user did not mean to trigger.
    const unchanged = value === current;
    const emptyTitle = editing === "title" && value === "";

    if (!unchanged && !emptyTitle) {
      onEdit(task.id, editing === "title" ? { title: value } : { notes: value });
    }
    setEditing(null);
  };

  /**
   * A single click anywhere on the row toggles it open; a double click on text
   * edits that text, whether the row is open or closed.
   *
   * The expand is held back a beat so the first click of a double click does
   * not open the row on its way to edit mode — without that, every rename
   * flashes the row open and shut on the way in.
   */
  const cancelPendingExpand = () => {
    if (expandTimer.current !== null) {
      window.clearTimeout(expandTimer.current);
      expandTimer.current = null;
    }
  };

  const handleRowClick = (event: React.MouseEvent) => {
    if (editing) return;

    // Buttons, the date chip and the menu own their own clicks. Toggling the
    // row as well would make pressing one feel like it did two things.
    if (
      (event.target as HTMLElement).closest(
        "button, input, textarea, select, a, [role='button'], .due-chip, .task-menu, .task-notes",
      )
    ) {
      return;
    }

    // A drag that happens to finish over the row still fires a click. Ignore
    // it, or reordering a task would toggle it open as well.
    const origin = pressOrigin.current;
    if (origin) {
      const moved =
        Math.abs(event.clientX - origin.x) + Math.abs(event.clientY - origin.y);
      if (moved > 4) return;
    }

    // The second click of a double click; the dblclick handler takes it.
    if (event.detail > 1) {
      cancelPendingExpand();
      return;
    }

    cancelPendingExpand();
    expandTimer.current = window.setTimeout(() => {
      expandTimer.current = null;
      onToggleExpand(task.id);
    }, DOUBLE_CLICK_GRACE_MS);
  };

  const handleTextDoubleClick = (field: "title" | "notes") => {
    cancelPendingExpand();
    beginEdit(field);
  };

  const handleEditKey = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      setEditing(null);
      return;
    }

    // Tab walks the row: title → description → date, then out. Each step
    // commits what is open, so moving on never silently drops an edit.
    if (event.key === "Tab" && !event.shiftKey) {
      event.preventDefault();
      commitEdit();
      if (editing === "title") beginEdit("notes");
      else setTimeout(() => dueButtonRef.current?.focus(), 0);
      return;
    }
    if (event.key === "Tab" && event.shiftKey && editing === "notes") {
      event.preventDefault();
      commitEdit();
      beginEdit("title");
      return;
    }

    if (event.key === "Enter") {
      // In a description Enter is a line break, because a description is prose
      // and breaking it is the common intent. Committing needs Ctrl+Enter —
      // and blurring or Escape still work, as everywhere else.
      if (editing === "notes") {
        if (event.ctrlKey || event.metaKey) {
          event.preventDefault();
          commitEdit();
        }
        return;
      }
      // A title is a label, so Enter finishes it. Shift+Enter is left alone
      // rather than inserting a break nothing downstream would render.
      if (!event.shiftKey) {
        event.preventDefault();
        commitEdit();
      }
    }
  };

  return (
    <li
      ref={rowRef}
      className={[
        "task-item",
        pending ? "is-pending" : "",
        isDragging ? "is-dragging" : "",
        isSubtask ? "is-subtask" : "",
        done ? "is-done" : "",
        isSettling ? "is-settling" : "",
        isExpanded ? "is-expanded" : "",
        error ? "has-error" : "",
        menuOpen ? "is-menu-open" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      onContextMenu={(event) => {
        event.preventDefault();
        setMenuOpen(true);
      }}
      onClick={handleRowClick}
      onPointerDown={(event) => {
        pressOrigin.current = { x: event.clientX, y: event.clientY };
        if (!onDragPress || editing) return;
        // Anything the user can operate keeps its own press. Dragging from a
        // checkbox or a menu button would make those unusable, and a press
        // inside text is a caret placement.
        if (
          (event.target as HTMLElement).closest(
            "button, input, textarea, select, a, [contenteditable='true']",
          )
        ) {
          return;
        }
        if (rowRef.current) onDragPress(event, rowRef.current);
      }}
    >
      <div className="task-row">
        <button
          className="checkbox"
          role="checkbox"
          aria-checked={done}
          aria-label={done ? `Mark "${task.title}" not done` : `Complete "${task.title}"`}
          onClick={() => onToggle(task.id)}
        >
          {done && (
            <svg viewBox="0 0 16 16" aria-hidden="true">
              <path
                d="M3.5 8.5l3 3 6-6.5"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                strokeLinejoin="round"
              />
            </svg>
          )}
        </button>

        <div className="task-main">
          {editing === "title" ? (
            <textarea
              ref={inputRef}
              className="task-edit"
              rows={1}
              value={draft}
              maxLength={1024}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={handleEditKey}
              onBlur={commitEdit}
            />
          ) : (
            <div className="task-line">
              <span
                className="task-title"
                onDoubleClick={() => handleTextDoubleClick("title")}
                // The full title, not a hint about editing. A one-line row
                // ellipsises at roughly 24 characters once a date chip is
                // present, and hover was the only way left to read the rest —
                // spending it on a tip the user needs once left the most common
                // row state unreadable.
                title={task.title}
              >
                {task.title}
              </span>

              {/* Says there is something behind this row worth opening it
                  for. Notes are hidden when collapsed so every row stays one
                  line tall, which otherwise leaves a task with a description
                  looking exactly like one without — and nothing to suggest
                  opening it. Only when collapsed: once open, the description
                  is right there and a mark pointing at it is noise. */}
              {task.notes && !isExpanded && (
                <span className="task-has-notes" title="Has details">
                  <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true">
                    <path
                      d="M3.5 5h9M3.5 8h9M3.5 11h5.5"
                      stroke="currentColor"
                      strokeWidth="1.5"
                      strokeLinecap="round"
                    />
                  </svg>
                </span>
              )}

              {/* The date sits on the title line rather than under it, so a
                  collapsed row is one line tall whatever it carries. */}
              {task.due && (
                <DueChip
                  triggerRef={dueButtonRef}
                  label={formatDue(task.due)}
                  overdue={overdue}
                  done={done}
                  open={dueOpen}
                  value={task.due}
                  onOpen={() => !done && setDueOpen(true)}
                  onClose={() => setDueOpen(false)}
                  onChange={(next) => {
                    setDueOpen(false);
                    onSetDue(task.id, next);
                  }}
                />
              )}
            </div>
          )}

          {editing === "notes" ? (
            <textarea
              ref={inputRef}
              className="task-edit is-notes"
              rows={2}
              value={draft}
              maxLength={8192}
              placeholder="Add details…"
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={handleEditKey}
              onBlur={commitEdit}
            />
          ) : isExpanded ? (
            // Expanded always offers the notes slot, empty or not — "add
            // details" being invisible until notes exist is the main thing the
            // collapsed row cannot express.
            /* One click, not two. The description is only on screen while
               the row is open, so a click here cannot be confused with the
               click that closes the row — the ambiguity that makes the title
               need a double click does not exist down here. */
            <span
              className={`task-notes ${task.notes ? "" : "is-placeholder"}`}
              onClick={() => beginEdit("notes")}
              title="Click to edit"
            >
              {task.notes || "Add details…"}
            </span>
          ) : null}
          {/* Notes are deliberately absent when collapsed: a row carrying
              details would otherwise be taller than one without, and the list
              stops scanning as a single column of tasks. */}

          {/* Only offered when open — a collapsed row with no date stays clean,
              and the chip above covers the case where one is already set. */}
          {isExpanded && !done && (
            <div className="task-date-row">
              {/* The calendar is for a date you have to look up. Today and
                  tomorrow are most of what a sticky note ever needs, and
                  putting them here spends one click on what used to cost
                  three — open the calendar, find the row, click the day. */}
              {!task.due && (
                <DueChip
                  triggerRef={dueButtonRef}
                  label="Add date"
                  overdue={false}
                  done={false}
                  open={dueOpen}
                  value={null}
                  onOpen={() => setDueOpen(true)}
                  onClose={() => setDueOpen(false)}
                  onChange={(next) => {
                    setDueOpen(false);
                    onSetDue(task.id, next);
                  }}
                />
              )}

              <button
                className="task-quick-date"
                onClick={() => onSetDue(task.id, dueDateInDays(0))}
              >
                Today
              </button>
              <button
                className="task-quick-date"
                onClick={() => onSetDue(task.id, dueDateInDays(1))}
              >
                Tomorrow
              </button>

              {/* Pushed to the right end of the same line: repeat belongs with
                  the date it repeats on, but it is a rarer choice than either
                  quick pick and should not sit between them. */}
              <button
                className="task-repeat"
                onClick={() => onOpenInGoogle(task.id)}
                aria-label="Repeat"
                title="Repeating tasks are set in Google Tasks"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    d="M3 8a5 5 0 0 1 8.5-3.5M13 8a5 5 0 0 1-8.5 3.5"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                  />
                  <path
                    d="M11.5 2v2.6H9M4.5 14v-2.6H7"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.5"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </button>
            </div>
          )}

          {isExpanded && (
            <div className="task-actions">
              <button
                className="task-action"
                onClick={() => onOpenInGoogle(task.id)}
                title="Starring, repeats and attachments live in Google Tasks"
              >
                Open in Google
              </button>
              <button
                className="task-action is-danger"
                onClick={() => onDelete(task.id)}
              >
                Delete
              </button>
            </div>
          )}
        </div>

        <button
          className="task-menu-trigger"
          aria-label="Task options"
          title="Task options"
          onClick={() => setMenuOpen(true)}
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <circle cx="8" cy="3" r="1.4" fill="currentColor" />
            <circle cx="8" cy="8" r="1.4" fill="currentColor" />
            <circle cx="8" cy="13" r="1.4" fill="currentColor" />
          </svg>
        </button>
      </div>

      {error && <p className="task-error">{error}</p>}

      {menuOpen && (
        <>
          <div
            className="menu-scrim"
            onClick={() => {
              setMenuOpen(false);
              setMoveOpen(false);
            }}
          />
          <div className="task-menu" role="menu">
            <button
              role="menuitem"
              className="task-menu-item"
              onClick={() => {
                setMenuOpen(false);
                beginEdit("notes");
              }}
            >
              {task.notes ? "Edit details" : "Add details"}
            </button>
            <button
              role="menuitem"
              className="task-menu-item"
              onClick={() => {
                setMenuOpen(false);
                setDueOpen(true);
              }}
            >
              {task.due ? "Change date" : "Add date"}
            </button>
            {task.due && (
              <button
                role="menuitem"
                className="task-menu-item"
                onClick={() => {
                  setMenuOpen(false);
                  onSetDue(task.id, null);
                }}
              >
                Remove date
              </button>
            )}

            {otherLists.length > 0 && (
              <>
                <div className="task-menu-sep" />
                <button
                  role="menuitem"
                  className="task-menu-item has-submenu"
                  aria-expanded={moveOpen}
                  onClick={() => setMoveOpen((v) => !v)}
                >
                  Move to list
                  <span className="submenu-chevron">{moveOpen ? "▴" : "▾"}</span>
                </button>

                {moveOpen && (
                  <div className="task-submenu">
                    {otherLists.map((list) => (
                      <button
                        key={list.id}
                        role="menuitem"
                        className="task-menu-item is-sub"
                        onClick={() => {
                          setMenuOpen(false);
                          setMoveOpen(false);
                          onMoveToList(task.id, list.id);
                        }}
                      >
                        {list.title}
                      </button>
                    ))}
                  </div>
                )}
              </>
            )}

            <div className="task-menu-sep" />

            {/* Starring, recurrence and attachments have no API — this is the
                honest route to them rather than pretending they don't exist. */}
            <button
              role="menuitem"
              className="task-menu-item"
              onClick={() => {
                setMenuOpen(false);
                onOpenInGoogle(task.id);
              }}
            >
              Open in Google Tasks
            </button>

            <div className="task-menu-sep" />

            <button
              role="menuitem"
              className="task-menu-item is-danger"
              onClick={() => {
                setMenuOpen(false);
                onDelete(task.id);
              }}
            >
              Delete
            </button>
          </div>
        </>
      )}

      {/* The menu's fallback calendar, for when there is no chip on screen to
          hang one off — a collapsed row with no date set.
          
          Gated on there being no visible chip, not just on having no date: an
          open row already shows "Add date", and that chip renders its own
          calendar from the same `dueOpen` flag. Without this the two appeared
          together, one behind the other. */}
      {dueOpen && !hasVisibleDueChip && (
        <DueChip
          label=""
          overdue={false}
          done={done}
          open
          value={null}
          onOpen={() => setDueOpen(true)}
          onClose={() => setDueOpen(false)}
          onChange={(next) => {
            setDueOpen(false);
            onSetDue(task.id, next);
          }}
          hideTrigger
        />
      )}
    </li>
  );
}

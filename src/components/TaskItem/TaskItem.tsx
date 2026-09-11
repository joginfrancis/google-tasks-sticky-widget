import { useEffect, useRef, useState } from "react";
import type { Task } from "../../types";
import { formatDue, isOverdue } from "../../lib/date";
import { DueChip } from "./DueChip";
import "./TaskItem.css";

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

  const done = task.status === "completed";
  const overdue = !done && isOverdue(task.due);

  useEffect(() => {
    if (editing && inputRef.current) {
      inputRef.current.focus();
      inputRef.current.select();
    }
  }, [editing]);

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
   * Collapsed, a click opens the row; expanded, it edits the title.
   *
   * The same target meaning two things is deliberate — it matches Google Tasks,
   * and it keeps the most common action (open and look) a single click while
   * still allowing a rename without a menu.
   */
  const handleTitleClick = (event: React.MouseEvent) => {
    // A drag that happens to finish over the title still fires a click. Ignore
    // it, or reordering a task would also toggle it open.
    const origin = pressOrigin.current;
    if (origin) {
      const moved =
        Math.abs(event.clientX - origin.x) + Math.abs(event.clientY - origin.y);
      if (moved > 4) return;
    }

    if (isExpanded) beginEdit("title");
    else onToggleExpand(task.id);
  };

  const handleEditKey = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.key === "Escape") {
      event.preventDefault();
      setEditing(null);
      return;
    }
    // Enter commits; Shift+Enter adds a line, which only makes sense in notes.
    if (event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      commitEdit();
    }
  };

  return (
    <li
      ref={rowRef}
      className={[
        "task-item",
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
            <span
              className="task-title"
              onClick={handleTitleClick}
              title={
                done ? undefined : isExpanded ? "Click to edit" : "Click for details"
              }
            >
              {task.title}
            </span>
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
            <span
              className={`task-notes ${task.notes ? "" : "is-placeholder"}`}
              onClick={() => beginEdit("notes")}
            >
              {task.notes || "Add details…"}
            </span>
          ) : (
            task.notes && (
              <span className="task-notes is-clamped">{task.notes}</span>
            )
          )}

          {task.due ? (
            <DueChip
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
          ) : (
            isExpanded &&
            !done && (
              <DueChip
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
            )
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

      {/* Opened from the menu when the task has no date to click yet. */}
      {dueOpen && !task.due && (
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

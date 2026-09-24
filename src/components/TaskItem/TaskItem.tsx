import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Task } from "../../types";
import { dueDateInDays, formatDue, isOverdue } from "../../lib/date";
import { DueChip } from "./DueChip";
import { RichText } from "./RichText";
import { FormatBar } from "./FormatBar";
import { NotesEditor } from "./NotesEditor";
import { toggleMarker } from "../../lib/richText";
import { growFull, growToFit } from "../../lib/windowFit";
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
  /** Part of a multi-selection. */
  isSelected?: boolean;
  /**
   * A Ctrl- or Shift-click. Supplied only where selection is offered, so its
   * absence leaves those clicks behaving as plain ones.
   */
  onSelect?: (id: string, mode: { toggle: boolean; range: boolean }) => void;
  /**
   * Adds a subtask under this task. Absent for subtasks themselves — Google
   * Tasks is one level deep — so its absence is what hides the option.
   */
  onAddSubtask?: (parentId: string, title: string) => Promise<string | null>;
  /**
   * Opens the "add a task below this one" field. The list owns which row has
   * it, so after each add the field can hop down onto the task just created
   * and several can be typed in order.
   */
  onStartAddBelow?: (id: string) => void;
  /** This row is the one showing the add-below field. */
  isAddingBelow?: boolean;
  /** The task being added below is a subtask, so the field indents to match. */
  belowIsSubtask?: boolean;
  /** Adds below this row; resolves to an error message, or null. */
  onSubmitBelow?: (title: string) => Promise<string | null>;
  onCancelBelow?: () => void;
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
  isSelected,
  onSelect,
  onAddSubtask,
  onStartAddBelow,
  isAddingBelow,
  belowIsSubtask,
  onSubmitBelow,
  onCancelBelow,
}: Props) {
  const [menuOpen, setMenuOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const [editing, setEditing] = useState<null | "title" | "notes">(null);
  const [draft, setDraft] = useState("");
  const [dueOpen, setDueOpen] = useState(false);
  /** The inline "add subtask" field is open under this task. */
  const [addingSubtask, setAddingSubtask] = useState(false);
  const [subtaskDraft, setSubtaskDraft] = useState("");
  const [subtaskError, setSubtaskError] = useState<string | null>(null);
  /**
   * Where to put the caret when the description opens for editing — the
   * character that was clicked. Null means "select everything", which is what
   * a title wants and a paragraph does not.
   */
  const caretTarget = useRef<number | null>(null);
  /** The description textarea, once mounted, so the format bar can act on it. */
  const notesRef = useRef<HTMLDivElement>(null);
  /** Bumped as the description is typed, so the note re-measures its height. */
  const [notesRevision, setNotesRevision] = useState(0);
  const [belowDraft, setBelowDraft] = useState("");
  const [belowError, setBelowError] = useState<string | null>(null);
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
  const canAddSubtask = Boolean(onAddSubtask) && !isSubtask && !done;
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
    if (!editing || !inputRef.current) return;
    const field = inputRef.current;
    field.focus();
    const caret = caretTarget.current;
    caretTarget.current = null;
    if (caret === null) {
      // A title is short and usually being replaced outright.
      field.select();
    } else {
      // A description is prose: clicking it means "put the cursor here", not
      // "highlight all of this". Ctrl+A is still there for the whole thing.
      const at = Math.min(caret, field.value.length);
      field.setSelectionRange(at, at);
    }
  }, [editing]);

  /**
   * A note stretches to the full height of the screen when the task it has
   * open does not fit inside it — the description, the date row and the
   * actions together — and shrinks back when the task is closed again.
   *
   * Measured rather than guessed: whether a given task overflows depends on
   * how tall the user has made this note, so a character count would be right
   * only by accident.
   */
  const releaseTall = useRef<null | (() => void)>(null);
  const setTall = (tall: boolean) => {
    if (tall === Boolean(releaseTall.current)) return;
    if (tall) {
      releaseTall.current = growFull();
    } else {
      releaseTall.current?.();
      releaseTall.current = null;
    }
  };

  useEffect(() => {
    if (!isExpanded) {
      setTall(false);
      return;
    }
    // After the row has been laid out with whatever just changed — an editor
    // opening, a line being typed — not before.
    const id = window.setTimeout(() => {
      const row = rowRef.current;
      const body = row?.closest(".widget-body") as HTMLElement | null;
      if (!row || !body) return;
      if (row.scrollHeight + 16 <= body.clientHeight) return;
      setTall(true);
      window.setTimeout(
        () => rowRef.current?.scrollIntoView({ block: "start", behavior: "auto" }),
        60,
      );
    }, 0);
    return () => window.clearTimeout(id);
  }, [isExpanded, editing, draft, notesRevision, task.notes]);

  // Leaving the row tall after it has gone would strand the note at full
  // height with nothing in it.
  useEffect(() => () => setTall(false), []);

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

    // Ctrl or Shift makes the click a selection instead of an open, the same
    // as in a file manager. Checked after the controls above, so Ctrl+clicking
    // the checkbox still completes the task rather than selecting it.
    if (onSelect && (event.ctrlKey || event.metaKey || event.shiftKey)) {
      cancelPendingExpand();
      onSelect(task.id, {
        toggle: event.ctrlKey || event.metaKey,
        range: event.shiftKey,
      });
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

  /** Wraps the selection in `**` or `*`, and takes it off again. */
  const applyMarker = (marker: string) => {
    const field = inputRef.current;
    if (!field) return;
    const next = toggleMarker(field.value, field.selectionStart, field.selectionEnd, marker);
    setDraft(next.text);
    // After React has rewritten the value, or the old selection is restored.
    window.setTimeout(() => field.setSelectionRange(next.start, next.end), 0);
  };

  const handleEditKey = (event: React.KeyboardEvent<HTMLTextAreaElement>) => {
    // Bold and italic are Markdown characters in the text, because that is all
    // a plain-text description can hold — Google shows the asterisks, this
    // widget shows the emphasis.
    if ((event.ctrlKey || event.metaKey) && !event.altKey) {
      const key = event.key.toLowerCase();
      if (key === "b" || key === "i") {
        event.preventDefault();
        applyMarker(key === "b" ? "**" : "*");
        return;
      }
    }

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

  /**
   * Enter adds the subtask and leaves the field open for the next, exactly as
   * the main add box behaves — a checklist under a task is usually several
   * steps typed in one go. Empty Enter does nothing; Escape closes.
   */
  const submitSubtask = async () => {
    const title = subtaskDraft.trim();
    if (!title || !onAddSubtask) return;
    setSubtaskDraft("");
    setSubtaskError(null);
    const failure = await onAddSubtask(task.id, title);
    if (failure) {
      // SPEC §3.3: a failed add never throws away what was typed.
      setSubtaskDraft(title);
      setSubtaskError(failure);
    }
  };

  const submitBelow = async () => {
    const title = belowDraft.trim();
    if (!title || !onSubmitBelow) return;
    setBelowDraft("");
    setBelowError(null);
    const failure = await onSubmitBelow(title);
    if (failure) {
      setBelowDraft(title);
      setBelowError(failure);
    }
  };

  /**
   * The options menu is positioned by hand, in window coordinates.
   *
   * Laid out inside the row it was clipped twice over: by the scrolling task
   * list, and by the window itself — which is how Delete disappeared off the
   * bottom of a short note. A fixed position escapes the scroller; growing the
   * window covers the rest.
   */
  const menuRef = useRef<HTMLDivElement>(null);
  const triggerRef = useRef<HTMLButtonElement>(null);
  const [menuPos, setMenuPos] = useState<{ top: number; left: number }>({
    top: 0,
    left: 0,
  });

  useLayoutEffect(() => {
    if (!menuOpen) return;

    let release: null | (() => void) = null;

    const place = () => {
      const menu = menuRef.current;
      const anchor = (triggerRef.current ?? rowRef.current)?.getBoundingClientRect();
      if (!menu || !anchor) return;

      const height = menu.offsetHeight;
      const width = menu.offsetWidth;
      const gap = 4;
      const edge = 6;

      // Below the button, above it when that would not fit, and clamped when
      // neither does — then the window is asked for the missing height.
      const below = anchor.bottom + gap;
      const above = anchor.top - height - gap;
      let top = below;
      if (below + height > window.innerHeight - edge) {
        top = above >= edge ? above : Math.max(edge, window.innerHeight - height - edge);
      }

      const left = Math.min(
        Math.max(edge, anchor.right - width),
        window.innerWidth - width - edge,
      );

      setMenuPos({ top, left });

      release?.();
      release = growToFit(menu);
    };

    place();
    window.addEventListener("resize", place);
    window.addEventListener("scroll", place, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", place, true);
      release?.();
    };
    // `moveOpen` is here because expanding the submenu makes the menu taller.
  }, [menuOpen, moveOpen]);

  const openSubtaskField = () => {
    if (!isExpanded) onToggleExpand(task.id);
    setAddingSubtask(true);
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
        isSelected ? "is-selected" : "",
        error ? "has-error" : "",
        menuOpen ? "is-menu-open" : "",
      ]
        .filter(Boolean)
        .join(" ")}
      onContextMenu={(event) => {
        // Inside text being edited, or over text the user has selected, the
        // right-click is about that text: leave the system's Cut / Copy /
        // Paste menu alone rather than covering it with the task menu.
        const target = event.target as HTMLElement;
        const selected = window.getSelection()?.toString() ?? "";
        if (target.closest("textarea, input") || selected.trim()) return;
        event.preventDefault();
        setMenuOpen(true);
      }}
      onClick={handleRowClick}
      onMouseDown={(event) => {
        // Shift+mousedown extends the browser's text selection across every
        // row between the last click and this one, which is not what a
        // Shift+click on a task means here.
        if (onSelect && event.shiftKey) event.preventDefault();
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
            <>
            {/* Above the box, not below: the buttons act on what is selected,
                and a control that sits under its text reads as belonging to
                whatever comes next. */}
            <FormatBar
              editor={notesRef.current}
              onChanged={() => setNotesRevision((n) => n + 1)}
            />
            <NotesEditor
              editorRef={notesRef}
              value={task.notes ?? ""}
              caret={caretTarget.current}
              onInput={() => setNotesRevision((n) => n + 1)}
              onCommit={(markdown) => {
                if (markdown !== (task.notes ?? "")) {
                  onEdit(task.id, { notes: markdown });
                }
                setEditing(null);
              }}
              onTabOut={() => setTimeout(() => dueButtonRef.current?.focus(), 0)}
              onTabBack={() => beginEdit("title")}
            />
            </>
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
              onClick={(event) => {
                // A link and a picture own their clicks; opening the editor
                // on top of one would make them unusable.
                if ((event.target as HTMLElement).closest(".rt-link, .rt-image")) {
                  return;
                }
                caretTarget.current = offsetAtPoint(event);
                beginEdit("notes");
              }}
              title="Click to edit"
            >
              {task.notes ? <RichText text={task.notes} /> : "Add details…"}
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
            /* Icons, not words. Three labelled buttons made a row of the note
               look like a dialog; these say the same thing in a line each and
               leave the description the width it wants. Every one keeps its
               name on hover and for a screen reader. */
            <div className="task-actions">
              {canAddSubtask && !addingSubtask && (
                <button
                  className="task-action"
                  onClick={openSubtaskField}
                  aria-label="Add subtask"
                  title="Add subtask"
                >
                  <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                    <path
                      d="M3 3.5v6a2 2 0 0 0 2 2h4"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.4"
                      strokeLinecap="round"
                    />
                    <path
                      d="M11.5 8.5v6M8.5 11.5h6"
                      fill="none"
                      stroke="currentColor"
                      strokeWidth="1.4"
                      strokeLinecap="round"
                    />
                  </svg>
                </button>
              )}
              <button
                className="task-action"
                onClick={() => onOpenInGoogle(task.id)}
                aria-label="Open in Google Tasks"
                title="Open in Google Tasks — starring, repeats and attachments live there"
              >
                <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                  <path
                    d="M8.5 3H3.5v9.5H13V7.5"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.4"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                  <path
                    d="M10 3h3v3M13 3L8 8"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.4"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </button>
              <button
                className="task-action is-danger"
                onClick={() => onDelete(task.id)}
                aria-label="Delete task"
                title="Delete"
              >
                <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
                  <path
                    d="M3.5 4.5h9M6.5 4.5V3h3v1.5M5 4.5l.6 8h4.8l.6-8"
                    fill="none"
                    stroke="currentColor"
                    strokeWidth="1.4"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                  />
                </svg>
              </button>
            </div>
          )}

          {isExpanded && addingSubtask && canAddSubtask && (
            <div className="subtask-add">
              <input
                className="subtask-input"
                autoFocus
                value={subtaskDraft}
                placeholder="Add a subtask…"
                maxLength={1024}
                onChange={(e) => {
                  setSubtaskDraft(e.target.value);
                  if (subtaskError) setSubtaskError(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void submitSubtask();
                  } else if (e.key === "Escape") {
                    e.preventDefault();
                    e.stopPropagation();
                    setAddingSubtask(false);
                    setSubtaskDraft("");
                    setSubtaskError(null);
                  }
                }}
                onBlur={() => {
                  if (!subtaskDraft.trim() && !subtaskError) setAddingSubtask(false);
                }}
              />
              {subtaskError && <p className="task-error">{subtaskError}</p>}
            </div>
          )}
        </div>

        {onStartAddBelow && !done && (
          <button
            className="task-add-below"
            aria-label="Add a task below"
            title="Add a task below"
            onClick={() => onStartAddBelow(task.id)}
          >
            <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
              <path
                d="M8 3.5v9M3.5 8h9"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            </svg>
          </button>
        )}

        <button
          ref={triggerRef}
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

      {isAddingBelow && (
        <div className={`add-below${belowIsSubtask ? " is-subtask" : ""}`}>
          <span className="add-below-mark" aria-hidden="true" />
          <input
            className="subtask-input"
            autoFocus
            value={belowDraft}
            placeholder="New task…"
            maxLength={1024}
            onChange={(e) => {
              setBelowDraft(e.target.value);
              if (belowError) setBelowError(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                void submitBelow();
              } else if (e.key === "Escape") {
                e.preventDefault();
                e.stopPropagation();
                onCancelBelow?.();
              }
            }}
            onBlur={() => {
              if (!belowDraft.trim() && !belowError) onCancelBelow?.();
            }}
          />
          {belowError && <p className="task-error">{belowError}</p>}
        </div>
      )}

      {menuOpen && (
        <>
          <div
            className="menu-scrim"
            onClick={() => {
              setMenuOpen(false);
              setMoveOpen(false);
            }}
          />
          <div
            className="task-menu"
            role="menu"
            ref={menuRef}
            style={{ top: menuPos.top, left: menuPos.left }}
          >
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
            {onStartAddBelow && !done && (
              <button
                role="menuitem"
                className="task-menu-item"
                onClick={() => {
                  setMenuOpen(false);
                  onStartAddBelow(task.id);
                }}
              >
                Add task below
              </button>
            )}
            {canAddSubtask && (
              <button
                role="menuitem"
                className="task-menu-item"
                onClick={() => {
                  setMenuOpen(false);
                  openSubtaskField();
                }}
              >
                Add subtask
              </button>
            )}
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

/**
 * The character offset a click landed on, so editing can start with the caret
 * there. Returns null where the browser cannot say — the caret then falls back
 * to selecting the whole thing, which is the old behaviour.
 */
function offsetAtPoint(event: React.MouseEvent): number | null {
  const host = event.currentTarget as HTMLElement;
  const doc = document as Document & {
    caretPositionFromPoint?: (x: number, y: number) => { offsetNode: Node; offset: number } | null;
    caretRangeFromPoint?: (x: number, y: number) => Range | null;
  };

  const position = doc.caretPositionFromPoint?.(event.clientX, event.clientY);
  if (position && host.contains(position.offsetNode)) return position.offset;

  const range = doc.caretRangeFromPoint?.(event.clientX, event.clientY);
  if (range && host.contains(range.startContainer)) return range.startOffset;

  return null;
}

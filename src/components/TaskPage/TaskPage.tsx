import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Task } from "../../types";
import { formatDue, dueDateInDays, isOverdue } from "../../lib/date";
import { NotesEditor, NOTES_LIMIT } from "../TaskItem/NotesEditor";
import { FormatBar } from "../TaskItem/FormatBar";
import { DueChip } from "../TaskItem/DueChip";
import "./TaskPage.css";

/** Where the count appears — well before Google's ceiling, not at it. */
const WARN_AT = 7000;
/** How long to wait after typing stops before saving. */
const AUTOSAVE_MS = 2000;

interface Props {
  task: Task | null;
  subtasks: Task[];
  /** Null while the first sync is still loading. */
  loading: boolean;
  onEdit: (id: string, patch: { title?: string; notes?: string }) => void;
  onSetDue: (id: string, due: string | null) => void;
  onToggle: (id: string) => void;
  onAddSubtask: (parentId: string, title: string) => Promise<string | null>;
  onOpenInGoogle: (id: string) => void;
}

/**
 * One task, in a window of its own.
 *
 * The sticky note is 340px wide because it is a note. This is the same task
 * with room to think in: a heading, a description that fills the window, and
 * the subtasks under it. Nothing here is a second copy of the data — it edits
 * the same task through the same commands, so the note behind it updates as
 * you type.
 *
 * The text sits in a column rather than spanning the window. A page can be
 * 900px wide; a *line* should not be, or it becomes unreadable at exactly the
 * moment you gave it more space.
 */
export function TaskPage(props: Props) {
  const { task } = props;
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const [dueOpen, setDueOpen] = useState(false);
  const [subtaskDraft, setSubtaskDraft] = useState("");
  const [saved, setSaved] = useState<"idle" | "saving" | "saved">("idle");
  const [length, setLength] = useState(0);

  const notesRef = useRef<HTMLDivElement>(null);
  const autosave = useRef<number | null>(null);

  const close = () => void invoke("close_task_page").catch(() => {});

  // Escape closes the page — the same key that closes everything else here.
  // Whatever is in the description is saved on the way out.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      const target = event.target;
      if (
        target instanceof Element &&
        target.closest("input, [contenteditable='true'], .due-popover")
      ) {
        return;
      }
      close();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(() => {
    return () => {
      if (autosave.current !== null) window.clearTimeout(autosave.current);
    };
  }, []);

  if (props.loading) {
    return <div className="page is-empty" data-tauri-drag-region />;
  }

  if (!task) {
    // Deleted, or completed and hidden, while the page was open. Saying so is
    // better than an empty window that looks broken.
    return (
      <div className="page" data-tauri-drag-region>
        <PageHead onClose={close} />
        <p className="page-gone">This task is no longer in the list.</p>
      </div>
    );
  }

  const done = task.status === "completed";

  /**
   * Saves what is in the editor a moment after typing stops.
   *
   * A description you sit in for twenty minutes should not depend on you
   * closing the window politely. The note behind this one updates at the same
   * moment, since both read the same task.
   */
  const scheduleSave = () => {
    const host = notesRef.current;
    if (host) setLength(host.innerText.length);
    if (autosave.current !== null) window.clearTimeout(autosave.current);
    setSaved("saving");
    autosave.current = window.setTimeout(() => {
      autosave.current = null;
      const editor = notesRef.current;
      if (!editor) return;
      void import("../../lib/richHtml").then(({ toMarkdown }) => {
        const markdown = toMarkdown(editor);
        if (markdown !== (task.notes ?? "")) props.onEdit(task.id, { notes: markdown });
        setSaved("saved");
        window.setTimeout(() => setSaved("idle"), 1600);
      });
    }, AUTOSAVE_MS);
  };

  const commitTitle = () => {
    const value = titleDraft.trim();
    if (value && value !== task.title) props.onEdit(task.id, { title: value });
    setEditingTitle(false);
  };

  const addSubtask = async () => {
    const title = subtaskDraft.trim();
    if (!title) return;
    setSubtaskDraft("");
    await props.onAddSubtask(task.id, title);
  };

  return (
    <div className="page">
      <PageHead onClose={close}>
        <span className={`page-state ${saved === "idle" ? "is-quiet" : ""}`}>
          {saved === "saving" ? "Saving…" : saved === "saved" ? "Saved" : ""}
        </span>
      </PageHead>

      <div className="page-body scroll-area">
        <div className="page-column">
          {editingTitle && !done ? (
            <textarea
              className="page-title-edit"
              autoFocus
              rows={1}
              value={titleDraft}
              maxLength={1024}
              onChange={(e) => setTitleDraft(e.target.value)}
              onBlur={commitTitle}
              onKeyDown={(e) => {
                if (e.key === "Enter" && !e.shiftKey) {
                  e.preventDefault();
                  commitTitle();
                } else if (e.key === "Escape") {
                  e.preventDefault();
                  e.stopPropagation();
                  commitTitle();
                }
              }}
            />
          ) : (
            <h1
              className={`page-title ${done ? "is-done" : ""}`}
              onClick={() => {
                if (done) return;
                setTitleDraft(task.title);
                setEditingTitle(true);
              }}
              title={done ? undefined : "Click to rename"}
            >
              {task.title}
            </h1>
          )}

          <div className="page-meta">
            <button
              className="page-check"
              role="checkbox"
              aria-checked={done}
              onClick={() => props.onToggle(task.id)}
            >
              {done ? "Completed" : "Mark complete"}
            </button>

            <DueChip
              label={task.due ? formatDue(task.due) : "Add date"}
              overdue={!done && isOverdue(task.due)}
              done={done}
              open={dueOpen}
              value={task.due}
              onOpen={() => setDueOpen(true)}
              onClose={() => setDueOpen(false)}
              onChange={(next) => {
                setDueOpen(false);
                props.onSetDue(task.id, next);
              }}
            />
            {!task.due && (
              <>
                <button
                  className="page-quick"
                  onClick={() => props.onSetDue(task.id, dueDateInDays(0))}
                >
                  Today
                </button>
                <button
                  className="page-quick"
                  onClick={() => props.onSetDue(task.id, dueDateInDays(1))}
                >
                  Tomorrow
                </button>
              </>
            )}
          </div>

          <FormatBar editor={notesRef.current} onChanged={scheduleSave} />

          <NotesEditor
            editorRef={notesRef}
            value={task.notes ?? ""}
            caret={null}
            onInput={scheduleSave}
            onCommit={(markdown) => {
              if (markdown !== (task.notes ?? "")) {
                props.onEdit(task.id, { notes: markdown });
              }
            }}
            onTabOut={() => {}}
            onTabBack={() => {}}
          />

          {length > WARN_AT && (
            <p className={`page-count ${length >= NOTES_LIMIT ? "is-over" : ""}`}>
              {length.toLocaleString()} of {NOTES_LIMIT.toLocaleString()} characters
              {length >= NOTES_LIMIT ? " — this is as much as Google will store" : ""}
            </p>
          )}

          <section className="page-subtasks">
            <h2>Subtasks</h2>
            {props.subtasks.length === 0 && (
              <p className="page-hint">Nothing under this task yet.</p>
            )}
            <ul>
              {props.subtasks.map((sub) => (
                <li key={sub.id}>
                  <button
                    className="page-sub-check"
                    role="checkbox"
                    aria-checked={sub.status === "completed"}
                    aria-label={`Complete "${sub.title}"`}
                    onClick={() => props.onToggle(sub.id)}
                  >
                    {sub.status === "completed" && (
                      <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true">
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
                  <span className={sub.status === "completed" ? "is-done" : ""}>
                    {sub.title}
                  </span>
                </li>
              ))}
            </ul>
            {!done && (
              <input
                className="page-sub-add"
                value={subtaskDraft}
                placeholder="Add a subtask…"
                maxLength={1024}
                onChange={(e) => setSubtaskDraft(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") {
                    e.preventDefault();
                    void addSubtask();
                  }
                }}
              />
            )}
          </section>

          <div className="page-foot">
            <button className="page-link" onClick={() => props.onOpenInGoogle(task.id)}>
              Open in Google Tasks
            </button>
            <button className="page-done" onClick={close}>
              Done
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function PageHead({
  onClose,
  children,
}: {
  onClose: () => void;
  children?: React.ReactNode;
}) {
  return (
    <header className="page-head" data-tauri-drag-region>
      {children}
      <button
        className="icon-button"
        onClick={onClose}
        aria-label="Close"
        title="Close — Escape"
      >
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
          <path
            d="M4 4l8 8M12 4l-8 8"
            stroke="currentColor"
            strokeWidth="1.5"
            strokeLinecap="round"
          />
        </svg>
      </button>
    </header>
  );
}

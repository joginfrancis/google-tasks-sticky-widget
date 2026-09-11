import {
  Fragment,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import type {
  Settings,
  SyncStatus as Status,
  Task,
  TaskList,
  WindowLayer,
} from "../../types";
import { TaskItem } from "../TaskItem/TaskItem";
import { useDragSession } from "../../hooks/useDragSession";
import { DragGhost } from "./DragGhost";
import { TaskInput } from "../TaskInput/TaskInput";
import { HeaderMenu } from "../HeaderMenu/HeaderMenu";
import { PinControl } from "./PinControl";
import { SyncButton } from "./SyncButton";
import { ListTitle } from "./ListTitle";
import { ColorBar, readableText } from "./ColorBar";
import { UndoStrip } from "./UndoStrip";
import "./TaskWidget.css";

interface Props {
  tasks: Task[];
  /** Reorder within the current list. Index is among top-level active rows. */
  onReorder: (id: string, toIndex: number) => void;
  taskLists: TaskList[];
  settings: Settings;
  status: Status;
  settlingIds: Set<string>;
  errors: Record<string, string>;
  /** Local-only note colour for the current list, or null for the default. */
  color: string | null;
  widgetError: string | null;
  pendingDelete: { title: string; extra: number } | null;
  onToggle: (id: string) => void;
  onDelete: (id: string) => void;
  onEdit: (id: string, patch: { title?: string; notes?: string }) => void;
  onSetDue: (id: string, due: string | null) => void;
  onOpenInGoogle: (id: string) => void;
  onMoveToList: (id: string, destinationListId: string) => void;
  onCreateList: (title: string) => Promise<string | null>;
  onRenameList: (id: string, title: string) => Promise<string | null>;
  onDeleteList: (id: string) => Promise<string | null>;
  onAdd: (title: string) => Promise<string | null>;
  onSelectList: (id: string) => void;
  onDuplicateNote: () => void;
  onChangeLayer: (layer: WindowLayer) => void;
  onChangeColor: (color: string | null) => void;
  onUndoDelete: () => void;
  onSyncNow: () => void;
  onOpenSettings: () => void;
  onHide: () => void;
  onQuit: () => void;
}

export function TaskWidget(props: Props) {
  const [completedOpen, setCompletedOpen] = useState(false);
  // One row at a time: several open at once would push the rest off a 340px
  // panel, and the point of expanding is to look at one thing.
  const [expandedId, setExpandedId] = useState<string | null>(null);
  const toggleExpand = (id: string) =>
    setExpandedId((current) => (current === id ? null : id));

  // Clicking anywhere that is not a task closes the open one. Without this the
  // only way to put a row away is to find and click it again, which is an odd
  // thing to have to do once you have already looked elsewhere.
  useEffect(() => {
    if (expandedId === null) return;

    const onPointerDown = (event: PointerEvent) => {
      const target = event.target as HTMLElement | null;
      // A popup belonging to the open row — the date picker especially — is
      // part of that row even when it is drawn outside the row's own box.
      if (target?.closest("li.task-item, .due-popover, .task-menu")) return;
      setExpandedId(null);
    };

    document.addEventListener("pointerdown", onPointerDown);
    return () => document.removeEventListener("pointerdown", onPointerDown);
  }, [expandedId]);

  const selectedId = props.settings.selectedTaskListId ?? "";

  // "Move to list" should never offer the list the task is already in.
  const otherLists = useMemo(
    () => props.taskLists.filter((l) => l.id !== selectedId),
    [props.taskLists, selectedId],
  );

  /**
   * A chosen colour replaces the surface tokens for this window only.
   *
   * Text is derived rather than picked, so no combination can end up
   * unreadable — see `readableText`.
   */
  const themeStyle = useMemo(() => {
    if (!props.color) return undefined;
    const text = readableText(props.color);
    return {
      "--surface": props.color,
      "--surface-header": props.color,
      "--surface-raised": props.color,
      "--text": text,
      "--text-secondary": `${text}bb`,
      "--text-muted": `${text}88`,
      "--border": `${text}22`,
      "--border-strong": `${text}44`,
    } as React.CSSProperties;
  }, [props.color]);

  /**
   * Ordering: parents in server order, each followed by its own children.
   * A settling task stays put until the animation ends, so the row the user
   * just clicked doesn't jump out from under the cursor.
   */
  const { active, completed } = useMemo(() => {
    const settling = props.settlingIds;
    const isActive = (t: Task) => t.status === "needsAction" || settling.has(t.id);

    const order = (list: Task[]) => {
      const parents = list.filter((t) => !t.parentId);
      const children = list.filter((t) => t.parentId);
      const out: Task[] = [];
      for (const parent of parents) {
        out.push(parent);
        out.push(...children.filter((c) => c.parentId === parent.id));
      }
      // Orphans — parent completed or deleted — must still be reachable.
      out.push(...children.filter((c) => !parents.some((p) => p.id === c.parentId)));
      return out;
    };

    return {
      active: order(props.tasks.filter(isActive)),
      completed: order(props.tasks.filter((t) => !isActive(t))),
    };
  }, [props.tasks, props.settlingIds]);

  // Only top-level, incomplete rows can be reordered: the move API positions
  // tasks among their siblings, and a subtask's position is owned by its parent.
  const reorderable = useMemo(
    () => active.filter((t) => !t.parentId),
    [active],
  );

  const listRef = useRef<HTMLUListElement>(null);

  /**
   * Which slot the pointer is over, measured from live row geometry rather than
   * a cached layout — rows change height when an error appears or a note wraps.
   *
   * Returns an insertion index in `reorderable`, or null when the pointer is
   * outside the list entirely.
   */
  const resolveIndex = useCallback(
    (client: { x: number; y: number }) => {
      const list = listRef.current;
      if (!list) return null;

      const bounds = list.getBoundingClientRect();
      const slack = 24; // Forgive a little overshoot past either end.
      if (
        client.y < bounds.top - slack ||
        client.y > bounds.bottom + slack ||
        client.x < bounds.left - slack ||
        client.x > bounds.right + slack
      ) {
        return null;
      }

      const rows = Array.from(
        list.querySelectorAll<HTMLElement>("li.task-item:not(.is-subtask)"),
      );
      for (let i = 0; i < rows.length; i += 1) {
        const rect = rows[i].getBoundingClientRect();
        // Past a row's midpoint means "after it", which is the next slot.
        if (client.y < rect.top + rect.height / 2) return i;
      }
      return rows.length;
    },
    [],
  );

  const { session, beginPress } = useDragSession({
    resolveIndex,
    onCommit: props.onReorder,
  });

  return (
    <div
      className={`widget${session !== null ? " is-dragging-active" : ""}`}
      style={themeStyle}
    >
      <header className="widget-header" data-tauri-drag-region>
        <button
          className="icon-button new-note-button"
          onClick={props.onDuplicateNote}
          aria-label="New note"
          title="New note for this list"
        >
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <path
              d="M8 3.2v9.6M3.2 8h9.6"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
            />
          </svg>
        </button>

        <ListTitle
          lists={props.taskLists}
          selectedId={selectedId}
          onSelect={props.onSelectList}
          onCreate={props.onCreateList}
          onRename={props.onRenameList}
        />

        {/* The header is now almost entirely buttons, so the window needs an
            explicit place to be grabbed. This stretches to fill whatever is
            left between the title and the controls. */}
        <div className="drag-handle" data-tauri-drag-region />

        <div className="widget-header-actions">
          <SyncButton status={props.status} onSyncNow={props.onSyncNow} />
          <PinControl
            layer={props.settings.windowLayer}
            onChange={props.onChangeLayer}
          />
          <HeaderMenu
            taskLists={props.taskLists}
            selectedListId={selectedId}
            onOpenSettings={props.onOpenSettings}
            onQuit={props.onQuit}
            onDeleteList={props.onDeleteList}
          />
          <button
            className="icon-button"
            onClick={props.onHide}
            aria-label="Close note"
            title="Close note"
          >
            <svg viewBox="0 0 16 16" width="11" height="11" aria-hidden="true">
              <path
                d="M3 3l10 10M13 3L3 13"
                stroke="currentColor"
                strokeWidth="1.5"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>
      </header>

      {/* Add-task sits directly under the heading, as in Google Tasks — capture
          is the most frequent action and should not need a scroll to reach. */}
      <div className="widget-add">
        <TaskInput onSubmit={props.onAdd} />
      </div>

      <div className="widget-body scroll-area">
        {active.length === 0 && completed.length === 0 ? (
          <EmptyState />
        ) : (
          <>
            {active.length === 0 && completed.length > 0 && (
              <p className="all-done">All done for now.</p>
            )}

            <ul className="task-list" ref={listRef}>
              {active.map((task) => {
                // The gap is drawn before the row currently occupying the slot,
                // so the list shows where a drop would land rather than where
                // the pointer is.
                const slot = reorderable.indexOf(task);
                const showGap =
                  session !== null &&
                  session.overIndex !== null &&
                  slot !== -1 &&
                  slot === session.overIndex;

                return (
                <Fragment key={task.id}>
                {showGap && (
                  <li className="drop-gap" aria-hidden="true" />
                )}
                <TaskItem
                  task={task}
                  isSubtask={Boolean(task.parentId)}
                  isDragging={session?.taskId === task.id}
                  onDragPress={
                    task.parentId
                      ? undefined
                      : (event, row) =>
                          beginPress({
                            taskId: task.id,
                            fromListId: selectedId ?? "",
                            fromIndex: reorderable.indexOf(task),
                            title: task.title,
                            row,
                            event,
                          })
                  }
                  isSettling={props.settlingIds.has(task.id)}
                  error={props.errors[task.id] ?? null}

                  isExpanded={expandedId === task.id}

                  onToggleExpand={toggleExpand}
                  onToggle={props.onToggle}
                  onDelete={props.onDelete}
                  onEdit={props.onEdit}
                  onSetDue={props.onSetDue}
                  onOpenInGoogle={props.onOpenInGoogle}
                  onMoveToList={props.onMoveToList}
                  otherLists={otherLists}
                />
                </Fragment>
                );
              })}
              {/* A drop past the last row lands here. */}
              {session !== null &&
                session.overIndex === reorderable.length && (
                  <li className="drop-gap" aria-hidden="true" />
                )}
            </ul>

            {completed.length > 0 && (
              <section className="completed">
                <button
                  className="completed-toggle"
                  onClick={() => setCompletedOpen((v) => !v)}
                  aria-expanded={completedOpen}
                >
                  <span className={`chevron-icon ${completedOpen ? "is-open" : ""}`}>
                    ▸
                  </span>
                  Completed ({completed.length})
                </button>

                {completedOpen && (
                  <ul className="task-list">
                    {completed.map((task) => (
                      <TaskItem
                        key={task.id}
                        task={task}
                        isSubtask={Boolean(task.parentId)}
                        isSettling={false}
                        error={props.errors[task.id] ?? null}

                        isExpanded={expandedId === task.id}

                        onToggleExpand={toggleExpand}
                        onToggle={props.onToggle}
                        onDelete={props.onDelete}
                        onEdit={props.onEdit}
                        onSetDue={props.onSetDue}
                        onOpenInGoogle={props.onOpenInGoogle}
                        onMoveToList={props.onMoveToList}
                        otherLists={otherLists}
                      />
                    ))}
                  </ul>
                )}
              </section>
            )}
          </>
        )}
      </div>

      {props.widgetError && <p className="widget-error">{props.widgetError}</p>}

      {props.pendingDelete && (
        <UndoStrip
          title={props.pendingDelete.title}
          extra={props.pendingDelete.extra}
          onUndo={props.onUndoDelete}
        />
      )}

      {/* Always present: collapsed it is a single wheel, so a menu item to
          reveal it would cost more than it saves. */}
      <ColorBar value={props.color} onChange={props.onChangeColor} />

      {/* Last child so it paints over the list without needing a z-index war,
          and inside `.widget` so it inherits this note's colour variables. */}
      {session !== null && <DragGhost session={session} />}
    </div>
  );
}

function EmptyState() {
  return (
    <div className="empty-state">
      <p className="empty-title">Nothing to do</p>
      <p className="empty-hint">Add a task above to get started.</p>
    </div>
  );
}

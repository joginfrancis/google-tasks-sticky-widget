import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { emit, listen } from "@tauri-apps/api/event";
import type { SyncStatus, Task, TaskList } from "../types";
import { isMainNote } from "../lib/window";

/**
 * How long a deleted task can be brought back. Long enough to notice the strip
 * and react, short enough that the deletion still feels like it happened.
 */
const UNDO_WINDOW_MS = 6000;

/**
 * How long a departed row stays hidden before this note checks for itself.
 *
 * A successful move announces both lists well inside this, so the re-read it
 * triggers normally settles the question first. This only matters when the move
 * failed and announced nothing, and it needs to be long enough not to race the
 * announcement into putting the row back for a frame.
 */
const DEPART_RECONCILE_MS = 2500;

/**
 * Moves `id` to `toIndex` among its top-level active siblings.
 *
 * `toIndex` is in the frame the move API uses: siblings with the moved task
 * already removed. Subtasks travel with their parent, and the widget regroups
 * children under parents when it renders, so only the parent order matters here.
 */
function reorderLocally(list: Task[], id: string, toIndex: number): Task[] {
  const moved = list.find((t) => t.id === id);
  if (!moved) return list;

  const children = list.filter((t) => t.parentId === id);
  const rest = list.filter((t) => t.id !== id && t.parentId !== id);
  const siblings = rest.filter(
    (t) => !t.parentId && t.status === "needsAction",
  );

  // Past the end means last, which is a legitimate drop rather than an error.
  const anchor = siblings[toIndex];
  if (!anchor) return [...rest, moved, ...children];

  const out: Task[] = [];
  for (const task of rest) {
    if (task.id === anchor.id) out.push(moved, ...children);
    out.push(task);
  }
  return out;
}

/**
 * Owns everything task-shaped: the cached list, the selected list id, sync
 * status, and the write path.
 *
 * Most mutations follow ARCHITECTURE §5.5 — apply locally, call Rust, reconcile
 * on success, revert on failure. There is no offline queue in v1 (SPEC §2.2), so
 * a write that ultimately fails is undone and reported on the row.
 *
 * `moveTask` is the exception and cannot be optimistic: `position` is
 * server-assigned, so the resulting order is unknown until Google answers.
 */
export function useTasks(connected: boolean) {
  const [tasks, setTasks] = useState<Task[]>([]);
  const [taskLists, setTaskLists] = useState<TaskList[]>([]);
  const [selectedListId, setSelectedListId] = useState<string | null>(null);
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [status, setStatus] = useState<SyncStatus>({
    state: "syncing",
    lastSyncedAt: null,
    message: null,
  });

  // Guards against a slow response for a list the user has already left.
  //
  // Written by `chooseList` rather than only during render: a `setState`
  // followed immediately by an awaited cache read would still see the previous
  // value here, and the read's result would be discarded as stale — which
  // silently defeated the whole paint-instantly-from-cache goal on launch.
  const currentListRef = useRef<string | null>(null);

  /** The delete waiting out its undo window, if any. */
  const pendingRef = useRef<{
    listId: string;
    taskId: string;
    timer: number;
  } | null>(null);
  const [pendingDelete, setPendingDelete] = useState<{
    title: string;
    /** Subtasks going with it, so the strip can say so. */
    extra: number;
  } | null>(null);

  const setError = useCallback((id: string, message: string | null) => {
    setErrors((prev) => {
      if (message === null) {
        if (!(id in prev)) return prev;
        const next = { ...prev };
        delete next[id];
        return next;
      }
      return { ...prev, [id]: message };
    });
  }, []);

  /** Single place the selected list changes, so the ref can never lag. */
  const chooseList = useCallback((id: string | null) => {
    currentListRef.current = id;
    setSelectedListId(id);
  }, []);

  /**
   * Rows hidden by a pending delete.
   *
   * Deletion waits out an undo window, so for those seconds the task is gone
   * from the screen but still in the cache and still in Google. Every re-read
   * would otherwise resurrect it — and a background sync lands inside that
   * window often enough that Delete looked like it simply did not work.
   */
  const suppressedRef = useRef<Set<string>>(new Set());

  /**
   * Other notes showing this list have their own copy of it, and nothing has
   * been deleted anywhere yet — so without telling them, a row deleted in one
   * note sits there in the other until the undo window expires. Both windows
   * hide it immediately, and Undo brings it back in both.
   */
  const broadcastPending = useCallback(
    (event: "task:pending-delete" | "task:pending-restore", taskId: string) => {
      void emit(event, { taskId, listId: currentListRef.current });
    },
    [],
  );

  const readCache = useCallback(async (listId: string) => {
    const cached = await invoke<Task[]>("list_tasks", { taskListId: listId });
    if (currentListRef.current !== listId) return;

    const hidden = suppressedRef.current;
    setTasks(
      hidden.size === 0
        ? cached
        : cached.filter(
            (t) =>
              !hidden.has(t.id) && !(t.parentId && hidden.has(t.parentId)),
          ),
    );
  }, []);

  /* -- Startup ------------------------------------------------------------ */

  useEffect(() => {
    if (!connected) {
      setTasks([]);
      setTaskLists([]);
      chooseList(null);
      return;
    }

    let cancelled = false;

    (async () => {
      try {
        const [cachedLists, saved] = await Promise.all([
          invoke<TaskList[]>("list_task_lists"),
          invoke<string | null>("get_selected_task_list"),
        ]);
        if (cancelled) return;

        // A fresh install has no cached lists, and everything downstream keys
        // off a selected list — so go to the network rather than wait for a
        // sync that can never start.
        let lists = cachedLists;
        if (lists.length === 0) {
          try {
            lists = await invoke<TaskList[]>("refresh_task_lists");
          } catch (err) {
            setStatus({
              state: /can't reach|cannot reach|network/i.test(String(err))
                ? "offline"
                : "error",
              lastSyncedAt: null,
              message: String(err),
            });
            return;
          }
        }
        if (cancelled) return;

        setTaskLists(lists);

        // Which list *this window* shows. Extra notes are pinned to the list in
        // their own label; only `main` follows the persisted selection.
        const pinned = await invoke<string | null>("note_list_id");
        if (cancelled) return;

        // Prefer the window's own list, then the saved one, but only if it
        // still exists — a list deleted in Google would otherwise leave the
        // window pointed at nothing.
        const exists = (id: string | null) =>
          Boolean(id && lists.some((l) => l.id === id));

        const chosen =
          (exists(pinned) && pinned) ||
          (exists(saved) && saved) ||
          lists[0]?.id ||
          null;
        chooseList(chosen);

        // Paint from cache before any network call — this is the whole point
        // of having one.
        if (chosen) {
          await readCache(chosen);
          // Point the scheduler at this list; it syncs immediately on being set.
          invoke("register_note_list", { listId: chosen }).catch(console.error);
        }
      } catch (err) {
        console.error(err);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [connected, readCache, chooseList]);

  /* -- Sync --------------------------------------------------------------- */

  const sync = useCallback(
    async (listId: string) => {
      setStatus((s) => ({ ...s, state: "syncing", message: null }));
      try {
        await invoke("sync_now", { taskListId: listId });
        await readCache(listId);
        setStatus({
          state: "synced",
          lastSyncedAt: new Date().toISOString(),
          message: null,
        });
      } catch (err) {
        const message = String(err);
        // Reaching Google at all is the distinction that matters here: an
        // unreachable server is "offline", anything else is a real error.
        const offline = /can't reach|cannot reach|network/i.test(message);
        setStatus((s) => ({
          state: offline ? "offline" : "error",
          lastSyncedAt: s.lastSyncedAt,
          message,
        }));
      }
    },
    [readCache],
  );

  // No initial sync here on purpose. Pointing the scheduler at a list makes it
  // sync immediately, and with no cursor yet that first delta *is* a full
  // fetch — so syncing here too would just double every launch and list switch.

  // Rust emits this after any successful sync, background polling included.
  useEffect(() => {
    const unlisten = listen<string>("tasks:updated", (event) => {
      if (event.payload === currentListRef.current) void readCache(event.payload);
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, [readCache]);

  // A delete waiting out its undo window in another note. Nothing has been
  // deleted yet, so this is purely about the two views agreeing: hide the row
  // here as well, and bring it back if that note's Undo is pressed.
  useEffect(() => {
    type Pending = { taskId: string; listId: string | null };

    const hide = listen<Pending>("task:pending-delete", (event) => {
      const { taskId, listId } = event.payload;
      if (listId !== currentListRef.current) return;
      suppressedRef.current.add(taskId);
      setTasks((prev) =>
        prev.filter((t) => t.id !== taskId && t.parentId !== taskId),
      );
    });

    const restore = listen<Pending>("task:pending-restore", (event) => {
      const { taskId, listId } = event.payload;
      if (listId === null || listId !== currentListRef.current) return;
      suppressedRef.current.delete(taskId);
      void readCache(listId);
    });

    return () => {
      void hide.then((fn) => fn());
      void restore.then((fn) => fn());
    };
  }, [readCache]);

  // The background scheduler owns sync state; mirror it rather than keeping a
  // second, divergent copy in the UI.
  useEffect(() => {
    const unlisten = listen<{
      state: SyncStatus["state"];
      lastSyncedAt: string | null;
      message: string | null;
    }>("sync:status", (event) => {
      setStatus({
        state: event.payload.state,
        lastSyncedAt: event.payload.lastSyncedAt,
        message: event.payload.message,
      });
    });
    return () => {
      unlisten.then((fn) => fn());
    };
  }, []);

  const selectList = useCallback(
    (id: string) => {
      chooseList(id);
      setTasks([]);

      // Only the main note owns the persisted selection. An extra note changing
      // it would silently repoint the main window too, which is not what
      // switching lists in one note should mean.
      if (isMainNote()) {
        invoke("set_selected_task_list", { taskListId: id }).catch(console.error);
      } else {
        // Still tell the scheduler, or this note's list would go unpolled.
        invoke("register_note_list", { listId: id }).catch(console.error);
      }

      void readCache(id);
    },
    [chooseList, readCache],
  );

  /* -- Mutations ---------------------------------------------------------- */

  const toggleTask = useCallback(
    async (id: string) => {
      const listId = currentListRef.current;
      if (!listId) return;

      const before = tasks.find((t) => t.id === id);
      if (!before) return;
      const completed = before.status !== "completed";

      setError(id, null);
      setTasks((prev) =>
        prev.map((t) =>
          t.id === id
            ? { ...t, status: completed ? "completed" : "needsAction" }
            : t,
        ),
      );

      try {
        const updated = await invoke<Task>("set_task_completed", {
          taskListId: listId,
          taskId: id,
          completed,
        });
        setTasks((prev) => prev.map((t) => (t.id === id ? updated : t)));
      } catch (err) {
        // Put the row back exactly as it was, then say why.
        setTasks((prev) => prev.map((t) => (t.id === id ? before : t)));
        setError(id, String(err));
      }
    },
    [tasks, setError],
  );

  /**
   * Deletion is deferred, not confirmed.
   *
   * Google has no recycle bin, so a mis-click on Delete destroys a task and its
   * notes permanently. A confirmation dialog would be the wrong fix — this is an
   * action taken dozens of times a day. Instead the row disappears immediately
   * and the API call waits behind an Undo window.
   *
   * Undo is a plain cache re-read, because nothing has been deleted anywhere
   * yet: the local SQLite row is untouched until the timer fires. If the app
   * closes inside the window the task simply survives, which is the safe way to
   * fail.
   */
  const commitDelete = useCallback(
    async (listId: string, taskId: string) => {
      pendingRef.current = null;
      setPendingDelete(null);

      try {
        await invoke("delete_task", { taskListId: listId, taskId });
        // Gone from Google and from the cache, so nothing is left to hide.
        suppressedRef.current.delete(taskId);
      } catch (err) {
        // The row is still in the cache, so lifting the suppression and
        // re-reading brings it back rather than leaving it deleted on screen
        // but alive in Google.
        suppressedRef.current.delete(taskId);
        await readCache(listId);
        setError(taskId, String(err));
      }
    },
    [readCache, setError],
  );

  /** Commits any pending delete straight away, without waiting out the timer. */
  const flushPendingDelete = useCallback(() => {
    const pending = pendingRef.current;
    if (!pending) return;
    window.clearTimeout(pending.timer);
    void commitDelete(pending.listId, pending.taskId);
  }, [commitDelete]);

  const deleteTask = useCallback(
    async (id: string) => {
      const listId = currentListRef.current;
      if (!listId) return;

      const doomed = tasks.filter((t) => t.id === id || t.parentId === id);
      if (doomed.length === 0) return;

      // A second delete commits the first rather than queueing — one pending
      // undo is comprehensible, a stack of them is not.
      flushPendingDelete();

      // Hide it from re-reads too, not just from the current render: a sync
      // landing inside the undo window would otherwise put the row straight
      // back, which is what made Delete look like it had done nothing.
      suppressedRef.current.add(id);
      setTasks((prev) => prev.filter((t) => t.id !== id && t.parentId !== id));
      broadcastPending("task:pending-delete", id);

      const timer = window.setTimeout(() => {
        void commitDelete(listId, id);
      }, UNDO_WINDOW_MS);

      pendingRef.current = { listId, taskId: id, timer };
      setPendingDelete({
        title: doomed[0].title,
        extra: doomed.length - 1,
      });
    },
    [tasks, flushPendingDelete, commitDelete],
  );

  const undoDelete = useCallback(() => {
    const pending = pendingRef.current;
    if (!pending) return;

    window.clearTimeout(pending.timer);
    pendingRef.current = null;
    setPendingDelete(null);
    // Nothing was deleted anywhere, so lifting the suppression and re-reading
    // is the whole of undo.
    suppressedRef.current.delete(pending.taskId);
    broadcastPending("task:pending-restore", pending.taskId);
    void readCache(pending.listId);
  }, [readCache]);

  /**
   * Title / notes / due edits share one path because they share one API call.
   * `due: null` clears the date; omitting `due` leaves it alone.
   */
  const editTask = useCallback(
    async (
      id: string,
      patch: { title?: string; notes?: string; due?: string | null },
    ) => {
      const listId = currentListRef.current;
      if (!listId) return;

      const before = tasks.find((t) => t.id === id);
      if (!before) return;

      setError(id, null);
      setTasks((prev) =>
        prev.map((t) => (t.id === id ? { ...t, ...patch } : t)),
      );

      try {
        const updated = await invoke<Task>("update_task", {
          taskListId: listId,
          taskId: id,
          title: patch.title,
          notes: patch.notes,
          due: patch.due ?? undefined,
          // Explicit, because IPC folds undefined and null together.
          clearDue: "due" in patch && patch.due === null,
        });
        setTasks((prev) => prev.map((t) => (t.id === id ? updated : t)));
      } catch (err) {
        setTasks((prev) => prev.map((t) => (t.id === id ? before : t)));
        setError(id, String(err));
      }
    },
    [tasks, setError],
  );

  /**
   * The single commit path for any change of position or list.
   *
   * Drag-and-drop, the context menu, and anything added later all end up here,
   * so there is one place where ordering can be wrong.
   *
   * Unlike the other mutations this is **not** optimistic beyond hiding the row:
   * `position` is server-assigned, so the resulting order is only known once
   * Google answers. Guessing it locally would show an order that then visibly
   * corrects itself.
   */
  const moveTask = useCallback(
    async (
      id: string,
      options: { toListId?: string; toIndex?: number } = {},
    ) => {
      const listId = currentListRef.current;
      if (!listId) return;

      const crossList = Boolean(options.toListId && options.toListId !== listId);
      setError(id, null);

      if (crossList) {
        // Leaving this list, so drop it from view immediately.
        setTasks((prev) => prev.filter((t) => t.id !== id && t.parentId !== id));
      } else if (options.toIndex !== undefined) {
        // Land the row where it was dropped before the network is consulted.
        // `move_task` is a round-trip to Google, and without this the list goes
        // on showing the old order for a second or more: the row appears to
        // snap back to where it started and then jump, which reads as a failed
        // drag. The readCache below still reconciles against what the server
        // actually did, and the catch re-reads if it refused.
        setTasks((prev) => reorderLocally(prev, id, options.toIndex as number));
      }

      try {
        await invoke("move_task", {
          taskListId: listId,
          taskId: id,
          destinationTaskListId: options.toListId ?? null,
          toIndex: options.toIndex ?? null,
        });
        // Rust resyncs the destination and emits tasks:updated; for a same-list
        // move this window re-reads through that listener.
        if (!crossList) await readCache(listId);
      } catch (err) {
        // Re-reading is safer than splicing rows back at a remembered index,
        // which would drift if a sync landed meanwhile.
        await readCache(listId);
        setError(id, String(err));
      }
    },
    [readCache, setError],
  );

  /**
   * Takes a task from another list into this one, at a position.
   *
   * The receiving window runs this, not the window the drag started in: it is
   * the one that knows its own list and where the drop landed, and it is still
   * open if the source note was closed mid-drag. `move_task` is told the
   * origin list because that is the collection the task must be moved *out* of.
   */
  const adoptTask = useCallback(
    async (taskId: string, fromListId: string, toIndex: number) => {
      const listId = currentListRef.current;
      if (!listId || fromListId === listId) return;

      setError(taskId, null);
      try {
        await invoke("move_task", {
          taskListId: fromListId,
          taskId,
          destinationTaskListId: listId,
          toIndex,
        });
        // Rust resyncs the destination, but re-read now so the row appears
        // without waiting for the event to come back round.
        await readCache(listId);
      } catch (err) {
        await readCache(listId);
        setError(taskId, String(err));
      }
    },
    [readCache, setError],
  );

  /**
   * The task has been dropped into another note; take it off this one now.
   *
   * Without this the row springs back to full brightness the instant the
   * pointer is released and only disappears once the move lands, so a
   * successful drag ends with a flash of the thing you just moved away.
   *
   * The receiving note performs the move, so this window is not told directly
   * whether it worked. It does not need to be: a move announces both lists, and
   * the re-read that follows either finds the task gone or puts it back. The
   * timer is the safety net for the case that announces nothing — a move that
   * failed outright — so the row cannot stay hidden on a task that never left.
   */
  const departTask = useCallback(
    (taskId: string) => {
      const listId = currentListRef.current;
      setTasks((prev) =>
        prev.filter((t) => t.id !== taskId && t.parentId !== taskId),
      );
      if (!listId) return;
      window.setTimeout(() => {
        if (currentListRef.current === listId) void readCache(listId);
      }, DEPART_RECONCILE_MS);
    },
    [readCache],
  );

  /** Kept as a named intent; the context menu reads better for it. */
  const moveTaskToList = useCallback(
    (id: string, destinationListId: string) =>
      moveTask(id, { toListId: destinationListId }),
    [moveTask],
  );

  const createList = useCallback(
    async (title: string): Promise<string | null> => {
      try {
        const created = await invoke<TaskList>("create_task_list", { title });
        setTaskLists((prev) => [...prev, created]);
        return null;
      } catch (err) {
        return String(err);
      }
    },
    [],
  );

  const renameList = useCallback(
    async (id: string, title: string): Promise<string | null> => {
      try {
        const updated = await invoke<TaskList>("rename_task_list", {
          taskListId: id,
          title,
        });
        setTaskLists((prev) => prev.map((l) => (l.id === id ? updated : l)));
        return null;
      } catch (err) {
        return String(err);
      }
    },
    [],
  );

  const deleteList = useCallback(
    async (id: string): Promise<string | null> => {
      try {
        await invoke("delete_task_list", { taskListId: id });
        const remaining = taskLists.filter((l) => l.id !== id);
        setTaskLists(remaining);

        // Deleting the list you were looking at has to land somewhere.
        if (currentListRef.current === id && remaining[0]) {
          chooseList(remaining[0].id);
          setTasks([]);
          invoke("set_selected_task_list", { taskListId: remaining[0].id }).catch(
            console.error,
          );
          void readCache(remaining[0].id);
        }
        return null;
      } catch (err) {
        return String(err);
      }
    },
    [taskLists, chooseList, readCache],
  );

  const openInGoogle = useCallback((id: string) => {
    invoke("open_task_in_google", { taskId: id }).catch((err: unknown) =>
      setError(id, String(err)),
    );
  }, [setError]);

  /**
   * Resolves to null on success, or the error message on failure, so the input
   * can hand the text back rather than losing what was typed (SPEC §3.3).
   */
  const addTask = useCallback(
    async (title: string): Promise<string | null> => {
      const listId = currentListRef.current;
      if (!listId) return "No task list selected.";

      const tempId = `pending-${crypto.randomUUID()}`;
      const optimistic: Task = {
        id: tempId,
        parentId: null,
        title,
        notes: null,
        due: null,
        status: "needsAction",
        position: "",
        updated: new Date().toISOString(),
      };
      setTasks((prev) => [optimistic, ...prev]);

      try {
        const created = await invoke<Task>("create_task", {
          taskListId: listId,
          title,
        });
        setTasks((prev) => prev.map((t) => (t.id === tempId ? created : t)));
        return null;
      } catch (err) {
        setTasks((prev) => prev.filter((t) => t.id !== tempId));
        return String(err);
      }
    },
    [],
  );

  return {
    tasks,
    taskLists,
    selectedListId,
    status,
    errors,
    selectList,
    syncNow: () => selectedListId && sync(selectedListId),
    toggleTask,
    deleteTask,
    addTask,
    editTask,
    openInGoogle,
    moveTask,
    moveTaskToList,
    adoptTask,
    departTask,
    createList,
    renameList,
    deleteList,
    pendingDelete,
    undoDelete,
  };
}

# Cross-Window Task Drag — Implementation Brief

You are implementing drag-and-drop of tasks **between sticky-note windows** in a Windows
sticky-note / task-list app. Several note windows are open on the desktop; each shows a
list of tasks. The user must be able to pick up a task in one note window and drop it
into a list in another note window.

**Scope is deliberately narrow: note window → note window only.** Nothing else is a
valid drop target. Not the desktop, not other applications, not the app's own non-note
windows. Anything else is a cancel. Hold this line — it is what keeps the work small.

This brief specifies **an approach**, not just a goal. The approach is deliberate and
non-obvious, so read it fully before writing code. It also rests on assumptions about
the codebase that you must verify first — **Phase 0 is mandatory and you must report
back before implementing anything.**

---

## 0. The core idea (read this first)

The two naive approaches both fail:

- **HTML5 drag-and-drop** does not cross WebView/window boundaries at all.
- **Native OLE drag (`IDataObject` / `DoDragDrop`)** does work, but requires serializing
  a drag payload, gives you only a static bitmap as the drag image, makes a custom drop
  indicator in the target list painful, and shares no code with the in-list reorder you
  already have.

**Instead: there is no drag payload. The drag is application state, and the dragged chip
is its own window.**

Three pieces:

1. **A `dragSession` record in the shared store.** All note windows are views over one
   store in one process. So "a drag is in progress, of task X, from list A, cursor at
   screen point P, hovering list B at index 3" is just state every window can already
   see. Nothing crosses a process boundary, so nothing needs serializing.

2. **A transparent, click-through ghost window** rendering the dragged chip, following
   the cursor. This is the trick that makes it work: you are not dragging *between* two
   WebViews, you are dragging a *third window* over the top of them.

3. **Native hit-testing.** The source window holds pointer capture for the whole drag.
   Each move, `WindowFromPoint(screenCursor)` says which window is under the cursor; if
   it is a note window, one throttled IPC call asks it which list and index sit at that
   client point. The answer goes into `dragSession`, and the target window renders its
   own drop indicator from its own store subscription.

The payoff: the commit is the *same* `moveTask` mutation used by in-list reorder and by
the context menu; the drop indicator is your own DOM, so it animates exactly like in-list
reorder; and the ghost is fully custom, so it can have the tilt and shadow of a real
sticky note.

The accepted cost: **you cannot drag a task out into another application.** Out of scope.
Windows Sticky Notes does not support it either. If it is ever wanted it is a separate
OLE drag-source path added later, and nothing here blocks that.

---

## Phase 0 — Validate before you build (MANDATORY)

Do not write feature code until you have done this and reported back.

Investigate the repository and answer each question with **file paths and line
references**, not from assumption.

### 0.1 Window and process model
- Is this Electron, WinUI 3 + WebView2, WPF + WebView2, Tauri, or something else?
- **Do all note windows run in one OS process?** If each window is a separate process
  the central premise fails — stop and report, because the store would need a
  cross-process channel first, which is a much larger change.
- How are note windows created today? Is there an existing window manager or registry of
  open windows? Where do window position and size get persisted?
- **Is there a reliable way to tell "this HWND is one of our note windows"?** Stage 3
  depends on it. If no such registry exists, note that — building one is part of the work.

### 0.2 State and store
- What is the state layer? (Zustand / Redux / MobX / custom emitter / direct SQLite reads)
- Is store state **shared live across windows**, or does each window hold its own copy
  synced by IPC or by re-reading a database? Write down exactly how a mutation in window
  A becomes a re-render in window B today — or state clearly that it does not.
- Where do tasks live? Schema of a task and of a list. What field determines order within
  a list: an integer index needing renumbering, a fractional rank, or a linked list?

### 0.3 Existing drag
- Is there in-list drag-to-reorder today? Where?
- Is it a library (dnd-kit, react-beautiful-dnd, SortableJS, HTML5 native) or custom?
- **Critically: is the reorder committed as a store mutation, or is it local component
  state that only writes on drop?** This decides how much of Stage 1 is real work.

### 0.4 IPC
- What is the IPC mechanism between the host layer and each WebView, and between windows?
- Is it request/response or fire-and-forget? Rough round-trip cost?

### 0.5 Platform access
- Can you call Win32 (`WindowFromPoint`, `SetCapture`, layered windows) — directly, via a
  native module, or only through a framework abstraction?
- Can you create a transparent, always-on-top, click-through, non-focusable window?
  Electron: `transparent`, `frame: false`, `focusable: false`, `hasShadow: false`, plus
  `setIgnoreMouseEvents(true)`. WinUI: `WS_EX_LAYERED | WS_EX_TRANSPARENT |
  WS_EX_NOACTIVATE`. Confirm which is actually available here.
- Is the app per-monitor DPI aware (v2)? Check the manifest / app config.

### 0.6 Report
Produce a short written report covering:
- Answers to all of the above, with file references.
- **Any point where this brief's assumptions are wrong**, and what that changes.
- Your proposed adaptation of the staged plan to the actual code.
- Anything in the plan you think is unnecessary, or missing.

Then **stop and wait for approval** before Phase 1.

---

## The staged plan

Build in this order. Each stage ships something usable on its own, and each exists to
de-risk the next. Do not skip ahead — Stage 1 in particular is what makes Stage 3 small,
and skipping it is what turns this into a rewrite.

---

### Stage 0 — `moveTask` mutation + "Move to list…" menu

**Goal:** one canonical way to move a task between lists, with no drag involved.

- Implement `moveTask(taskId, toListId, toIndex)` as a single store mutation, handling
  same-list reorder and cross-list move through the same code path.
- Add "Move to list…" to the task context menu, listing all lists — including ones whose
  windows are closed or minimized.
- Ensure the mutation propagates to every open window's view.

**Why first:** about an hour of work, it is the correctness backstop for every later
stage, and it is the only path that works when the target note isn't on screen. Every
later stage commits through this exact function.

**Done when:** moving a task via the menu updates source and target windows live and
survives an app restart.

---

### Stage 1 — Re-express in-list drag as `dragSession` + `moveTask`

**Goal:** no new user-visible behavior. This is a refactor, and it is the single most
important stage.

Add to the shared store:

```ts
type DragSession = {
  taskId: string
  fromListId: string
  fromIndex: number
  // physical screen pixels — one canonical space, see DPI below
  cursor: { x: number; y: number }
  // offset cursor→chip top-left, so the chip does not jump on grab
  grabOffset: { x: number; y: number }
  // current drop target; null when over anything that is not a note list
  overWindowId: string | null
  overListId: string | null
  overIndex: number | null
  // enough to render the ghost without re-querying
  ghost: { width: number; height: number; title: string; color: string }
}

// store
dragSession: DragSession | null
dragStart(session: DragSession): void
dragMove(cursor: Point, over: { windowId, listId, index } | null): void
dragCancel(): void
dragCommit(): void   // calls moveTask(...) then clears
```

Rewrite existing in-list reorder onto it:

- Pointer-down + movement threshold → `dragStart`
- Pointer-move → `dragMove`
- Pointer-up → `dragCommit`; Escape or capture-lost → `dragCancel`
- The list renders its gap / drop indicator **purely from `dragSession.overListId` and
  `overIndex`**, never from local component state.

If in-list reorder currently uses a DnD library that owns this state internally, you will
likely need to replace it with a custom pointer-event implementation. That is expected
and is the point of the stage — the library cannot see across windows.

**Why:** after this, a cross-window drop is *the same operation* as an in-list drop with a
different `overListId`. Everything remaining is plumbing.

**Done when:** in-list reorder behaves exactly as before, every frame of it is readable
from the store, and the drop commits via `moveTask`.

---

### Stage 2 — The ghost window

**Goal:** the chip lifts out of the list and follows the cursor anywhere on screen.
Dropping anywhere animates it back (every drop is still a no-op at this stage).

- On `dragStart`, show the ghost window: transparent, frameless, always-on-top,
  **non-focusable**, **click-through**. Click-through is essential — the ghost must not
  intercept the hit-testing Stage 3 depends on.
- Size it to the chip plus room for shadow. Each move, position it at
  `cursor - grabOffset`, converted into that monitor's coordinate space.
- Render the chip: task title, the note's colour, a slight tilt, a drop shadow. This is
  where the physical-sticky-note feel comes from — spend a little care here.
- The source list shows the task as a faded placeholder / collapsing gap while dragging.
- On `dragCancel` / `dragCommit`, hide the ghost.

**Reuse one ghost window.** Create it lazily and hide/show it rather than creating and
destroying a window per drag — window creation is slow enough to be visible at drag start.

**Why before Stage 3:** this debugs all the cursor, capture and window plumbing while the
semantics are still trivial. Do not merge it with Stage 3.

**Done when:** you can drag a task out of its window, across the desktop, over other
windows, and release anywhere with no crash, no orphaned ghost, and the task returning
cleanly to its original position.

---

### Stage 3 — Hit-test note windows and drop

**Goal:** the actual feature.

Each throttled move (see Performance):

1. `WindowFromPoint(screenCursor)` → HWND. Look it up in the note-window registry.
   **If it is not a note window, `over = null`** — desktop, other apps, and the app's own
   non-note windows all fall out here for free. If it resolves to the ghost window you
   have a click-through bug; fix that rather than working around it.
2. If it is a note window, send it a request: *"what list and drop index are at client
   point (x, y)?"* It answers from its own DOM, reusing the same index-calculation code
   the source window uses for in-list reorder.
3. Write the answer into `dragSession` via `dragMove`.
4. The target window, subscribed to the store, renders its drop indicator: a gap opening
   at `overIndex`, animated the same way as in-list reorder.
5. On pointer-up, `dragCommit` → `moveTask(taskId, overListId, overIndex)`.

Also handle:
- **A note window partially behind another window.** `WindowFromPoint` returns the
  topmost window at that point, which is the correct answer — you should not drop into an
  obscured window. Do not try to be clever here.
- **Hovering a note window's header/chrome rather than its list** → `over = null`, no
  indicator, drop cancels.
- **Auto-scroll** the target list when hovering near its top or bottom edge.
- **Optional:** if the cursor rests over a note window for ~600ms, raise that window.
  Evaluate it; if it feels twitchy, drop it.

**Done when:** dragging a task from note A into a list in note B lands it at the indicated
position, both windows update immediately, and the change persists.

---

## The two things that will actually bite you

### Pointer capture — the target gets no mouse events

Once the source window captures the pointer, mouse events over *other* windows still route
to the source. That is exactly what you want for tracking, but it has a consequence that
breaks naive implementations:

> **The target window receives no native mouse events during the drag.**

So every piece of hover feedback in the target — drop indicator, gap, highlight — must be
driven from `dragSession` pushed by the drag owner, never from the target's own mouse
handlers. Stage 1 sets this up correctly by construction. If you find yourself wanting to
add a `mousemove` handler in the target window, stop: that is the signal you have drifted
off the design.

Handle capture loss too: `WM_CANCELMODE`, alt-tab, a system dialog appearing, the source
window closing mid-drag. All route to `dragCancel`, and `dragCancel` must hide the ghost.
An orphaned always-on-top ghost stuck on the user's screen is the worst failure mode this
design has, so make it unreachable — hide the ghost on window-blur, on capture-lost, and
in a `finally` around the move handler.

### DPI and multi-monitor

Mixed-scaling multi-monitor setups hand you screen coordinates in one space and client
coordinates in another. Get this right up front; discovering it later means re-checking
every coordinate in the system.

- Keep `dragSession.cursor` in **physical screen pixels**. One canonical space.
- Convert at each boundary: client→screen on the source, screen→client on the target,
  screen→window-position for the ghost.
- Make the ghost per-monitor DPI aware (v2) and re-scale its contents when it crosses
  monitors.
- **Test explicitly** on two monitors at different scale factors (e.g. 150% and 100%),
  dragging across the boundary both ways. The symptom of getting it wrong is the chip
  drifting a few pixels off the cursor on the secondary display.

---

## Performance

- Throttle `dragMove` to animation frames: one store write and at most one hit-test IPC
  round-trip per frame. Never hit-test per raw mouse event.
- Skip the IPC hit-test entirely when the cursor is still inside the same list region as
  the previous frame; recompute the index locally.
- Update the ghost's *position* natively each frame; re-render its *contents* only when
  the drag target changes.
- Budget: dragging over a list of 200 tasks stays at 60fps.

---

## Test checklist

Functional:
- [ ] Drag within a list reorders (Stage 1 regression — re-check every stage)
- [ ] Drag note A → note B: top, middle, and bottom of the target list
- [ ] Drag into an empty list
- [ ] Drag to note B and back to note A's original position
- [ ] Drop on the desktop → cancels, task returns
- [ ] Drop on a non-app window (e.g. Explorer) → cancels
- [ ] Drop on a note window's header → cancels
- [ ] Escape mid-drag → cancels
- [ ] Alt-tab mid-drag → cancels, ghost disappears
- [ ] Target note closed mid-drag → cancels cleanly
- [ ] Source note closed mid-drag → cancels cleanly, no orphaned ghost
- [ ] "Move to list…" menu still works and agrees with drag results
- [ ] All changes persist across app restart

Environmental:
- [ ] Two monitors at different DPI, dragging across the boundary both ways
- [ ] Target note partially obscured by another window
- [ ] Several notes open (4+)
- [ ] Long list requiring auto-scroll at both edges

---

## Out of scope

- Dragging a task out to another application (Explorer, Outlook, a text editor)
- Dragging a whole note/window into another note
- Dropping onto a minimized or hidden note — "Move to list…" covers that case
- Multi-select drag of several tasks at once. Shape `dragSession` so `taskId` could become
  `taskIds` later, but do not build it.

---

## Working agreement

- Complete Phase 0 and report before writing feature code.
- One stage per commit or PR. Do not start a stage before the previous one is verified.
- Stage 1 is a pure refactor: if in-list reorder behaves differently afterward, that is a
  bug, not an improvement.
- If the design fights the codebase at any point — particularly if note windows turn out
  not to share a process or a store — stop and say so rather than working around it.

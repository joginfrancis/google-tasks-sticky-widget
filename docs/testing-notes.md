# Testing notes

Observations from using the app, recorded as they come up. **Nothing here is
built.** The point of writing them down rather than acting on each one is that a
rebuild costs four minutes and a release costs more than that — so they
accumulate until there is enough to justify a round.

Each entry says what was asked for, what the code does today, and what building
it would actually involve. Where something turns out to exist already, that is
recorded too: knowing a thing is done is as useful as knowing it is not.

Started 7 October 2026, during the first real testing phase.

---

## 1. Drag a multi-selection into another note

**Asked for:** with several tasks selected, dragging should carry the whole
selection to another note, not just the row under the pointer.

**Today:** not implemented. A drag carries exactly one task — `beginPress` takes
a single `taskId`, and the cross-window protocol broadcasts that one id
(`src/components/TaskWidget/TaskWidget.tsx`, `src/hooks/useDragSession.ts`,
`src/lib/dragBus.ts`). Starting a drag on a selected row drags that row alone and
leaves the selection behind.

**Already done, as asked:** deleting a whole selection *is* implemented —
`onDeleteMany` with a confirmation in the selection bar and a single Undo
covering every task (`confirmDeleteSelected`). Copy and complete work on the
selection too.

**What it would take:**
- The drag session carries `taskIds: string[]` rather than one id, and the ghost
  shows a count ("3 tasks") instead of a title.
- `dragBus` payloads carry the list; the receiving note adopts them in order.
- Order matters on arrival: Google positions each task by `previous`, so they
  have to be inserted one after another or they land reversed.
- A subtask in the selection whose parent is not selected needs a rule — most
  likely it travels as a top-level task, the same as dragging it out alone.
- Failure is partial by nature: five tasks, three moved, two refused. The undo
  story for that needs deciding before the code is written.

**Size:** roughly half a day, most of it in the partial-failure behaviour rather
than the dragging.

---

## 2. A closed note comes back on the shortcut

**Seen:** the left-hand *Fab Daily* note has been closed many times and
reappears every time `Ctrl+Shift+G` is pressed.

**Cause, confirmed in the code:** that note is the `main` window, and `main`
does not close. `WindowEvent::CloseRequested` calls `api.prevent_close()` and
hides it instead (`src-tauri/src/lib.rs`). The shortcut's show branch then shows
*every* note window, hidden or not (`src-tauri/src/hotkey.rs`), so a window that
was only ever hidden comes straight back.

It is not a stale-state bug and no amount of closing will fix it: pressing × on
the main note and pressing the shortcut are, as written today, opposite halves
of the same pair.

**Why `main` was made special:** the app runs with `skipTaskbar`, so if the last
window really closed there would be nothing on screen and nothing in the
taskbar — only the tray icon. Hiding rather than closing was the guard against
"the app has vanished".

**Related, visible in the same screenshot:** *Fab Daily* is open twice — the
`main` window and a second note on the same list. Two windows showing one list is
allowed by design, but when one of them cannot be closed it reads as a bug
rather than a choice.

---

## 3. Closing a note should remove it from the layout

**Asked for:** closing a note takes it out of the arrangement for good. The list
itself stays in the dropdown, so it can be opened again deliberately — but the
next show/hide cycle brings back only the notes that are still open.

**Today:** closing an *extra* note does exactly this already — it is removed
from the registry and from `open_notes`, so it does not return
(`persist_open_notes`). Only `main` misbehaves, for the reason in §2.

**What it would take:**
- `main` stops being special: closing it closes it, and it leaves `open_notes`
  like any other note.
- The guard that made it special needs replacing, or closing the last note
  leaves the app running with nothing on screen. Options:
  - the shortcut, pressed with no notes open, reopens the last arrangement —
    which makes it "show my notes" rather than "unhide windows";
  - or the tray menu grows an explicit *Open a note*, and the shortcut does
    nothing when there is nothing to show;
  - or closing the last note quits, which is honest but surprising for a widget
    that lives in the tray.
- The hotkey's show branch must distinguish *hidden* from *closed*. It currently
  shows every window it can find, which is the half of the bug that makes a
  hidden window indistinguishable from an open one.
- `restore_session` on launch should reopen exactly what was open, with no
  implicit main note. It already reads `open_notes`; the main window is created
  by the Tauri config before that runs, which is the other place "main is
  special" is written down.

**Size:** half a day, nearly all of it in deciding what the shortcut means when
nothing is open — the code change is small and the behaviour change is not.

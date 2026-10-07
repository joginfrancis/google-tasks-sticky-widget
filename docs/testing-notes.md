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

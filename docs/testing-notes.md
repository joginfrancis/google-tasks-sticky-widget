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

**Decided, 12 October:**

- **× removes a note from the layout.** Not from Google, not from the list
  dropdown — the list stays there to be opened again. `main` loses its exemption
  and closes like any other note.
- **The shortcut means "show my notes".** Pressed when nothing is open, it
  reopens the arrangement that was last on screen. If there is no arrangement to
  reopen — first run, or every note closed and the record emptied — it opens one
  note on the default list.
- **It never opens a second copy of something already on screen.** Reopening
  the arrangement skips any note already there and focuses the set rather than
  duplicating it. The duplicate *Fab Daily* in the screenshot is the shape of
  that mistake.

So there are three states and the shortcut handles each: notes visible → hide
them; notes hidden → show them; nothing open → reopen the last arrangement, or
the default note if there was none.

The record to reopen from is `open_notes` in `window.json`, which already holds
label → list id and is written after every open and close. What it needs is to
survive the closing of the *last* note rather than being emptied by it — the
arrangement is what the user wants back, and emptying it is what would lose it.

---

## 4. Does the layout survive a shutdown or a reinstall?

**Today: yes, with one gap.** None of it lives in the installed folder, so
uninstalling and reinstalling does not touch it:

| What | Where |
| --- | --- |
| Which notes are open, and on which list | `%APPDATA%\in.startupmission.stickywidget\window.json` (`openNotes`) |
| Each note's position and size | `%LOCALAPPDATA%\in.startupmission.stickywidget\.window-state.json` |
| Note colours, pins, preferences | `window.json` |
| The Google sign-in | Windows Credential Manager |
| The task cache | `%LOCALAPPDATA%\…\cache.sqlite` |

This has been exercised by accident: the app has been uninstalled and
reinstalled several times during development and the notes, their lists and the
sign-in came back every time.

**When it is written:** `openNotes` is written after *every* open and close, not
at exit — deliberately, so a crash or a power cut cannot lose the arrangement.
Window geometry is different: the window-state plugin writes on close and on
exit, so a hard power-off can lose the last *move or resize* while still keeping
the right notes on the right lists.

**The gap:** uninstalling through Windows runs the NSIS uninstaller, which can
offer to remove application data. Answering yes to that would take the lot —
layout, colours, cache. The sign-in would survive, since it is in Credential
Manager rather than a folder. Worth checking what that prompt actually says
before telling anyone else "reinstalling is safe".

**Related, and the reason this matters for §3:** if closing the last note
empties `openNotes`, then "survives a shutdown" and "reopens the last
arrangement" both break in the same move. The record has to outlive the last
window.

**To verify (not yet done):**
- Uninstall via Settings → Apps and read the prompt wording; note whether the
  default is to keep or delete.
- Power-off test: arrange notes, pull the plug, start again — expect right notes
  and lists, possibly stale positions.

---

## 5. Auto-layout for the open notes

**Asked for:** a way to have the notes arrange themselves rather than being
dragged into place one at a time.

**Today:** placement is manual, with one assist — a new note opens *beside* the
note it was opened from, to the right if there is room, to the left if not,
cascading only when neither fits (`open_note_labelled`). Positions are saved per
label by the window-state plugin, so an arrangement survives a restart. Nothing
tidies an arrangement that has drifted, and nothing lays out a fresh set.

**Shapes this could take, roughly in order of usefulness:**

- **Arrange now** — a tray item, or one in the ⋮ menu: lay every open note out
  in columns from one edge of the work area, 340px wide, a gap between them,
  wrapping to a second column when the screen runs out. The screenshot of five
  notes along the right-hand side is already this arrangement, built by hand.
- **Pick an edge** — the same thing, but left/right/top-left as a choice, since
  which side is free depends on what else is on screen.
- **Equal heights** — notes in a column share the height available rather than
  each keeping whatever height it was dragged to.
- **Keep tidy** — re-run the arrangement whenever a note opens or closes. This
  is the one to be careful with: it overrides deliberate placement, so it should
  be a setting that is off by default, not the behaviour.

**What it would take:**
- The work area of the monitor each note is on (already used by `note_set_tall`
  and `note_grow`), and the DPI scale per monitor for a multi-screen desk.
- A pass that assigns positions in label order, so the result is stable: running
  it twice produces the same layout.
- The existing `animate` helper can move each window, so notes slide into place
  rather than teleporting.
- **Undo matters more than the layout.** Arranging destroys an arrangement
  somebody may have spent time on. Positions should be captured before the move
  and restorable from the same menu for as long as the app is running.
- Notes on a second monitor need a rule: arrange per-monitor where they already
  are, rather than dragging everything onto the primary screen.

**Size:** about half a day for *Arrange now* with undo; the "keep tidy" setting
is another couple of hours on top, and is the part most likely to annoy.

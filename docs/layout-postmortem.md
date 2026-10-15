# Post-mortem: the layout and align tools

Written 15 October 2026, after the layout work in v0.1.3–v0.1.4. Nothing here
is a plan to build; it is a record of why this one feature needed twenty-two
commits in four days, what the user experience actually is now, and which parts
of screen switching are verified and which are not.

## 1. The defect ledger

Nine distinct defects shipped and were then fixed inside this one feature,
grouped by what was actually wrong rather than by how it looked.

### Deadlocks (three fixes, two of them wrong)

1. **The app froze whenever Arrange or Align was pressed.** First theory: the
   move animation was generating too many events. The animation was removed.
   The freeze stayed.
2. **Second theory: the window calls needed to be on the main thread.** They
   were moved into `run_on_main_thread`. This made it *worse* — `set_position`
   and `set_size` post to the event loop and wait for it, so running them
   *inside* the event loop is a guaranteed self-wait.
3. **The actual cause**: moving windows produces a storm of `Moved`/`Resized`
   events, and the geometry writer called `save_window_state`, which takes the
   same plugin lock that the plugin's own event hook holds while handling those
   events. Fix: placement happens on a plain OS thread, and saving is debounced
   until 1.2s of stillness (`LAYOUT_BUSY` + `GEOMETRY_QUIET_MS`).

A fourth, earlier instance of the same class: the task page window opened blank
and unclosable because `open_task_page` was a synchronous command. Tauri
commands that create windows must be `async`. The codebase already carried that
warning in a comment above `open_note_window`; it was not read before the next
window command was written.

### Bad data written into the saved layout (two)

4. **Only one note came back after unplugging a display.** Windows reports
   `-32000,-32000` at `160x28` for a hidden or minimised window — the parking
   spot it uses for iconic windows. The arrangement had been captured while the
   notes were hidden by the shortcut, so four notes were saved at the parking
   spot. Fix: never capture a window that is not visible, never overwrite a
   complete arrangement with a partial one, drop parked entries on load.
5. **Still only one note.** The cleanup in (4) left a *one-note* remembered
   arrangement, which still counted as "remembered" and therefore outranked the
   correct five-note relative layout. Fix: a remembered arrangement is only used
   when it covers every note; shorter ones are discarded at load.

### Wrong arithmetic (two)

6. **Notes returned to the wrong screen.** Monitor origins move: the laptop
   panel sits at `0,0` when alone and at `1920,0` when a monitor is attached on
   its left. Saved coordinates were replayed verbatim. Fix: `LayoutNote` stores
   `monitor_x`/`monitor_y` and the restore translates by the delta.
7. **The order shuffled on a screen change, and the spacing came out uneven.**
   The new arrangement was computed from where the notes *currently* were — but
   by the time a display change is observed, Windows has already dragged them
   into a heap, so the heap was what got mapped. Separately, scaling each note
   by its own fractions stretches the gaps while leaving the notes alone. Fix:
   `relative_layout` stores proportions of the arrangement's own bounding box
   (the "media query" model that was asked for), and `fit_arrangement` applies
   one uniform scale, centred, capped at 1.35×.

### The shortcut (two)

8. **Ctrl+Shift+G appeared not to work.** Key auto-repeat fired the handler
   twice, so the toggle cancelled itself — the log showed a show and a hide in
   the same second. A 350ms guard was not enough; the repeat was measured at
   582ms; the guard is now 700ms.
9. **The guard then swallowed the first press after launch**, because the
   elapsed-time counter starts at 0 and `0 - 0 < 700`. Fix: a `last != 0`
   sentinel. Verified afterwards by sending the keystroke from PowerShell and
   reading window rectangles through `EnumWindows`: 5 visible → 0 → 5.

## 2. Why it kept failing

The nine defects have four causes between them, and only one of them is about
Windows being awkward.

**Fixing symptoms before locating causes.** The freeze got three fixes; the
first two were guesses dressed as explanations, and the second actively made
things worse. No log line was added before the first attempt. Once the code was
instrumented, the real cause was found in a single pass. The cost of guessing
was two reinstall-and-retest cycles, each about four minutes of build plus a
round trip asking whether it was better.

**No tests on the only part that was testable.** Window placement cannot be
unit-tested — whether Windows honours a `set_position`, what a monitor reports
while it is being unplugged. But defects (6) and (7) were pure arithmetic on
integers, exactly the part that *can* be tested, and there were no tests until
after both had shipped. There are now six (`layout_tests`, `relative_tests`),
and they encode the two bugs that had already escaped.

**Persisted state was trusted as if it were correct.** The system writes a
layout file and reads it back later under different conditions. Defects (4) and
(5) are the same mistake twice: no validation at the boundary. A value of
`-32000` is not a position and never was; a one-note arrangement for a
five-note session is not a layout. Both are rejected on load now, but each
check was written only after the failure it prevents had already been seen.

**Verification depended on the user.** For most of this work the only signal
was "it's not working", which does not distinguish between nine different
faults. The loop got dramatically faster the moment the app's own log was made
useful (2MB files, `KeepAll`) and window geometry could be read directly with
`EnumWindows`. That tooling should have existed before the first layout commit,
not after the eighth.

## 3. The user experience, assessed honestly

**There are six concepts behind one goal.** Named layouts, per-display pixel
memory, the relative shape, Arrange, Align, and the Screen picker. Someone who
simply wants their notes in the right place has to hold all six, and nothing on
screen explains how they relate. "What are these?" was asked twice during this
work. That is the clearest UX finding in the whole exercise, and it is
unresolved — the controls were reduced, the model was not.

**What did improve.**

- The dropdown applies on change. A separate "Use" button asked for the same
  thing to be said twice.
- Five labelled buttons became an icon strip (Arrange, Align, Save, New,
  delete), with the save marks lighting up only when there is something unsaved.
- Every action says what it is doing immediately ("Arranging…", "Restoring
  Work…") and then what it did ("Arranged 5 notes — keep it?"). The original
  version was silent while a background thread moved windows, which read as a
  freeze even when it was not.
- Screens are labelled "Screen 2 (main) — 1920 × 1080", sorted left to right.
  Both panels were labelled "1920 × 1080" before, because `Monitor::size()`
  reports physical pixels and the two came out identical.

**What is still wrong.**

- **No undo.** Arrange and Align destroy the previous arrangement, and the only
  recovery is a layout that was already saved. "Arranged 5 notes — keep it?"
  implies there is a way to decline, and there isn't one. Worse: the planning
  note in `testing-notes.md` said "**Undo matters more than the layout**" before
  any of this was built, and it was built without undo anyway.
- **Failure is silent.** If a note does not move — its list is gone, it is
  hidden, its monitor vanished — the count simply comes out lower. The status
  line says "Arranged 4 notes" when there are five, and nothing says which one
  was left behind or why.
- **The Screen picker disagrees with reality while it is working.** Its value
  was derived from live occupancy, which lags the background placement, so it
  worked exactly once per opening of the panel. It now holds a local choice and
  re-reads after a delay: the symptom is fixed, but the control can still
  briefly contradict the screen.
- **Align's rules are invisible.** "Nearest tidy structure" means equal widths,
  integral multiples of a base width, even heights down a column. Nothing says
  so, so when it moves a note somewhere unexpected it looks like a bug rather
  than a rule being applied.
- **Hidden notes interact badly with everything.** The hide/show shortcut and
  the layout tools share state, and the one data-corruption bug in this list
  came from their intersection. A hidden note is now invisible to capture by
  design, which is correct, but it also means pressing Arrange while notes are
  hidden quietly does less than it claims.

## 4. Screen switching: what is verified

**Verified by direct measurement** — Win32 `EnumWindows`/`GetWindowRect` against
the running app, not by reading the code:

- Five notes on the main screen, correct proportions, nothing off-screen:
  `10,10 499x810 | 648,10 499x401 | 1286,10 499x401 | 648,521 499x401 | 1286,521 499x401`.
- The remembered set for `\\.\DISPLAY1|\\.\DISPLAY6` holds 5 notes, and
  `relativeLayout` holds 5 entries.
- `move_notes_to_screen` moves every note, and can be used more than once.
- The shortcut's three states: 5 visible → hidden → 5 restored.
- The arithmetic: proportions preserved onto a smaller screen, nothing
  overflowing the work area, a single note not inflated to fill a 4K panel, a
  note following its display when that display's origin moves.

**Not verified.** The actual physical event — pulling the cable out and putting
it back. `DisplaySwitch.exe` has no effect on this machine, and reconfiguring
the displays through the Windows API was not something to do unasked. So the
handling of a real unplug — whether `watch_screens` fires, whether the work area
reported mid-transition is sane, whether the projection runs before or after
Windows has finished dragging the windows itself — rests on the log and on
reasoning, not on observation. Every bug in section 1 that involved a display
changing was found *after* shipping, which is exactly the risk here.

**The one test worth running by hand**: with five notes arranged across both
screens, unplug the second display, wait for the notes to settle, then plug it
back in. If the single-screen view shows all five in the same shape, and the
two-screen view comes back as it was, the feature is done. If it does not, the
log now keeps enough history to say why in a single pass.

## 5. What would prevent the next round

In rough order of value:

1. **Instrument before fixing.** Three of the nine defects cost an extra
   build-and-reinstall cycle because a theory was implemented instead of a log
   line. The rule in one sentence: if the failure cannot be seen in the log, add
   the log first.
2. **Validate on the way in, not just on the way out.** The layout file is read
   back under conditions that did not exist when it was written. Treat it as
   untrusted input: reject parked coordinates, reject sets smaller than the
   session, reject monitors that no longer exist.
3. **Undo for layout operations.** Keep the previous arrangement in memory and
   offer one step back. It is what makes Arrange and Align safe to try, and
   being safe to try is what makes them learnable.
4. **Say what did not happen.** "Arranged 4 of 5 — one note's list is gone" is
   worth more than a correct-looking count.
5. **Collapse the model before adding to it.** Two concepts — saved layouts,
   and a tidy-up — with screens handled automatically and invisibly. The
   per-display memory and the relative projection are implementation detail that
   leaked into the interface.
6. **Re-read the comments already in the file.** The async-command deadlock was
   documented above a neighbouring function and then repeated anyway.

## 6. What was resolved, 15 October 2026

Worked through in the order of section 3, one commit per idea. Everything here
is in the build; what is *verified* is listed in section 4, and the additions
below are marked honestly.

| Item | Fix | How far it is verified |
| --- | --- | --- |
| Failure is silent | Every layout command answers with `{moved, total, skipped}` — a phrase per reason, not just a count. "Arranged 4 of 5 notes — left 1 note whose list is gone". | Rust test for the phrasing and plurals; frontend test for the sentence; not yet seen on screen. |
| No undo | `remember_for_undo` captures the arrangement before Arrange, Align, restoring a preset or moving screens; the Undo mark puts it back, and keeps the replaced arrangement so undo is a toggle. | Compiles and the app runs with it; the button itself unclicked. |
| Hidden notes sit out | `reveal_notes` brings hidden notes back before Arrange or Align reads positions, and a note still parked afterwards is reported rather than dropped. | Reasoned from the known −32000 behaviour; not exercised. |
| The Screen picker lagged | Placement emits `layout:settled` when the windows have actually moved; Settings listens and re-reads. The 900ms guess is gone. | Event is emitted on every placement; the panel's reaction unobserved. |
| Align's rules were invisible | The panel states them: Arrange fills even columns from the right; Align keeps each note where it is, one width for all, every column shared out evenly. | Text, visible on the panel. |
| Six marks read as a control panel | A divider splits them: Arrange/Align/Undo act on the arrangement, Save/New/delete act on the named layout. No feature was removed — the deeper collapse is a decision about what to drop, which is not mine to make. | Presentation only. |

**Still not verified, and worth one pass by hand**: the five minutes of
clicking that this session could not do — screen control was declined, and
driving the panel with synthetic keystrokes after that would not have been
right. In order: press Arrange, then Undo (the notes should go back exactly);
press Align twice (the second press should change nothing); hide the notes with
the shortcut and press Arrange (they should come back and all be arranged);
switch screens twice in a row from the dropdown; and the physical unplug test
from section 4, which remains the one that matters most.

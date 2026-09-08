# UX backlog

Candidate improvements, judged against what this product actually is: a small
panel that lives on screen permanently and is *glanced at*, not worked in.

Written 2026-09-08, after Phase 7. Ordered by value per hour, not by ambition.
Separate from [ui-parity.md](ui-parity.md), which is only about matching Google
Tasks — everything here is beyond parity.

---

## Tier 1 — fixes a real problem today

### 1. Undo for delete (~2h)

**Delete is instant and permanent.** No confirmation, no undo, and Google has no
recycle bin. One misclick on the `···` menu destroys a task and its notes.

Not solved by a confirmation dialog — a widget you use dozens of times a day
should not ask twice. Solve it by **deferring the API call**: hide the row
immediately, show a small "Task deleted — Undo" strip for ~6 seconds, and only
then send the DELETE. Undo inside the window is a local state restore with the
task's real id intact, not a recreate.

Also covers the failure case better than we do now: if the delayed delete fails,
the row comes back rather than silently vanishing from the UI while surviving in
Google.

### 2. Keyboard navigation of the list (~3h)

Today the keyboard reaches the add field and nothing else. For a panel that sits
in the corner of a coding session, being able to tick something off without
touching the mouse is most of the value.

- `↑` / `↓` move a selection
- `Space` toggles complete
- `Enter` edits the title
- `Delete` deletes (into the undo window above)
- `Esc` clears the selection

Costs a visible focus style, which the design currently lacks for rows.

### 3. Global show/hide hotkey (~2h)

The defining affordance of a widget, and the one thing SPEC deferred that is
genuinely missed: once hidden, the only route back is hunting for a tray icon
Windows files into an overflow flyout by default.

`Ctrl+Shift+G` as a default, rebindable in Settings, and it must fail loudly if
another app already owns the combination rather than silently doing nothing.

## Tier 2 — makes capture fast enough to trust

### 4. Multi-line paste creates multiple tasks (~1h)

Paste five lines into quick-add, get five tasks. People capture in bursts —
from a chat message, a meeting note, an email. Currently a paste produces one
task with newlines mangled into it.

Cheap, and it turns the widget into a genuine inbox.

### 5. Natural-language dates in quick-add (~4h)

`Order bearings tomorrow` → task "Order bearings", due tomorrow.
`Call Ravi friday` → due Friday.

High value for a capture-first widget, and Google's own web UI doesn't do it.

**The risk is false positives** eating words out of the title: "Review the
March proposal" must not lose "March". Mitigate by matching only a small
closed vocabulary (`today`, `tomorrow`, weekday names, `next week`, `in N days`)
and only as a **trailing** phrase, then showing the parsed date as a chip in the
input *before* commit so a wrong guess is visible and removable.

Do not attempt a general date parser. The failure mode of a clever one is
silently wrong data.

### 6. Clickable links in titles and notes (~1h)

People paste URLs into tasks constantly. Detect `https://` and render as a link
that opens in the system browser through the existing safe-open path.

Only `https`, never `file:` or anything else, and the link text stays exactly
what the user typed rather than a prettified version — a URL that displays
differently from where it goes is a phishing pattern, even in your own data.

## Tier 3 — character and polish

### 7. Overdue indicator on the tray icon (~2h)

When the widget is hidden, it may as well not exist. A small dot on the tray
icon when something is overdue gives it a reason to stay hidden — which is the
point of a tray app.

### 8. Roll-up / collapse to header (~2h)

Double-click the header to collapse the window to just its title bar, and again
to restore. Very sticky-note, very Windows, and it lets the widget shrink to
almost nothing without being dismissed.

### 9. Fade when unfocused (~1h)

Drop to ~85% opacity when the window loses focus. Keeps an always-on-top panel
from competing with whatever is being worked on. Must be a setting, and must not
apply while a menu is open.

### 10. Daily reminder for tasks due today (~3h)

A single Windows notification each morning listing what is due, rather than
per-task times. **Needs no API support** — the date is enough — so unlike the
per-task time in [ui-parity.md §1](ui-parity.md), this is buildable today.

## Considered and rejected

| Idea | Why not |
|---|---|
| Filters (All / Due today / Overdue) | Already in SPEC §2.2 as deferred, and still right: on a glanceable list of eight items, filters are chrome nobody touches. |
| Search | Same reason. Reconsider only if list sizes grow past a scroll. |
| Density / compact mode | One more setting to explain, for a panel the user already resizes freely. |
| Pin to desktop wallpaper | Considered at the very start and rejected: fragile `WorkerW` parenting that breaks on Explorer restart and multi-monitor changes. |
| Rich text in notes | The API stores plain text. Anything rendered would be a lie. |
| Per-task colours / labels | No API field. Would be local-only metadata invisible everywhere else. |

## Suggested order

**1 → 2 → 3** first: they fix a data-loss risk and close the two biggest
interaction gaps. Roughly a day.

**4 and 6** next: two hours together, and they change how the widget feels to
use more than their size suggests.

Everything else after the v1 gates in [BUILD_PLAN.md](BUILD_PLAN.md) — Phase 8's
security pass and Phase 9's docs matter more than any item here.

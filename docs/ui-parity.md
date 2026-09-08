# UI parity with Google Tasks

Element-by-element comparison against the Google Tasks web/side-panel UI,
recorded 2026-09-07. Field availability verified against the live Task resource
reference the same day.

The goal set by the project owner: **match everything Google Tasks offers.**
Four items cannot be matched, for reasons that are properties of the API rather
than choices — those are called out first so the plan below is honest.

---

## 1. Impossible through the API

Google's own clients are first-party and use interfaces the public Tasks API
does not expose. These four are not deferrals; they cannot be built.

| Google UI element | Why not |
|---|---|
| **Star / "Add to Starred"** | No `starred` field exists on the Task resource. The whole roster is `kind, id, etag, title, updated, selfLink, parent, position, notes, status, due, completed, deleted, hidden, links[], webViewLink, assignmentInfo`. |
| **Recurrence** (the ⟳ chip, "Daily") | No recurrence field of any kind. A repeating task arrives as an ordinary single task with no marker, so the widget cannot show or set one. |
| **"Set time" on the due date** | `due` accepts an RFC 3339 timestamp but the API explicitly "isn't possible to read or write the time". Offering a time picker would appear to work and silently discard the value. |
| **Add attachment** (Drive) | `links[]` is documented "Output only… this collection is read-only". |

**How to handle them rather than pretend:** the README lists them under Known
Limitations, and the per-task menu offers **Open in Google Tasks** using the
task's `webViewLink` — one click to the place where starring, recurrence and
attachments do work. That converts four dead ends into one honest escape hatch.

## 2. Achievable, ranked by value

| # | Google element | Have it? | API route | Notes |
|---|---|---|---|---|
| 1 | Notes / description under the title | ✗ | `notes`, writable, 8192 chars | Already fetched and cached; only the UI is missing. Cheapest real win. |
| 2 | Inline edit of the title | ✗ | `patch` `title`, 1024 chars | |
| 3 | Inline edit of notes | ✗ | `patch` `notes` | |
| 4 | Due-date chip with a calendar picker | Partial | `patch` `due` | Read-only display exists. Needs set + clear. **Date only.** |
| 5 | Clear due date (the ⊗ affordance) | ✗ | `patch` `due: null` | Falls out of #4. |
| 6 | Per-task ⋮ options menu | Partial | — | Right-click works today, but nothing signals it exists. A visible ⋮ on hover matches Google and is discoverable. |
| 7 | Move task to another list | ✗ | `tasks.move` + `destinationTasklist` | Confirmed present 2026-09-05. Recurrent tasks cannot be moved. |
| 8 | Drag to reorder | ✗ | `tasks.move` + `previous`/`parent` | `position` is output-only; reordering is a move call, not a field write. |
| 9 | Create a subtask / indent | Partial | `insert` with `parent` | Renders today, cannot be created. Needs the open question in ARCHITECTURE §11.2 answered. |
| 10 | "New list" | ✗ | `tasklists.insert` | |
| 11 | Rename / delete a list | ✗ | `tasklists.patch` / `.delete` | Google offers these in its list menu. |
| 12 | "Add a task" at the top | Partial | — | Ours is at the bottom. Google's is top. Cosmetic; ours suits a note better. |
| 13 | Completed section, collapsible | ✓ | — | Matches, count included. |
| 14 | Due-date chip styling | Partial | — | Google uses an outlined pill with a clock glyph; ours is plain right-aligned text. Ours is denser, which suits 340px. |
| 15 | Circular checkboxes | ✗ | — | Google uses circles; we use rounded squares. Squares read as Windows-native; this is a deliberate divergence, not an oversight. |
| 16 | Drag handle (⠿) on hover | ✗ | — | Only meaningful once #8 exists. |
| 17 | Overdue emphasis | ✓ (better) | — | Google shows the date in red; we do the same plus bold. |
| 18 | Task list switcher | ✓ | — | In the ⋮ menu rather than a top-level control. |

## 3. Where we already exceed Google Tasks

Worth keeping rather than sanding off in the name of parity:

- **Always-on-top, frameless, tray-resident.** Google has no desktop presence.
- **Offline read.** The panel paints from cache; Google's web UI shows nothing.
- **Sync status** — Google gives no indication of freshness at all.
- **Overdue emphasis** is stronger.
- **Keyboard**: `Ctrl+R`, `Ctrl+,`, `Ctrl+A`, `Esc`.

## 4. Proposed order

Grouped so each stage is independently shippable and each ends somewhere usable.

**Stage A — the task row becomes complete** (highest value per hour)
1. Notes displayed under the title, muted, truncated to two lines
2. Visible ⋮ per-task menu on hover, replacing hidden right-click
3. "Open in Google Tasks" via `webViewLink` — the escape hatch for §1
4. Inline title editing
5. Inline notes editing

**Stage B — dates**
6. Due-date chip restyled as a pill
7. Calendar picker to set a date, and clear it
   *No time field. A disabled one would be worse than none.*

**Stage C — organisation** ✅ done 2026-09-07
8. ✅ Move to another list — `tasks.move` with `destinationTasklist`
9. ✅ Create a new list
10. ✅ Rename / delete a list

Notes from building it:

- **Deleting the last list is refused**, in the backend rather than only the UI.
  Google keeps a default list for the same reason; a widget with no list to show
  has no recoverable state.
- **Deletion is confirmed inline**, naming the list and saying plainly that its
  tasks go too. Google offers no undo for this.
- **A failed move gets a specific message** — repeating tasks cannot change
  lists, and since recurrence is invisible to us we cannot pre-empt it, only
  explain it after the fact.
- **Move removes then re-inserts the cached row.** Rows carry their list id, so
  a plain upsert would leave a duplicate under the old list until the next full
  sync.

**Stage D — ordering** (most fiddly, least essential on a short list)
11. Drag to reorder via `tasks.move`
12. Create subtasks by indent — needs ARCHITECTURE §11.2 resolved first

## 5. Deliberate divergences

Not everything Google does suits a 340px always-on-top panel. These stay
different on purpose, and the reasoning belongs in the record:

- **Square checkboxes**, for Windows 11 consistency.
- **Add-task at the bottom**, where a sticky note's blank line lives.
- **Denser rows** — Google's side panel gives one task ~64px with its chips; at
  480px tall that is seven visible tasks. Ours fits about twelve.
- **Settings as an overlay**, not a second window.

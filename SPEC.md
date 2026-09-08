# Google Tasks Sticky Widget — Specification

A small, always-available Windows 11 desktop widget that shows your Google Tasks
in a sticky-note-style panel. Personal use, local-first, no backend server.

**Status:** scope defined, not yet implemented.
**Related:** [ARCHITECTURE.md](ARCHITECTURE.md) · [BUILD_PLAN.md](BUILD_PLAN.md)

---

## 1. Product intent

A glanceable note on the desktop, not a task manager. The success test is: you
look at it, you immediately know what's next, and you tick something off without
breaking focus. Anything that pulls you into managing tasks *inside the widget*
is out of scope — Google Tasks on web and mobile already does that well.

Design consequences:

- Small by default (~340×480), one column, no chrome beyond a thin header.
- Reads correctly at a glance from across the desk.
- Never blocks on the network. Renders from cache instantly, then updates.
- No modal dialogs in the main flow. Settings is the only secondary window.

## 2. Scope

### 2.1 In scope for v1

**Window behavior**

- Frameless window, custom drag region in the header.
- Always-on-top, toggleable from the header menu and settings.
- Resizable within sensible min/max bounds.
- Position, size, and always-on-top state persisted across restarts.
- System tray icon: show/hide, sync now, settings, quit.
- Close button hides to tray rather than exiting.
- Optional start with Windows, optional start hidden.

**Account**

- "Connect Google" via system browser, OAuth 2.0 Authorization Code + PKCE.
- Single scope: `https://www.googleapis.com/auth/tasks`.
- Tokens stored in Windows Credential Manager, never on disk in plaintext.
- Account status display, re-authenticate, and disconnect with token revocation.

**Tasks**

- Display tasks from one selected task list.
- Show title, completion state, and due date (date only — see §4.1).
- Show one level of subtask nesting, indented.
- Complete / uncomplete a task.
- Quick-add: inline input at the bottom, Enter to create, Escape to cancel.
- Delete a task via context menu.
- Manual refresh.
- Completed tasks struck through and collapsed into a "Completed (n)" section.

**Sync**

- Adaptive polling against the Tasks API using `updatedMin` delta queries.
- Optimistic UI on writes, revert on final failure.
- Exponential backoff on errors.
- Status indicator: Synced / Syncing / Offline / Error, click for detail.

**Appearance**

- Light, dark, and follow-system themes.
- Windows 11 visual language: rounded corners, subtle elevation, restrained motion.

### 2.2 Explicitly deferred to v2

Each of these is additive — none require reworking v1.

| Deferred | Why |
|---|---|
| Offline write queue + pending-operation log | Largest single chunk of complexity in the project, for a machine that is online ~99% of the time. v1 retries writes with backoff and reverts on failure. |
| Conflict detection and resolution UI | Only meaningful once offline writes exist. See ARCHITECTURE §6 for why it can never be fully airtight anyway. |
| Editing task titles and notes | Each needs its own editing affordance in a 340px window. The web app is one click away. |
| Setting and changing due dates | Read-only display in v1. Needs a date picker and an edit mode. |
| Moving tasks between lists | Supported by the API (§4.2), just not core to a glanceable widget. |
| Reordering / drag-to-sort | Requires `tasks.move` semantics, not a simple field write. |
| Multiple lists at once ("All Lists") | Contradicts the single-list glanceable model and multiplies sync state. |
| Filters (All / Incomplete / Due today / Overdue) | UI you will not use on a list of eight items. |
| Global keyboard shortcuts | Needs conflict handling with other apps and a rebinding UI. |
| Widget opacity control | Cosmetic. |
| MSI / NSIS installer, code signing | v1 runs from a locally built binary. |

### 2.3 Out of scope permanently

- Any backend server. The app talks to Google and nothing else.
- Analytics, telemetry, crash reporting, advertising.
- Scraping Google Tasks or using undocumented endpoints.
- Support for non-Google task providers.
- Distribution to other users — that would require Google OAuth verification review.

## 3. Functional requirements

### 3.1 First run

The widget opens to a connect prompt, not an empty list:

```
+--------------------------------+
|  My Google Tasks               |
|                                |
|  Keep your Google Tasks        |
|  visible on your desktop.      |
|                                |
|      [ Connect Google ]        |
|                                |
|  Only Google Tasks access is   |
|  requested. Not Gmail, Drive,  |
|  Contacts, or your password.   |
+--------------------------------+
```

After a successful connect: fetch task lists, prompt to pick a default list if
more than one exists, then show the widget.

### 3.2 Main view

```
+--------------------------------+
|  My Tasks              ●  ···  |
+--------------------------------+
|  [ ] Finish Urumi CAD          |
|  [ ] Buy filament         Fri  |
|      [ ] 1.75mm PLA black      |
|  [ ] Order bearings            |
|                                |
|  Completed (1)              v  |
+--------------------------------+
|  + Add task...                 |
+--------------------------------+
```

- `●` is the sync status dot. `···` opens the header menu.
- Header menu: Always on top, Task list, Sync now, Settings, Hide, Quit.
- Due dates render right-aligned and muted; overdue uses the danger accent.
- An empty list shows a quiet "Nothing to do" state, not a blank panel.

### 3.3 Task interactions

| Action | Trigger | Behavior |
|---|---|---|
| Complete | Click checkbox | Strikes through immediately, moves to Completed after a short delay, syncs |
| Uncomplete | Click checkbox in Completed | Returns to the main list, syncs |
| Add | Click "+ Add task...", type, Enter | Appears immediately, syncs |
| Cancel add | Escape | Closes input, discards text |
| Delete | Right-click → Delete | Removes immediately, syncs |
| Refresh | Header menu → Sync now | Forces a full, non-delta sync |

All writes are optimistic: the UI updates first, then the request goes out. If
the request ultimately fails after retries, the row reverts and an inline error
appears on that row.

### 3.4 Settings window

Compact, separate window, four sections:

- **General** — Start with Windows, Start hidden, Always on top
- **Google** — Connected account, task list selector, Sync now, Disconnect
- **Appearance** — Light / Dark / System
- **About** — Version, licenses, privacy statement, log file location

### 3.5 Keyboard

In-window only for v1; no global hotkeys.

- `Enter` — commit the add-task input
- `Escape` — cancel the add-task input
- `Ctrl+R` — sync now
- `Ctrl+,` — open settings

## 4. Constraints imposed by the Google Tasks API

These are properties of the API, not choices. They shape the product and must be
documented in the README so they don't read as bugs.

### 4.1 Due dates are date-only

The `due` field accepts an RFC3339 timestamp but the time portion is discarded
server-side. The mobile app's due *times* are not exposed through the API. The
UI must therefore never offer a time picker.

### 4.2 Moving between lists is supported

Verified 2026-09-05: `tasks.move` accepts `destinationTasklist`. Recurrent tasks
cannot be moved between lists. See [docs/api-findings.md](docs/api-findings.md) §1.

### 4.3 Ordering is not a writable field

`position` is output-only. Reordering goes through `tasks.move` with `previous`
and `parent` parameters, not a `patch`.

### 4.4 Subtask nesting

The reference documents no maximum depth, and `parent` is read-only on the
resource (set via `tasks.insert`'s `parent` parameter or `tasks.move`). The
widget renders a single indent level as a **design choice** for a narrow panel,
not because the API forbids more.

### 4.8 Completed tasks become hidden

`tasks.list` defaults to `showHidden=false`, and a task completed in Google's own
web or mobile client is marked hidden. Without `showHidden=true` such a task
disappears from results entirely — during a delta sync the widget would keep
showing it as incomplete forever, looking exactly like a sync bug. Every list
call must send `showHidden=true` and `showDeleted=true`.

### 4.9 Results are paginated

`maxResults` defaults to 20, maximum 100. Every list call must follow
`pageToken` to exhaustion or lists longer than 20 items silently truncate.

### 4.5 Recurrence is invisible

Repeating tasks appear as single instances with no recurrence metadata. The
widget can neither show nor create them.

### 4.6 No push notifications

The Tasks API has no `watch` / webhook channel. Polling is the only option. See
ARCHITECTURE §5 for the strategy.

### 4.7 No conditional writes

There are no ETags on tasks and no `If-Match` support, so a read-then-write
cannot be made atomic. See ARCHITECTURE §6.

## 5. Non-functional requirements

**Performance**

- Cold start to visible tasks, from cache: under 500 ms.
- Idle memory: under 150 MB.
- Idle CPU: effectively zero between sync ticks.

**Reliability**

- A network failure never produces an error dialog, only a status change.
- A failed write never silently discards the user's input.
- The app survives a token revocation from Google's account page by returning to
  the connect prompt with a clear message.

**Security** — see ARCHITECTURE §7. The hard rules:

- Refresh tokens live only in Windows Credential Manager.
- No token ever reaches the frontend, SQLite, a log file, or an external host.
- OAuth happens in the system browser, never an embedded WebView.
- PKCE with a high-entropy verifier; `state` validated on return.

**Privacy**

- Task content is transmitted only between the local machine and Google.
- No analytics in any version.

**Error messages** are written for a person, not a protocol:

> Bad: `Error 401` — Good: `Google access expired. Reconnect in Settings.`
>
> Bad: `Network exception` — Good: `Can't reach Google. Showing your last synced tasks.`

## 6. Acceptance criteria for v1

The build is done when all of the following pass by hand:

1. Fresh install → connect Google → tasks appear.
2. Add a task; it appears in Google Tasks on the web within one sync cycle.
3. Complete a task on the web; the widget reflects it within one sync cycle.
4. Restart the app; window position, size, theme, and selected list are restored.
5. Disconnect the network; the widget keeps showing cached tasks and reports Offline.
6. Reconnect; status returns to Synced without user action.
7. Attempt a write while offline; the row reverts and shows an inline error.
8. Revoke access from the Google account page; the app returns to the connect prompt.
9. Disconnect Google in settings; credentials are gone from Credential Manager.
10. Enable start-with-Windows; reboot; the widget launches, hidden if configured.
11. Grep the codebase and log files for token material; nothing found.

## 7. Known limitations to document in the README

- Due times are not supported (API limitation).
- Recurring tasks show as single instances (API limitation).
- Subtasks render one level deep (design choice for a narrow panel).
- Sync is polled, not instant; changes made elsewhere appear within ~30–60 s.
- The OAuth consent screen shows an "unverified app" warning on first connect,
  because the app is not submitted for Google verification. This is expected for
  a personal build.

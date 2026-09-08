# Cross-window task drag — Phase 0 report

> **Superseded in part, 2026-09-09.** The blocking finding below — "there is
> only one window" — is no longer true. `src-tauri/src/notes.rs` now ships the
> multi-window work this report priced at 3–4 days: `NoteRegistry` answers both
> questions §0.1 said nothing could, and `open_note` / `restore_session` /
> `register_note_list` are in place. Stage 0 is also done, not half-done:
> `tasks_commands::move_task` performs the `toIndex` → `previous` translation
> §0.4 called out as missing. Stage 1 (in-list reorder) landed in f601190.
>
> Still accurate and still worth heeding: §0.2 on the JS side having no shared
> store (so `dragSession` must move into Rust for Stage 3), §0.4 on every reorder
> being a fallible network round-trip with no offline queue, and §0.5 on Smart App
> Control blocking unsigned build scripts. The recommendation to drop the 600ms
> raise-on-hover stands.

Investigation requested by the implementation brief, 2026-09-08. No feature code
written, per the brief's working agreement.

**Verdict: do not start Phase 1.** The design is sound, but it rests on four
assumptions about this codebase. One is false outright, two are half-true in ways
that change the staging, and one removes a stage's justification.

---

## 0.1 Window and process model

**Framework:** Tauri 2.11 + React 19 + TypeScript. `src-tauri/tauri.conf.json`.

**Number of note windows: one.**

```
tauri.conf.json:13   "windows": [
tauri.conf.json:15       "label": "main",
```

`src-tauri/src/window.rs:84` hard-codes `app.get_webview_window("main")` as the
only window lookup in the codebase. There is no window manager, no registry of
open notes, and no code path that creates a second window.

**This is the finding that blocks the brief.** Its opening line — "Several note
windows are open on the desktop" — describes a product that does not exist yet.
Everything downstream (hit-testing note windows, a note-window registry, drag
between two lists on screen) presumes a multi-window app.

Multi-window is listed in [action-plan.md](action-plan.md) as an open question
with a 3–4 day estimate, deliberately not folded into any wave. It is a
prerequisite for this brief, not a detail of it.

**Process model, for when that changes:** Tauri runs one Rust process hosting all
windows. WebView2 spawns its own OS processes per WebView, but the Rust core —
where the SQLite cache and managed state live — is shared. So the brief's "no
process boundary to serialize across" holds **on the Rust side**.

## 0.2 State and store

**Half-true, and the half that is false matters.**

The brief says: *"All note windows are views over one store in one process."*

- **Rust side — true.** `Store` (`src-tauri/src/store/mod.rs`) wraps one SQLite
  connection behind a `Mutex`, managed by Tauri and reachable from any window.
- **JS side — false.** Each WebView is an isolated JavaScript context. Today all
  task state is React `useState` inside one hook:

  ```
  src/hooks/useTasks.ts:22   const [tasks, setTasks] = useState<Task[]>([]);
  src/hooks/useTasks.ts:23   const [taskLists, setTaskLists] = useState<TaskList[]>([]);
  src/hooks/useTasks.ts:24   const [selectedListId, setSelectedListId] = useState(...);
  ```

  There is no shared JS store, and no mechanism by which a `setState` in one
  window reaches another. Adding a second window today would give two React trees
  that know nothing about each other.

**How a mutation in window A would reach window B today:** through Rust. A
command writes to SQLite, then emits an event (`tasks:updated`,
`sync:status`, `window:layer`) which every window's listener picks up and
re-reads the cache from. That machinery already exists and works —
`src/hooks/useTasks.ts` subscribes to `tasks:updated` — it has simply never had a
second subscriber.

**Adaptation required:** `dragSession` cannot be "a record in the shared store"
as written, because the shared store the brief means (a JS one) does not exist.
It must live in **Rust managed state**, mutated by commands and broadcast as an
event. That is more plumbing than the brief assumes, but it is the same shape as
`SyncManager` (`src-tauri/src/sync/mod.rs`), which already does exactly this:
`Arc<Mutex<Shared>>` plus `app.emit(...)`.

## 0.3 Existing drag — there is none

```
$ grep -rln "dnd|draggable|onDragStart|dragover|SortableJS" src/
(no matches)
```

**Stage 1 is not a refactor.** The brief frames it as "no new user-visible
behavior… if in-list reorder behaves differently afterward, that is a bug". There
is nothing to preserve — in-list reorder does not exist. Stage 1 becomes
build-from-scratch.

That is not fatal, and the ordering advice still holds: building in-list drag on
a `dragSession` shape first is exactly right, and it is already on the roadmap as
Wave 3 item 12. But the working agreement's "Stage 1 is a pure refactor" clause
does not apply, and the stage is larger than the brief implies.

## 0.4 Ordering is not ours to control

**This is the deepest mismatch, and the brief cannot know it.**

The brief asks for `moveTask(taskId, toListId, toIndex)` as "a single store
mutation". Ordering here is not local state:

```
src-tauri/src/google/models.rs:66
  /// Only the fields we ever write. `position` is output-only and `parent` is
  /// set through `tasks.move`, so neither belongs here.
```

`position` is a server-assigned opaque string. There is no integer index, no
fractional rank, and no writable ordering field. Reordering is a **`tasks.move`
API call** taking a `previous` task id — not an index.

Consequences:

- `toIndex` must be translated to "the id of the task that should precede this
  one" at commit time.
- **Every drop is a network round-trip that can fail.** The brief's payoff —
  "the commit is the same `moveTask` mutation used by in-list reorder" — holds
  structurally, but that mutation is not local and not instant.
- v1 has no offline write queue ([SPEC.md](../SPEC.md) §2.2), so a reorder
  attempted offline fails and reverts. Acceptable, but it must be designed for
  rather than discovered.

## 0.5 Platform access

All available; nothing blocks the mechanism.

| Need | Available |
|---|---|
| Win32 (`WindowFromPoint`, capture) | Yes — `windows` crate already in the tree transitively; `winreg` used directly in `src-tauri/src/autostart.rs` |
| Transparent, frameless window | Yes — already in use: `tauri.conf.json` sets `"transparent": true`, `"decorations": false` |
| Always-on-top | Yes — `set_always_on_top`, shipped in Wave 1 |
| Click-through | Yes — Tauri exposes `set_ignore_cursor_events(true)` |
| Non-focusable | Yes — window builder `focused(false)` |
| Per-monitor DPI v2 | Yes — tao's default on Windows |

**One platform caveat worth flagging early:** Smart App Control is **enforcing**
on this machine and has already blocked one crate's unsigned build script
(`tauri-plugin-autostart`, see [BUILD_PLAN.md](../BUILD_PLAN.md) Phase 7). Any
new native dependency this work needs may simply refuse to build, at random,
based on binary reputation. Budget for working around it rather than assuming a
crate will install.

## 0.6 What I think of the brief

**The design is good.** The three-part mechanism — Rust-held drag state, a
click-through ghost window, `WindowFromPoint` hit-testing with a throttled IPC
index query — is the right answer for this stack, and better than OLE drag for
the stated scope. Two calls I would keep exactly as written:

- **Pointer capture means the target gets no mouse events**, so all target
  feedback must come from pushed state. That is the non-obvious insight the whole
  design turns on, and it is correct.
- **Narrow scope to note→note.** `WindowFromPoint` returning "not one of ours"
  makes desktop, other apps, and our own non-note windows fall out for free.

**What I would change:**

1. **Stage 0 is already half-built.** Cross-list move shipped in parity Stage C:
   `move_task_to_list` (`src-tauri/src/google/client.rs:221`) plus a "Move to
   list" submenu on every task. What is missing is the `toIndex` → `previous`
   translation and same-list reorder.
2. **Add a Stage −1: multi-window.** Without it there is nothing to drag between.
   This is the real cost of the feature and it should be priced honestly:
   per-window state, per-window sync targets, a window registry, per-window
   colour/position persistence. 3–4 days before Stage 0 begins.
3. **Move `dragSession` into Rust**, following `SyncManager`'s existing pattern,
   rather than a JS store.
4. **Drop the 600ms raise-on-hover.** The brief already marks it optional. With a
   window that can sit *behind* other windows (shipped in Wave 1), auto-raising
   during a drag will fight that setting.

**Total cost, honestly:** ~4 days of prerequisite plus ~3 days of the brief.
Roughly a week, for a convenience path over the "Move to list" menu that already
works — and which the brief itself relies on for minimized notes.

## Recommendation

Not now. Suggested order:

1. **v1 gates** — offline behaviour has never been tested despite three
   acceptance criteria covering it; the security pass has not run; the project is
   still not under version control. ~1.5 days.
2. **In-list drag reorder** (Wave 3 item 12). Needed regardless, and it is this
   brief's Stage 1 done properly.
3. **Decide multi-window as a product question**, not a feature ticket. If the
   answer is no, this brief is moot; if yes, it is worth doing well.
4. **Then this brief**, adapted per the four points above.

Awaiting a decision before any Phase 1 work, per the brief's working agreement.

# Action plan

One ordered plan, merging the owner's feature requests (2026-09-08) with the
candidates in [ux-backlog.md](ux-backlog.md) and the outstanding stages in
[ui-parity.md](ui-parity.md).

Supersedes the tiering in `ux-backlog.md`. [BUILD_PLAN.md](BUILD_PLAN.md) still
governs the v1 gates, which run as a parallel track (see the end).

---

## Assessment of the requested features

Everything requested, with an honest verdict before any of it is scheduled.

| Requested | Verdict | Note |
|---|---|---|
| Pin icon: always-on-top **and** go beneath other apps | ✅ Build | Tauri exposes `set_always_on_bottom`. Becomes a three-state pin rather than a toggle. |
| Double-click the heading to rename the list | ✅ Build | Better than the menu route we have. **Forces a choice** — see "Conflicts". |
| Colour picker, presets + custom | ✅ Build, with care | Local-only; no API field for it. Contrast must be derived, not chosen. |
| Add task at the top, like Google | ✅ Build | Reverses a deliberate design choice. Owner's call, and easily made a setting. |
| Drag-and-drop reorder | ✅ Build | Already planned as parity Stage D. |
| Drag a task to another list "opened nearby" | ⚠️ Reshape | Implies two widget windows. See "Open question". |
| Windows taskbar button for reopen/minimise | ✅ Build | Contradicts the widget default, so it becomes a setting. Solves "can't find it" better than the tray does. |
| Icon to open in Google Tasks | ✅ Already built | In the `···` menu. Promote to a visible row icon. |
| Repeat, tracked locally | ✅ Build, v2 | Feasible. Real duplication risk — see the design note. |
| Task editing | ✅ Already built | Inline title and notes editing shipped in Stage A. |
| Keyboard shortcut to show/hide the widget | ✅ Build | The most-missed thing in the product today. |
| Multi-select / drag-select | ✅ Build | Pairs with keyboard navigation; do them together. |

### Conflicts to settle

**Double-click the header** was also the natural gesture for roll-up (collapse
to title bar, `ux-backlog.md` §8). Both cannot own it. **Rename wins** — it is
requested, and roll-up duplicates what hiding to tray already does. Roll-up is
dropped from the plan.

**Add-task at top** reverses the reasoning in `ui-parity.md` §5, which put it at
the bottom "where a sticky note's blank line lives". The owner has asked for
Google's placement; that settles it. Shipped as a setting so either is available,
defaulting to top.

### Open question — one note, or many?

"Drag the task to another list opened nearby" implies **two widget windows side
by side**, which is a different product shape: one sticky note per list, each
with its own position, size and colour.

That is a genuinely appealing idea — it is what physical sticky notes do, and it
makes the colour picker far more useful — but it is an architecture change, not a
feature. Per-window state, per-window sync targets, a window manager, and
cross-window drag-and-drop between two WebViews (which is the hard part; it needs
a native drag payload, not HTML5 DnD).

**Estimate: 3–4 days on its own.** Not folded into any wave below.

Until it is decided, moving between lists is served by:
- the existing `···` → **Move to list**, and
- **drag a task onto the list name in the header** (Wave 3), which is the same
  gesture without the second window.

---

## Wave 1 — Safety and control (~1.5 days)

Nothing here is cosmetic. One is a live data-loss risk.

1. **Undo for delete** (~2h) — deletion is currently instant, permanent, and
   Google has no recycle bin. Defer the API call ~6s behind an "Undo" strip; the
   row is restored locally with its real id intact.
2. **Three-state pin control** (~2h) — on top / normal / beneath other windows,
   replacing the current on/off. Persisted; tray checkbox follows.
3. **Global show/hide hotkey** (~3h) — default `Ctrl+Shift+G`, rebindable, and
   it must report failure loudly if another app already owns the combination
   rather than silently doing nothing.
4. **"Show in taskbar" setting** (~1h) — flips `skipTaskbar`. For anyone who
   would rather Alt-Tab to it than hunt the tray.

## Wave 2 — Editing and capture (~1.5 days)

5. **Double-click the header to rename the list** (~2h) — must not fight the
   drag region; a double-click that also nudges the window is worse than no
   feature.
6. **Move quick-add to the top**, with a setting for the old position (~2h).
7. **Multi-line paste creates multiple tasks** (~1h) — paste five lines, get
   five tasks. Turns the widget into an inbox.
8. **Clickable `https` links** in titles and notes (~1h) — opened through the
   existing safe-open path; link text stays exactly as typed.
9. **Open-in-Google icon on the row** (~1h) — currently menu-only.

## Wave 3 — Selection and ordering (~2.5 days)

10. **Keyboard navigation** (~3h) — `↑↓` select, `Space` complete, `Enter` edit,
    `Delete` remove, `Esc` clear. Needs a row focus style.
11. **Multi-select** (~4h) — `Shift`-click for ranges, `Ctrl`-click for
    individuals, drag-select over rows. Bulk complete / delete / move.
12. **Drag to reorder** (~4h) — `tasks.move` with `previous`/`parent`.
    `position` is output-only, so this is a move call, not a field write.
13. **Drag a task onto the header list name to move it** (~2h) — the
    single-window answer to the cross-window request.
14. **Create subtasks by indent** (~2h) — blocked until the open question in
    [ARCHITECTURE.md](../ARCHITECTURE.md) §11.2 is probed: whether
    `tasks.insert` honours its `parent` parameter. That probe is 15 minutes.

## Wave 4 — Appearance (~1 day)

15. **Colour picker** (~5h) — a row of presets plus a full picker. Stored
    per-list, locally; there is no API field, so this never leaves the machine.
    **Foreground colour must be derived from the chosen background**, not picked
    separately — a user-chosen pair will eventually be unreadable, and a sticky
    note nobody can read is worse than a white one.
16. **Fade when unfocused** (~1h) — ~85% opacity, off by default, suppressed
    while any menu is open.

## Wave 5 — Time and repeat (~3 days)

Everything here is **local invention**, not Google parity: the API exposes no
recurrence field and discards the time component of a due date. Both were
verified against a live account — see [api-findings.md](api-findings.md) §2 and
[ui-parity.md](ui-parity.md) §1.

17. **Daily "due today" notification** (~3h) — needs no API support at all,
    since the date is enough. Build this first; it may be most of what the
    repeat request is really after.
18. **Local repeat rules** (~8h) — daily / weekdays / weekly / custom pattern.
    On completing a marked task, create the next instance in Google with the
    next due date.

    **The duplication risk is real and must be designed around**: if the same
    task also repeats in Google, we cannot see that — there is no field to read —
    and both would fire. Mitigation: **act only on tasks carrying our own
    marker.** A task you set to repeat in Google has no marker, so we leave it
    alone. The marker is the ownership signal.

    The marker rides in `notes` rather than local storage, so it survives a
    cache wipe and is visible on every device. It shows as a line of text in
    Google's UI — that is the honest cost of the workaround.
19. **Local reminder time per task** (~4h) — same marker mechanism, driving a
    Windows notification. Invisible to Google's own clients by construction.

---

## Parallel track — v1 gates

These are not features and they do not compete with the waves; they are what
makes the thing shippable. From [BUILD_PLAN.md](BUILD_PLAN.md), 13 items
outstanding:

- **Offline behaviour, actually tested** (~30m) — three SPEC §6 acceptance
  criteria cover it and none have ever been exercised. The single biggest
  unknown in the project.
- **Sleep-resume and network-reconnect sync triggers** (~1h).
- **Security pass** (~2h) — the ARCHITECTURE §7.5 grep checklist, capability
  audit, IPC audit.
- **README, setup doc, known limitations** (~2h).
- **Release build run from a clean directory** (~1h).

**Not under version control.** The project is not a git repository; every file
lives only in OneDrive. Worth `git init` before more accumulates — and Phase 8's
"audit git history for credentials" cannot happen until it exists.

## Recommended order

**Wave 1, then the offline test and security pass, then Wave 2.**

Wave 1 removes a way to lose data and closes the two worst control gaps. The
gates after it are short and protect everything built so far. Waves 3–5 are then
free to run in any order the owner prefers.

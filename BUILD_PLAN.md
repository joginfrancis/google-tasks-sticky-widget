# Build Plan — v1

Sequenced plan to a working widget. Read [SPEC.md](SPEC.md) for scope and
[ARCHITECTURE.md](ARCHITECTURE.md) for design.

Estimated total: **~6 working days.** Phase 4 is the one most likely to overrun —
loopback callback handling and Credential Manager are both fiddlier than they look.

---

## Phase 0 — Verify before building — ✅ DONE 2026-09-05

Findings recorded in [docs/api-findings.md](docs/api-findings.md).

- [x] Current Google docs read for desktop OAuth, PKCE, and the Tasks API.
- [x] `due` confirmed date-only.
- [x] `tasks.move` **does** accept `destinationTasklist`.
- [x] Quota confirmed: 50,000 queries/day, no documented per-minute limit.
- [x] `keyring` 4.2.0; Windows store is an optional dependency.

Two findings changed the plan and would otherwise have shipped as convincing bugs:

- **`showHidden=true` is mandatory.** Tasks completed in Google's own clients are
  hidden and vanish from list results; a delta sync would show them as incomplete
  forever.
- **Pagination is mandatory.** `maxResults` defaults to 20; lists would truncate.

Both are now hard requirements in ARCHITECTURE §5.2.

Still open, deliberately deferred to the phase that can answer them empirically:
whether a Desktop client can omit `client_secret` (Phase 4), and whether
`tasks.insert` can create a subtask via `parent` (Phase 5).

## Phase 1 — Google Cloud setup — ✅ DONE 2026-09-05

- [x] Cloud project created, **Google Tasks API** enabled.
- [x] Google Auth Platform configured (Branding: app name + support email +
      developer contact; App domain fields left blank deliberately, since any
      URL there forces an Authorized Domain you must own).
- [x] Scope `https://www.googleapis.com/auth/tasks` added, nothing else.
- [x] **User type set to Internal** (Workspace org `startupmission.in`).
- [x] Desktop app OAuth client created; JSON downloaded to
      `C:\Users\ADMIN\.gtasks-widget\` — outside the repo and outside OneDrive.

**Deviation from plan, for the better.** The plan called for External +
Publish-to-Production to dodge the 7-day refresh token expiry. Internal is
cleaner: no verification, no unverified-app warning, and the 7-day rule doesn't
apply at all (it is scoped to External + Testing).

Consequences accepted:

- Only `@startupmission.in` accounts can authorize; the widget syncs that
  account's tasks, not a personal Gmail account.
- The app is tied to the org — leaving it, or an admin restricting API access,
  breaks the widget.

**Gate:** deferred to Phase 4, where the flow is exercised end to end.

## Phase 2 — Window shell — ✅ DONE 2026-09-05

Nothing about Google. Just get a widget-shaped window behaving correctly.

- [x] `create-tauri-app` with React + TypeScript + Vite.
- [x] Frameless, transparent-capable, no taskbar entry, min/max size bounds.
- [x] Custom drag region on the header; verify dragging doesn't swallow clicks
      on header controls.
- [x] Always-on-top toggle.
- [x] Persist position, size, and always-on-top to a settings file; restore on
      launch. Handle the monitor-disconnected case — clamp to the nearest
      visible monitor rather than opening off-screen.
- [x] Tray icon with show/hide/quit; close button hides rather than exits.

**Gate:** drag it to a corner, quit, relaunch, it reappears in the same place.

## Phase 3 — UI on mock data — ✅ DONE 2026-09-05

The highest-leverage phase. Build the whole interface against a hardcoded array
so design decisions happen before any network code can block them.

- [x] Design tokens for light and dark; system-theme subscription.
- [x] Task row: checkbox, title, due badge, overdue styling, one level of
      subtask indent.
- [x] Completion animation — strike through, brief pause, move to Completed.
- [x] Collapsible "Completed (n)" section.
- [x] Inline quick-add with Enter/Escape.
- [x] Context menu with Delete.
- [x] Header menu, sync status dot with its four states.
- [x] Empty state, loading state, connect prompt (SPEC §3.1).
- [x] Settings window, all four sections, wired to nothing.

**Gate:** it looks and feels like the finished product. Screenshot it. If it
isn't glanceable at this stage, no amount of sync work will fix that later.

## Phase 4 — Auth — ✅ DONE 2026-09-07

- [x] PKCE verifier and `state` from the OS CSPRNG, with unit tests.
- [x] Loopback listener on `127.0.0.1:0`, ephemeral port into the redirect URI.
- [x] System browser via the opener plugin; 5-minute timeout.
- [x] Constant-time `state` validation, with a prefix-attack test.
- [x] Token exchange; refresh token in Credential Manager, access token in
      memory only. Real round-trip test passes.
- [x] Proactive refresh margin of 5 minutes before expiry.
- [x] Disconnect: revoke, then delete — deletion happens even when revocation fails.
- [x] `get_account_status` returning connected + authorizing, nothing more.
- [x] Redaction: no authorization URL, token, or verifier reaches a log.
- [x] Third-party crates (`keyring`, `keyring_core`, `reqwest`) capped at INFO
      so no unvetted dependency DEBUG output can land in a log file.

**Client secret question closed:** required. See ARCHITECTURE §4.2.

**Deviation:** `get_account_status` reports connected/not, with no email address.
Showing the account name needs the `userinfo.email` scope, and SPEC §2.1 commits
to `auth/tasks` alone — a second scope on the consent screen is a real cost for
a label. Revisit only if the account becomes ambiguous.

**Gate:** connect succeeded end to end, no unverified-app warning (Internal app).
Restart-persistence and revoke-from-Google still to be exercised in Phase 6,
where a live token is needed anyway.

## Phase 5 — Data layer and API client — ✅ DONE 2026-09-07

- [x] SQLite schema and versioned migrations, WAL mode.
- [x] `TasksClient`: list task lists, list tasks, insert, patch, delete.
- [x] `showHidden=true`, `showDeleted=true`, and `pageToken` looping enforced
      inside the client rather than left to callers.
- [x] Typed error mapping; a quota 403 and a permission 403 are distinguished by
      the body's `reason`, since the status alone cannot tell them apart.
- [x] Cache read path feeding the UI; mock data deleted.

**Verified against a real account:** 18 tasks synced, cache order matched
Google's exactly, due dates stored correctly, and 14 tasks completed in Google's
own clients came back — which is the `showHidden` finding proving itself.

**Two bugs found and fixed during this phase:**

1. *First-run deadlock.* The cache starts empty and sync is keyed on a selected
   list, so nothing would ever fetch the task lists — a permanently blank widget
   on a fresh install. Added `refresh_task_lists`.
2. *Discarded first cache read.* `currentListRef` was assigned during render, so
   a `setState` followed by an immediate cache read still saw the old value and
   threw the result away — defeating paint-instantly-from-cache on launch. The
   ref is now written by the single function that changes the list.

**Diagnostic added:** `cargo run --example dump_cache` prints what the cache
actually holds. It is what separated "we never fetched it" from "we stored it
wrong" when the widget and Google appeared to disagree.

**Gate:** real tasks render. Offline-relaunch still to be exercised.

## Phase 6 — SyncManager — ✅ MOSTLY DONE 2026-09-07

- [x] One background thread owns polling; everything else pokes it through a
      channel, so there is a single place that decides when to call Google.
- [x] Three-tier cadence: 30s active, 60s idle, 5min hidden.
- [x] Refresh on window focus and on show-from-tray.
- [x] Delta sync with the `updatedMin` cursor and 30s clock-skew slack.
- [x] Exponential backoff, deterministic ±12.5% jitter, 15-minute ceiling,
      `Retry-After` honoured, reset on success.
- [x] Optimistic writes with revert-on-failure; a failed quick-add hands its
      text back rather than losing it.
- [x] `sync:status` events driving the status dot.
- [x] Human-readable error strings throughout.

**Measured, not assumed:** 31s gaps while active, then a 61s gap once the
two-minute activity window lapsed.

**Two design flaws caught by watching the live log:**

1. *Blur counted as activity*, so clicking any other app refreshed the timer and
   pinned the widget at the fast cadence forever. Losing focus now does nothing.
2. *Double sync on every launch and list switch* — the frontend fired a full
   sync while pointing the scheduler at the list, which itself syncs
   immediately. With no cursor yet that first delta already is a full fetch, so
   the frontend's copy was pure duplication. Removed.

**Still outstanding for this phase:**

- [ ] Sleep-resume and network-reconnect triggers (the timer covers both within
      one interval today, just less promptly).
- [ ] Full sync after a cursor-invalidating error.
- [ ] Offline behaviour verified by actually pulling the network.

## Phase 7 — Windows integration — ✅ DONE 2026-09-08

- [x] Start with Windows, via the per-user `HKCU\...\Run` key.
- [x] Start hidden, honoured from both the stored preference and the `--hidden`
      flag the Run entry passes.
- [x] Task list selector wired to settings and persisted (done in Phase 5).
- [x] In-window keyboard shortcuts (done in Phase 3).
- [x] Tray menu: Show/hide, Always on top, **Sync now**, **Settings…**, Quit.

**Blocker hit, and worked around rather than disabled.** `tauri-plugin-autostart`
ships a build script, and **Smart App Control is enforcing on this machine**, so
Windows refused to run the unsigned build-script executable:

```
Code Integrity ... build-script-build.exe did not meet the
Enterprise signing level requirements (os error 4551)
```

Turning Smart App Control off is irreversible without reinstalling Windows —
wildly disproportionate for one registry value. The plugin was dropped and
[src-tauri/src/autostart.rs](src-tauri/src/autostart.rs) writes the Run key
directly via `winreg`: no build script, no new binary, less code than the plugin
needed. Worth remembering for any future dependency — a crate with a build
script may simply not build here.

> Smart App Control was subsequently turned off anyway (Phase 9), so the
> constraint that forced this no longer applies. The decision stands regardless:
> the direct `winreg` write is less code than the plugin it replaced and adds no
> build script. Keeping it.

Two details that matter more than they look:

- **The registry is the source of truth**, not a cached copy. The user can
  delete the entry from Task Manager's Startup tab, and a cached value would
  then quietly lie.
- **The stored path is refreshed at startup** if the executable has moved. A
  stale entry fails silently at sign-in, which looks exactly like the setting
  not working.

> **Found in Phase 2:** Windows 11 files every new tray icon into the hidden
> overflow flyout by default, so the icon is invisible until the user clicks the
> `^` chevron or enables it under Taskbar settings. Because the tray is the only
> way back to a hidden window (`skipTaskbar` is on), a user who hides the widget
> before discovering the icon has no route back. Either the first-run flow says
> where to find the icon, or the installer pins it. Not optional polish.


## Phase 8 — Security pass and tests (half day)

- [ ] Run the ARCHITECTURE §7.5 grep checklist.
- [ ] Audit every Tauri capability; delete anything unused.
- [ ] Audit every IPC command against ARCHITECTURE §3's rules.
- [ ] Check `.gitignore` and git history for credential material.
- [ ] Automated tests where they earn their place: PKCE generation, `state`
      validation, token store round-trip and delete, backoff arithmetic, delta
      cursor handling, malformed API response handling.
- [ ] Walk the full SPEC §6 acceptance list by hand.

**Do not** write automated tests for window position persistence or tray
behavior — manual verification is faster and more honest there.

## Phase 9 — Docs and ship (half day)

> **Both build blockers resolved 2026-09-11.** Kept in full because each cost
> days, and the second is easy to re-create by accident.
>
> ---
>
> ### Blocker 1 — Smart App Control (resolved: turned off)
>
> Bundling compiles fresh build-script executables, and SAC (enforcing here)
> refuses to run binaries it has no reputation for. Reproduced three times on
> 2026-09-08:
>
> | Attempt | Blocked at |
> |---|---|
> | `tauri build` | `vswhom-sys` build script |
> | `tauri build` again | same file — deterministic, not the reputation lottery |
> | `tauri build --debug` | `tauri-plugin-window-state`, which had built fine an hour before |
>
> `tauri dev` still works, because its build scripts were compiled before SAC
> started refusing them. That is luck, not a guarantee.
>
> **Only three real fixes**, and none of them are code:
>
> 1. **Turn SAC off** — the only route to an installer built here. Irreversible
>    without reinstalling Windows, so it is the owner's decision to make and to
>    execute, not something to work around in the build.
> 2. **Build in CI** — clean, but an unsigned installer would likely be blocked
>    when *run* here too, so it only helps once SAC is off anyway.
> 3. **Code-sign** — what SAC is actually asking for. Right answer if the app
>    ever leaves this machine; overkill for a personal tool.
>
> Do not attempt to engineer around this in the build scripts. It is a security
> control the user deliberately has on.
>
> **2026-09-11 — it now blocks compilation entirely.** The block moved from
> build scripts and output binaries to the **proc-macro DLLs `rustc` loads**:
>
> ```
> cssparser_macros-...dll: LoadLibraryExW failed:
> An Application Control policy has blocked this file. (os error 4551) (retried 5 times)
> ```
>
> Every previous workaround is therefore dead. Skipping the bundler, plain
> `cargo build`, and `tauri dev` all fail the same way, because none of them
> avoids loading a proc-macro. The last binary built before this
> (`target/release/sticky-widget.exe`, 2026-09-10) still runs when launched from
> Explorer, so the app on screen is usable — it just cannot be rebuilt.
>
> While this held, `cargo clean` was unsurvivable: cached artifacts were the
> only reason anything built at all. Learned the expensive way — a
> `cargo clean -p tauri …`, meant to fix a corrupted fingerprint from an
> interrupted build, removed 236 MB that could not be rebuilt.
>
> **Resolved** by turning Smart App Control off (2026-09-11). One-way without
> reinstalling Windows, so it was the owner's decision and the owner's to
> execute. Code-signing remains the right answer if the app ever leaves this
> machine.
>
> ---
>
> ### Blocker 2 — Controlled Folder Access (resolved: target dir moved out)
>
> Turning SAC off uncovered a second, unrelated block underneath it. The
> project lives under `OneDrive\Documents`, which Windows protects by default
> and which **cannot be removed from the protected folder list**. Every crate's
> build script compiles to its own executable inside `target\` and then writes
> its results there — so each one is an unrecognised process writing to a
> protected path, and Defender blocks it.
>
> **The failure mode is deceptive and cost most of the diagnosis time.** CFA
> does not report access denied. It reports the file as missing:
>
> ```
> Failed to create ...\build\proc-macro2-13ee5db823cf6b2d\out\probe:
> The system cannot find the file specified. (os error 2)
> ```
>
> which reads as a corrupted target directory and invites a `cargo clean` that
> makes things worse. The only reliable way to identify it is Defender's own
> log — **event ID 1123** (1124 when auditing):
>
> ```powershell
> Get-WinEvent -LogName "Microsoft-Windows-Windows Defender/Operational" |
>   Where-Object Id -in 1123,1124
> ```
>
> **Per-file exemptions cannot solve this.** There are ~200 build scripts, their
> paths carry content hashes, and the hashes change. Exempting `cargo.exe`,
> `rustc.exe` and `node.exe` does nothing, because none of those is the process
> doing the write.
>
> **Fix:** `src-tauri/.cargo/config.toml` points `target-dir` at
> `C:/cargo-target/sticky-note`. Build scripts then live and write outside the
> protected folder; the source stays where it is and Documents keeps full
> protection with no folder-level holes. This also stopped OneDrive from syncing
> the target directory — it had accumulated **22.8 GB across 19,819 files**,
> re-uploaded on every compile.
>
> `git.exe` needed a separate exemption (all three shims: `mingw64\bin`, `cmd`,
> `bin`). Without it `.git\` is read-only and commits fail with the same
> misleading `No such file or directory`. `sh.exe` was deliberately *not*
> exempted — it is a general-purpose shell and the hole would be far wider than
> git needs. The cost is that git hooks stay blocked; this repo has none.
>
> **One residual gap.** Tauri's own build script writes `src-tauri/gen/schemas`
> *inside* the project, so that write is still blocked — and **the build still
> exits 0**. Schemas are correct as of 2026-09-11 (verified by timestamp against
> `capabilities/default.json`), but after any capability change, check that
> `gen/schemas/capabilities.json` is newer than the capability file. If it is
> not, the build has silently kept a stale permission schema.

- [ ] `README.md`: what it is, screenshot, install, and an explicit statement
      that it uses OAuth and never sees the Google password.
- [ ] `docs/google-cloud-setup.md`: reproducible Phase 1, including why the
      consent screen must be published.
- [ ] Known limitations from SPEC §7, so they don't read as bugs.
- [ ] Release build; run it from a clean directory to catch anything that only
      worked in dev.

---

## Sequencing rules

**Mock data before network.** Phase 3 must be visually finished before Phase 4
starts. Otherwise OAuth debugging blocks all design work, and the project stalls
in the least motivating place possible.

**Don't chase v2 features mid-build.** Every deferred item in SPEC §2.2 is
additive by construction. If one starts feeling essential during Phase 6, write
it on a list and keep going.

**Ship at Phase 9 and use it for two weeks** before deciding what v2 contains.
The prediction worth testing: you will miss due-date editing and nothing else on
the deferred list.

## Risk register

| Risk | Likelihood | Mitigation |
|---|---|---|
| Loopback OAuth callback fights Windows firewall or Tauri's runtime | Medium | Isolate in a standalone Rust binary first, before integrating |
| `keyring` behaves unexpectedly on Windows | Low | Verified in Phase 0 |
| Consent screen left in Testing → weekly reconnects | Medium | Phase 1 gate; confirmed real in Phase 0; the single most common way this project gets abandoned |
| `showHidden` omitted → completed tasks reappear as incomplete | ~~High~~ Closed | Found in Phase 0; now a Phase 5 gate |
| Frameless drag region swallows header clicks | Medium | Phase 2 gate explicitly checks it |
| Scope creep into v2 features | **High** | Deferred list is written down and dated; revisit only after two weeks of use |

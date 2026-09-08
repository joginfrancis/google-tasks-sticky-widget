# Architecture

Technical design for the Google Tasks Sticky Widget. Read [SPEC.md](SPEC.md) first
for what the product does and does not do.

---

## 1. Stack

| Layer | Choice | Reason |
|---|---|---|
| Shell | Tauri 2.x | Native Win32 window control, tray, and credential access from Rust; ~10 MB binary vs Electron's ~120 MB. |
| Frontend | React + TypeScript + Vite | Fast iteration on the UI, which is where most of the design work is. |
| Native | Rust | OAuth, token storage, HTTP to Google, SQLite, window and tray management. |
| Local store | SQLite (`rusqlite`) | Read cache and settings. Small, embedded, no server. |
| Credentials | Windows Credential Manager (`keyring` crate) | OS-level DPAPI protection for the refresh token. |

**Electron is rejected** on binary size and memory for a widget expected to run
permanently. **A backend server is rejected** — it would put the user's tasks on
someone else's machine for no benefit (see §8).

## 2. Process and trust boundary

```
  ┌──────────────────────────────────────────┐
  │  WebView (React)                         │
  │  · renders tasks, handles input          │  ← no network access,
  │  · holds no tokens, makes no HTTP calls  │    no credentials
  └────────────────┬─────────────────────────┘
                   │ Tauri IPC (typed commands)
  ┌────────────────▼─────────────────────────┐
  │  Rust core                               │
  │  ┌────────────┐  ┌────────────────────┐  │
  │  │ SyncManager│──│ TasksClient (HTTP) │──┼──→ googleapis.com
  │  └─────┬──────┘  └─────────┬──────────┘  │
  │        │                   │             │
  │  ┌─────▼──────┐   ┌────────▼──────────┐  │
  │  │ LocalStore │   │ AuthManager       │  │
  │  │ (SQLite)   │   │ (PKCE, refresh)   │  │
  │  └────────────┘   └────────┬──────────┘  │
  │                            │             │
  │                   ┌────────▼──────────┐  │
  │                   │ Credential Manager│  │
  │                   └───────────────────┘  │
  └──────────────────────────────────────────┘
```

**The critical invariant: the WebView never sees a token and never talks to
Google.** All network I/O is in Rust. If the WebView is ever compromised — a
malicious task title, a supply-chain issue in an npm dependency — it can call
IPC commands but cannot exfiltrate credentials, because it never holds them.

This drives the IPC design in §3.

## 3. IPC surface

Every exposed command is an attack surface, so the list is deliberately short and
each is coarse-grained (an intent, not a primitive).

```
list_tasks()                     -> Task[]           // from cache
list_task_lists()                -> TaskList[]
create_task(title)               -> Task
set_task_completed(id, bool)     -> Task
delete_task(id)                  -> ()
select_task_list(id)             -> ()
sync_now()                       -> SyncStatus
get_sync_status()                -> SyncStatus
get_settings() / set_settings()  -> Settings
begin_google_auth()              -> ()               // fire and forget
disconnect_google()              -> ()
get_account_status()             -> AccountStatus    // email + connected bool
set_always_on_top(bool)          -> ()
hide_to_tray()                   -> ()
```

Rules:

- No command accepts a URL, a file path, or raw HTTP parameters.
- No command returns a token, an authorization code, or a PKCE verifier.
- `get_account_status` returns the account email for display and nothing else.
- Tauri capabilities are allowlisted per-command; the default filesystem, shell,
  and HTTP plugins stay disabled.

State changes are pushed to the frontend with Tauri events (`tasks:updated`,
`sync:status`, `auth:changed`) rather than the frontend polling IPC.

## 4. Authentication

### 4.1 Flow

Authorization Code + PKCE, in the system browser, with a loopback redirect.

1. Generate a 32-byte random PKCE verifier and a 32-byte random `state`, both
   from a CSPRNG. Compute `code_challenge = BASE64URL(SHA256(verifier))`.
2. Bind an ephemeral listener on `127.0.0.1:0`; the OS-assigned port becomes part
   of the redirect URI (`http://127.0.0.1:{port}`).
3. Open the authorization URL in the default browser via the OS shell.
4. On callback: validate `state` in constant time, reject on mismatch, then close
   the listener immediately. Serve a plain "You can close this window" page.
5. Exchange the code for tokens over HTTPS, sending the verifier.
6. Store the refresh token in Credential Manager. Keep the access token in
   process memory only — never persisted.

The listener binds to `127.0.0.1`, never `0.0.0.0`, and shuts down the moment it
has a response or after a 5-minute timeout.

### 4.2 On the client secret

**Settled 2026-09-07: the secret is required.** The docs list `client_secret` as
"Optional", but a real Desktop client rejects the exchange without it
(`400 invalid_request`) and accepts it with. So **the secret ships inside the
binary**, where anyone can extract it.

This is expected and documented — a desktop client secret is not a confidential
credential. Do not attempt to obfuscate it; that accomplishes nothing and
complicates the build.

`exchange_code` still tries without the secret first and falls back. That costs
one failed request on first connect and nothing thereafter, and it means the app
keeps working unchanged if Google relaxes the requirement.

Either way, **PKCE is what actually secures the flow** — without the verifier, an
intercepted authorization code is useless. On a loopback redirect this is not
optional in practice: any other local process could otherwise race for the code.

The genuine secret is the **user's refresh token**, which is why every rule in §7
is about that and not about the client credentials.

Note also that Google has **removed** the out-of-band (OOB) copy/paste redirect
and discourages custom URI schemes for desktop apps on impersonation grounds.
Loopback is confirmed supported for Windows desktop and is the right choice here.

### 4.3 Refresh token lifetime

While the OAuth consent screen is in **Testing** mode, Google expires refresh
tokens after 7 days, which would force a reconnect every week.

Confirmed 2026-09-05. The exemption for this rule covers only name/email/profile
scopes; `auth/tasks` is well outside it.

**Publish the consent screen to Production, unverified, before real use.** The
user accepts an "unverified app" warning once and refresh tokens then persist.
Verification review is only required for distribution to other people, which
SPEC §2.3 puts out of scope.

Two further expiry causes to handle rather than crash on:

- **Six months of non-use** invalidates a refresh token.
- **100 refresh tokens per account per client ID**; exceeding it silently
  invalidates the oldest. Only reachable through repeated connect/disconnect
  cycles — worth remembering if auth breaks inexplicably during development.

Both degrade to the same place: clear credentials, show the connect prompt.

### 4.4 Token refresh and revocation

- Refresh proactively when the access token is within 5 minutes of expiry.
- A `401` triggers exactly one refresh-and-retry; a second `401` means the grant
  is gone — clear credentials, emit `auth:changed`, show the connect prompt.
- Disconnect calls Google's revocation endpoint, then deletes the Credential
  Manager entry and clears cached account data. Deletion happens even if
  revocation fails, so a network error cannot leave credentials behind.

## 5. Sync

### 5.1 Why polling

The Tasks API has no `watch` / push channel — unlike Drive, Calendar, and Gmail,
it was never implemented. There is no Pub/Sub topic and no long-poll endpoint.

Even if there were, push delivers to a public HTTPS endpoint that Google can
reach, which a desktop app does not have. Using it would require a relay server,
which SPEC §2.3 rules out. Polling is not a compromise here; it is the correct
design for this class of client.

### 5.2 Delta queries

`tasks.list` accepts `updatedMin`, so steady-state polling asks only "what changed
since my last successful sync" and usually gets an empty response back. This is
what makes frequent polling cheap.

Four requirements, all verified 2026-09-05 (see
[docs/api-findings.md](docs/api-findings.md)):

- **`showDeleted=true`**, or deletions made elsewhere never arrive and stale rows
  persist indefinitely.
- **`showHidden=true`**, without exception. A task completed in Google's own web
  or mobile client is marked *hidden*, and `showHidden` defaults to false — so it
  would drop out of results entirely and the widget would keep showing it as
  incomplete forever. This is the single easiest way to ship a convincing
  phantom bug.
- **Paginate.** `maxResults` defaults to 20, max 100. Loop on `pageToken` until
  exhausted on every call, delta and full alike.
- **Persist the cursor from the last *successful* response**, minus ~30 s of
  slack for clock skew. Overlapping is harmless — re-applying an identical update
  is a no-op. A gap loses an edit permanently.

A full, non-delta sync runs on first connect, on manual refresh, and after any
sync error that leaves the delta cursor untrustworthy.

### 5.3 Cadence

| State | Interval |
|---|---|
| Window visible, interaction within the last 2 min | 30 s |
| Window visible, idle | 60 s |
| Hidden to tray, or session locked | 5 min |
| Offline | paused; resume on network-up |

Event-driven refreshes matter more than the interval, and each resets the timer:

- Window shown or focused — makes perceived lag near zero regardless of cadence
- Resume from sleep
- Network reconnect
- Immediately after any local write, to reconcile

Nothing faster than 30 s. Not for quota reasons — the documented courtesy limit
is 50,000 queries/day, and 30 s polling of one list is ~2,900/day, under 6% —
but because
Google's own clients take seconds to propagate anyway, and a network wake-up
every few seconds is a real idle-power cost on a laptop.

### 5.4 Error handling and backoff

| Status | Response |
|---|---|
| `401` | Refresh once, retry once. Second failure → disconnect state. |
| `403` | Inspect the reason. Rate-limit reasons back off; permission errors surface as a reconnect prompt. |
| `404` | Task or list deleted elsewhere. Drop it from the cache silently. |
| `429` | Back off, honour `Retry-After` when present. |
| `5xx` | Back off and retry. |
| Timeout / DNS failure | Treat as offline; do not retry aggressively. |

Backoff doubles from the base interval to a 15-minute ceiling, with jitter, and
resets on the first success. Without this, an expired grant or a flapping
connection becomes a hot retry loop that burns quota and can turn a blip into a
rate-limited outage longer than the original problem.

### 5.5 Writes in v1

There is no pending-operation queue in v1 (SPEC §2.2). A write is:

1. Apply optimistically to the local store and emit `tasks:updated`.
2. Issue the API call with up to 3 retries and backoff.
3. On success, reconcile the server's returned representation into the cache.
4. On final failure, revert the optimistic change and attach an error to that row.

The user's typed text is never discarded without being shown — a failed
quick-add returns its text to the input rather than vanishing.

## 6. Conflicts — an honest account

The Task resource does carry an `etag` field (output only), but the API provides
**no conditional writes** — there is no `If-Match` on `patch`, so there is no way
to say "write this only if it hasn't changed."

The consequence: a read-then-write cannot be made atomic. Between reading a task
and writing it, a change from the phone can land, and the write will overwrite it
with no way to detect that at write time. The window is short but it is real, and
**no amount of client-side logic closes it.**

What is achievable, and what v2 should implement:

- Compare `updated` timestamps before pushing, and treat a newer remote value as
  a conflict.
- Prefer the local change for user-initiated edits, since the user just made it.
- Surface a message when a conflict is resolved non-obviously, rather than
  silently discarding either side.

What must not be written into a spec as a guarantee: "never overwrite newer
remote changes." It cannot be guaranteed on this API. For a single-user widget
where the same person is on both ends, the practical risk is low — but the
limitation belongs in the docs rather than in an unfulfillable requirement.

## 7. Security

### 7.1 Token handling

1. Refresh tokens live only in Windows Credential Manager, under a
   single well-known entry name.
2. Access tokens live in process memory only, never persisted.
3. No token in SQLite, `localStorage`, `sessionStorage`, or any file.
4. No token crosses the IPC boundary into the WebView.
5. No token, authorization code, or PKCE verifier is logged at any level,
   including `DEBUG`.
6. No token is sent anywhere except Google's own HTTPS endpoints.

### 7.2 OAuth hardening

- System browser only; embedded WebView authentication is forbidden.
- PKCE verifier and `state` from a CSPRNG, 32 bytes each.
- `state` compared in constant time; mismatch aborts the flow.
- Loopback listener bound to `127.0.0.1`, torn down immediately after use.
- Single scope: `https://www.googleapis.com/auth/tasks`. No Gmail, Drive,
  Calendar, or Contacts.
- HTTPS only, with certificate validation left at defaults.

### 7.3 Logging

Structured logs with `ERROR` / `WARN` / `INFO` / `DEBUG`, `DEBUG` off in release
builds. A redaction helper wraps anything auth-adjacent; **full authorization
URLs are never logged** because they carry the code and state as query
parameters. Log task IDs, never task content.

### 7.4 Repository hygiene

`.gitignore` covers `client_secret*.json`, `credentials.json`, `.env*`,
`*.token`, and the local SQLite files. No credential file is ever committed; if
one is, the client is rotated in the Cloud Console rather than merely deleted
from the working tree.

### 7.5 Review checklist before release

Grep the tree for: `access_token`, `refresh_token`, `client_secret`,
`authorization_code`, `code_verifier`, `localStorage`, `sessionStorage`,
`console.log`, `println!`, `dbg!`. Then audit git history for credential files,
re-read every Tauri capability for permissions that are not required, and re-read
every IPC command for anything that would let the frontend reach privileged
native functionality it does not need.

## 8. Data model

SQLite is a **cache and settings store**, not a source of truth. Google is
authoritative; the local database exists so the widget can paint instantly on
launch and survive a network outage. It may be deleted at any time without data
loss.

```
task_lists
  id            TEXT PRIMARY KEY
  title         TEXT
  updated       TEXT              -- RFC3339 from the API

tasks
  id            TEXT PRIMARY KEY
  task_list_id  TEXT
  parent_id     TEXT NULL         -- one level only, see SPEC §4.4
  title         TEXT
  notes         TEXT NULL         -- stored, not editable in v1
  due           TEXT NULL         -- date only, see SPEC §4.1
  status        TEXT              -- needsAction | completed
  completed_at  TEXT NULL
  position      TEXT              -- server-assigned, read-only
  updated       TEXT
  deleted       INTEGER
  sync_state    TEXT              -- synced | pending | error

sync_state
  key           TEXT PRIMARY KEY  -- e.g. last_sync_updated_min
  value         TEXT

settings
  key           TEXT PRIMARY KEY
  value         TEXT
```

`pending_operations` is deliberately absent — it belongs with the v2 offline
queue. `sync_state` on a task carries the transient optimistic flag in v1.

**No authentication material appears in this database.**

## 9. Frontend structure

```
src/
  components/
    TaskWidget/          shell, header, drag region
    TaskItem/            row, checkbox, due badge, context menu
    TaskInput/           inline quick-add
    TaskListSelector/
    SyncStatus/          dot + detail popover
    Settings/
    Onboarding/          first-run connect prompt
  hooks/                 useTasks, useSyncStatus, useSettings
  ipc/                   typed wrappers over Tauri invoke + event listeners
  types/                 shared with Rust via generated bindings
  styles/                tokens (light/dark), primitives
src-tauri/
  src/
    main.rs
    commands/            IPC entry points, one module per domain
    auth/                pkce.rs, flow.rs, tokens.rs
    google/              client.rs, models.rs, errors.rs
    sync/                manager.rs, scheduler.rs, backoff.rs
    store/               db.rs, migrations.rs, queries.rs
    secure/              credential_manager.rs
    window/              positioning, always-on-top, tray, autostart
  capabilities/          minimal allowlist
```

The `ipc/` layer is the only place the frontend touches Tauri, and `types/` is
generated from the Rust structs so the two sides cannot drift.

## 10. Theming

CSS custom properties on `:root` for the light palette, redefined under a
`[data-theme="dark"]` selector and under `prefers-color-scheme: dark` guarded so
an explicit choice wins in both directions. Theme preference is persisted in
settings; "System" subscribes to the Windows theme change event through Tauri.

## 11. Open questions

Phase 0 (2026-09-05) closed the API questions — see
[docs/api-findings.md](docs/api-findings.md). Remaining:

1. ~~Can a Desktop client omit `client_secret`?~~ **Closed 2026-09-07: no.** See §4.2.
2. **Can `tasks.insert` create a subtask via its `parent` parameter**, given the
   resource field is read-only? Only matters if subtask creation is ever added;
   v1 renders but does not create them.
3. **Tauri 2 autostart plugin vs. a registry write** — which behaves better with
   the "start hidden" option.
4. ~~`keyring` Windows feature flag~~ **Closed 2026-09-07.** `keyring` 4.2.0 with
   `default-features = false` and features `["v1", "windows-native-keyring-store"]`.
   Default features drag in the whole Linux secret-service stack, so disabling
   them matters. A real round-trip test against Credential Manager passes.

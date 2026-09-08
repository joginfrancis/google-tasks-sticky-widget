# Phase 0 — API findings

Verified against live Google documentation on **2026-09-05**. Re-check before any
major rework; OAuth policy and API surface both change.

---

## 1. `tasks.move` supports `destinationTasklist` — CONFIRMED PRESENT

Moving a task between lists **is** a first-class operation. `tasks.move` takes:

| Parameter | Description |
|---|---|
| `parent` | New parent task identifier. Omitted if moving to top level. |
| `previous` | New previous sibling identifier. Omitted if moving to first position. |
| `destinationTasklist` | Destination task list identifier. If set, the task is moved from `tasklist` to this list. |

**Caveat:** the docs note recurrent tasks cannot currently be moved between lists.

**Impact:** reverses the earlier concern. The destructive delete-and-recreate
fallback is not needed. Moving between lists is a clean v2 feature.

Source: [tasks.move](https://developers.google.com/workspace/tasks/reference/rest/v1/tasks/move)

## 2. `due` is date-only — CONFIRMED

> "It isn't possible to read or write the time that a task is scheduled for using the API."

The field is an RFC 3339 timestamp, but the time component is not retained.

**Impact:** no time picker, ever. Matches SPEC §4.1 as written.

## 3. `position` is output-only — CONFIRMED

> "Output only." … "Use the 'move' method to move the task to another position."

**Impact:** matches SPEC §4.3. Reordering is a `tasks.move` call, not a `patch`.

## 4. `parent` is documented read-only on the resource

The `parent` field on the Task resource is marked read-only, and "a parent task
can never be an assigned task (from Chat Spaces, Docs)."

**Unresolved:** `tasks.insert` accepts a `parent` query parameter, so creating a
subtask should be possible at insert time; re-parenting an existing task goes
through `tasks.move`. **Verify empirically in Phase 5** — this only matters if
subtask creation is ever added, which v1 does not do.

No maximum nesting depth is stated in the reference. The one-level limit is
observed behavior in the product, not a documented API constraint — treat the
UI's single indent level as a rendering decision rather than an enforced rule.

## 5. Recurrence — CONFIRMED ABSENT

No recurrence field or concept appears anywhere in the Task resource reference.
Repeating tasks surface as ordinary single tasks.

## 6. `showHidden` — NEW, AND IT MATTERS

> `showCompleted` — "Flag indicating whether completed tasks are returned in the
> result." Defaults to **True**. Note that **`showHidden` must also be True** to
> display tasks completed in first-party clients like the web UI.

`showHidden` defaults to **False**.

**Impact — this is the most consequential finding of Phase 0.** A task completed
in the Google Tasks web or mobile app becomes *hidden*, and with default
parameters it will simply not come back from `tasks.list`. Without
`showHidden=true`:

- The "Completed (n)" section would be permanently empty for anything completed
  elsewhere.
- Worse, in a **delta sync** the task would vanish from results entirely,
  and the widget would keep showing it as incomplete forever — indistinguishable
  from a sync bug.

**Action:** always send `showHidden=true` and `showDeleted=true` on every
`tasks.list` call, delta and full alike. Added to ARCHITECTURE §5.2 as a hard
requirement.

## 7. Pagination is mandatory — NEW

`maxResults` defaults to **20**, maximum **100**.

**Impact:** the plan had no pagination. A list with more than 20 items would
silently truncate. Every `tasks.list` must loop on `pageToken` until exhausted.
Added to ARCHITECTURE §5.2.

Also available and potentially useful later: `completedMin`, `completedMax`,
`dueMin`, `dueMax`, `showAssigned`.

Source: [tasks.list](https://developers.google.com/workspace/tasks/reference/rest/v1/tasks/list)

## 8. OAuth for desktop apps

**Out-of-band (OOB) is removed.**

> "The manual copy/paste option, also referred to as an out of band (OOB)
> redirect method, is no longer supported."

**Custom URI schemes are discouraged** — the docs flag them as no longer
supported for desktop due to app-impersonation risk.

**Loopback IP redirect is supported** for "macOS, Linux, and Windows desktop
(but not Universal Windows Platform) apps." This confirms the
`http://127.0.0.1:{ephemeral_port}` approach in ARCHITECTURE §4.1.

**PKCE** is described as "Recommended" rather than mandated, with
`code_challenge` and `code_challenge_method` listed as recommended parameters.
We use it regardless — for a loopback redirect on a shared machine it is the only
thing preventing another local process from racing the authorization code.

**`client_secret` is listed as "Optional"** at the token exchange, with a note
that it is "not applicable to requests from clients registered as Android, iOS,
or Chrome applications." Desktop app clients are not in that exemption list and
Google does issue a secret for them.

**RESOLVED 2026-09-07 — the secret is required.** The implementation tries the
exchange without it first and falls back. Observed on a real Desktop client:

```
WARN  token endpoint returned 400 Bad Request: invalid_request
INFO  retrying token exchange with the client secret
INFO  google account connected
```

So despite the docs listing `client_secret` as "Optional", a Desktop app client
cannot omit it. **The secret ships inside the binary**, where anyone can extract
it. This is expected and documented by Google — a desktop client secret is not a
confidential credential, and PKCE is what secures the flow. Do not obfuscate it.

The retry-then-fallback logic stays in place: it costs one failed request on
first connect only, and it means the app keeps working if Google ever relaxes
this.

Source: [OAuth 2.0 for Mobile & Desktop Apps](https://developers.google.com/identity/protocols/oauth2/native-app)

## 9. Refresh token expiry — CONFIRMED, plus extras

> "A Google Cloud Platform project with an OAuth consent screen configured for an
> external user type and a publishing status of 'Testing' is issued a refresh
> token expiring in 7 days, unless the only OAuth scopes requested are a subset
> of name, email address, and user profile."

`auth/tasks` is well outside that exemption, so **the 7-day expiry applies to us
in Testing.** Publishing the consent screen to Production remains a hard gate in
BUILD_PLAN Phase 1.

Two additional expiry causes worth handling:

- **Six months of non-use** invalidates a refresh token. Rare for a widget that
  runs daily, but the app must degrade to the connect prompt rather than error.
- **100 refresh tokens per Google Account per client ID.** Exceeding it silently
  invalidates the oldest. Only reachable by repeated connect/disconnect cycles
  during development — worth knowing if auth mysteriously breaks in Phase 4.

Source: [Using OAuth 2.0 to Access Google APIs](https://developers.google.com/identity/protocols/oauth2)

## 10. Quota — CONFIRMED

> "The Tasks API has a courtesy limit of 50,000 queries per day."

No per-minute or per-user-per-minute limit is documented.

**Impact:** the cadence in ARCHITECTURE §5.3 is far inside budget. Steady-state
30 s polling of one list is ~2,900 requests/day, under 6% of quota. Even with
pagination on a large list, headroom is ample. The cadence is chosen for battery
and propagation-latency reasons, not quota — as ARCHITECTURE §5.3 already says.

Source: [Tasks API quotas](https://developers.google.com/workspace/tasks/limits)

## 11. `keyring` crate

Current release **4.2.0**. Core API is `Entry::new` / `set_password` /
`get_password` / `delete_credential`.

Windows Credential Manager support comes via `windows-native-keyring-store`,
listed as an **optional** dependency — so a feature flag is likely required
rather than it being on by default. Confirm the exact flag when adding the
dependency in Phase 4.

**Secret size:** no limit documented in the crate. Windows Credential Manager
itself caps the credential blob (commonly cited at 2,560 bytes). Google refresh
tokens are a few hundred characters, so this is not a practical concern — but do
not use this store for anything larger.

Sources: [keyring on docs.rs](https://docs.rs/keyring) ·
[keyring-rs](https://github.com/open-source-cooperative/keyring-rs)

---

## Summary of changes forced by Phase 0

| Finding | Change |
|---|---|
| `showHidden` must be true | Hard requirement on every `tasks.list`; would have caused a silent, bug-looking sync failure |
| Pagination required | `pageToken` loop added; would have truncated lists over 20 items |
| `destinationTasklist` exists | List-move de-risked; caveat about recurrent tasks noted |
| `client_secret` possibly omissible | Flagged for a 10-minute empirical test in Phase 4 |
| Custom URI schemes discouraged | Loopback confirmed as the only sensible choice |

Two of these — `showHidden` and pagination — would have shipped as convincing
bugs. Phase 0 paid for itself.

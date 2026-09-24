# Sticky Widget

By **[Jogin Francis](https://github.com/joginfrancis)**.

Your Google Tasks lists as sticky notes on the Windows desktop. One note per
list, each its own small window with its own colour and its own place on
screen. Everything you change is your real Google Tasks data, synced both ways
with the web app and your phone.

**[Download for Windows](https://github.com/joginfrancis/google-tasks-sticky-widget/releases/latest)**
 · [Website](https://joginfrancis.github.io/google-tasks-sticky-widget/)
 · [Privacy policy](https://joginfrancis.github.io/google-tasks-sticky-widget/privacy.html)

## What it does

- **Add without ceremony.** Type and press Enter, again and again. Paste an
  indented list and you get a list, subtasks and all. Add a task directly below
  any other, or a subtask inside it.
- **Drag anywhere.** Reorder by dragging, drag right to nest, drag a task onto
  another note to move it to that list.
- **Dates.** Today and tomorrow in a click, a calendar when you need a real
  date.
- **Descriptions** take bold, italics, bullets, clickable links and linked
  images — written as the plain text Google stores, so nothing is lost on your
  phone.
- **Out of the way.** One shortcut shows or hides every note from any
  application; closing them puts the app in the tray.

Not included, because the Google Tasks API does not offer them: starring,
repeats, attachments, and working offline. The app says so where you would look
for them, and links to Google Tasks for the ones that live there.

## Installing

Download the `.exe` from the
[latest release](https://github.com/joginfrancis/google-tasks-sticky-widget/releases/latest)
and run it. Windows will warn that the publisher is unknown — the installer is
not code signed — so choose **More info**, then **Run anyway**. Open the app,
click **Connect**, and allow access to your tasks.

New versions offer themselves inside the app.

## Your data

The app talks to Google and to nobody else. Tasks are cached on your own
machine, the sign-in token lives in Windows Credential Manager, and there are
no analytics of any kind. The
[privacy policy](https://joginfrancis.github.io/google-tasks-sticky-widget/privacy.html)
says it in full, and the source here is the proof.

## Building it yourself

The published build is tied to one Google Cloud project, whose API quota is
shared by everyone using that build. A copy built with your own Google client
is an independent app with its own quota and its own consent screen — see
**[docs/build-your-own.md](docs/build-your-own.md)**.

```bash
npm install
npm run tauri dev
```

Tests: `npx vitest run` for the frontend, `cargo test` in `src-tauri` for the
Rust core.

## How it is put together

Tauri 2 with a Rust core and a React + TypeScript frontend. One SQLite cache
and one sync manager in Rust; every note window is a view of the same state.
The design notes are worth reading before changing anything:

- [ARCHITECTURE.md](ARCHITECTURE.md) — the shape of the app and why
- [SPEC.md](SPEC.md) — what it is meant to do
- [docs/api-findings.md](docs/api-findings.md) — what the Google Tasks API
  actually does, measured rather than assumed
- [docs/build-your-own.md](docs/build-your-own.md) — running it on your own
  Google project
- [docs/publishing-and-verification.md](docs/publishing-and-verification.md) —
  Google's consent screen and verification

## Credits

Created and designed by **Jogin Francis** — the idea, the product design,
the interaction and visual design, and the architectural decisions behind how
it works. Many of the harder problems here were solved by him directly: how a
task drags between two separate note windows, how a plain-text description can
carry formatting without lying to Google, and where the app should refuse to
pretend an API limit does not exist.

The code was written by Claude (Anthropic) working to his direction, review
and repeated refinement.

## Licence

MIT — see [LICENSE](LICENSE). Copyright © 2026 Jogin Francis.

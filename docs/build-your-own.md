# Building your own copy

The published app is tied to one Google Cloud project — mine. That project owns
the OAuth client baked into the installer, and its API quota is shared by
everyone who uses that build.

You do not have to accept either. The app reads its Google credentials from
whatever you give it, so a copy built with your own client is a genuinely
independent app: your consent screen, your quota, your users. Nothing in the
code points back here.

Reasons to do this rather than installing the release:

- **Quota.** 50,000 Tasks API requests a day are shared across all users of a
  build. Your own project starts with its own 50,000.
- **Trust.** You compile it yourself, so you are not taking my word for what
  the binary does.
- **A Workspace of your own.** An *Internal* app in your organisation needs no
  Google verification at all and never expires its sign-in. If everyone using
  it shares a domain, this is by far the easiest route.
- **Changing it.** It is your fork; the code is yours to alter.

## 1. A Google Cloud project

1. [console.cloud.google.com](https://console.cloud.google.com) → create a
   project.
2. **APIs & Services → Library → Google Tasks API → Enable.**
3. **Google Auth Platform → Audience**, and choose:
   - **Internal** if the project sits in a Google Workspace organisation and
     only people in it will use the app. No verification, no expiry, done.
   - **External** otherwise. You then get a *Testing* app: add each user's
     Google account under **Test users** (up to 100), and be aware that a
     sign-in expires after 7 days until the app is verified. See
     [publishing-and-verification.md](publishing-and-verification.md) if you
     want to remove that limit.
4. **Clients → Create client → Desktop app.** Download the JSON.

The download is a *desktop* client. Its "secret" is not a confidential
credential — Google says so, and PKCE is what actually protects the flow (see
[api-findings.md](api-findings.md) §8). It still should not be committed to a
public repository, but nothing breaks if someone reads it out of a binary.

## 2. Point the build at it

Either of these works; the file wins if both are present.

**A file on the machine that builds it** — the development route:

```
%USERPROFILE%\.gtasks-widget\client_secret_<whatever>.json
```

Or set `GTASKS_CLIENT_SECRET_FILE` to its full path.

**Environment variables** — what CI uses:

```
GTASKS_CLIENT_ID=<the client_id from the JSON>
GTASKS_CLIENT_SECRET=<the client_secret from the JSON>
```

`build.rs` bakes whichever it finds into the binary, so the installed app needs
nothing on the user's machine. A build with neither still compiles; it then
looks for the file at run time and says so plainly if there is none.

## 3. Build it

Prerequisites: [Node](https://nodejs.org) 20+, [Rust](https://rustup.rs), and
the Microsoft C++ build tools that Tauri lists for Windows.

```bash
npm install
npm run tauri dev      # run it
npm run tauri build    # installers, in src-tauri/target/release/bundle
```

## 4. If you want your own releases and updates

The release pipeline in `.github/workflows/release.yml` builds, signs and
publishes installers when you push a `v*` tag. To use it on your fork:

1. `npx tauri signer generate -w ~/.tauri/myapp.key` — this key is what proves
   an update came from you. Keep it; losing it means every user reinstalls.
2. Put the **public** key in `src-tauri/tauri.conf.json` under
   `plugins.updater.pubkey`, and change `endpoints` to your own repository.
3. Add repository secrets: `TAURI_SIGNING_PRIVATE_KEY` (the private key file's
   contents), `GTASKS_CLIENT_ID`, `GTASKS_CLIENT_SECRET`.
4. `git tag v0.1.0 && git push origin v0.1.0`, then publish the draft release
   the workflow creates.

## What your users will see

Windows will warn that the publisher is unknown, because the installer is not
code signed. That is a certificate you buy, not something the code can fix.

## What you are responsible for

The consent screen carries your name, the privacy policy link is yours, and the
quota is yours to manage. If you publish your build to people outside your
organisation, write your own privacy policy rather than pointing at mine — it
has to describe *your* app and *your* project.

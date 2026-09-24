# Publishing the app to real users

Everything here is done in Google's console by the project owner, not in this
repository. Checked against Google's own documentation on 2026-09-23; the
console is renamed and rearranged often, so trust the wording on screen over
the wording here.

## Why this is needed at all

The app asks for one scope, `https://www.googleapis.com/auth/tasks`. Google
classes that as **sensitive**, which has two consequences while the consent
screen is in **Testing**:

- only accounts listed as **test users** can sign in — 100 of them, at most;
- a refresh token expires after **7 days**, so every user is signed out
  roughly weekly, however often they use the app.

Neither can be worked around in code, and neither is a bug. Publishing the
consent screen and passing verification removes both.

Sensitive scopes do **not** need the annual third-party security assessment —
that applies only to *restricted* scopes (Gmail, Drive file contents). This is
a form and a video, not an audit, and it costs nothing.

## What to have ready before starting

1. **A homepage**, on a domain you control. Not a login page: it has to say
   what the app is and what it does, and link to the privacy policy. GitHub
   Pages is free and sufficient — `https://joginfrancis.github.io/google-tasks-sticky-widget/`
   works, and so does a custom domain.
2. **A privacy policy page**, on the same domain, linked from the homepage. It
   must say plainly what Google data the app touches (tasks and task lists),
   why, where it is stored (this machine, in SQLite, plus the refresh token in
   Windows Credential Manager) and that it is never sent anywhere else.
3. **Domain ownership**, verified in Google Search Console with the same Google
   account that owns the Cloud project.
4. **A demo video**, unlisted on YouTube is fine. It has to show, in one
   unbroken flow: opening the app, clicking Connect, the **whole consent
   screen in English** with the scope visible, and then the app actually using
   Google Tasks — adding a task, completing one, the list syncing.
5. **A justification for the scope**, a few honest sentences. See below.

## The steps

1. **Google Cloud Console → APIs & Services → OAuth consent screen**
   (newer consoles call it *Google Auth Platform*). Pick the project that owns
   the client this app is built with.
2. **Branding**: app name, user support email, app logo (optional — a logo
   triggers a separate brand review, so skip it for the first pass), homepage
   URL, privacy policy URL, developer contact email. The privacy policy link
   here and the one on the homepage must be the same URL.
3. **Authorized domains**: add the domain the homepage is on. It must already
   be verified in Search Console or the console will reject it.
4. **Data access / Scopes**: confirm `auth/tasks` is the only scope listed. If
   anything else is there, remove it — every extra scope is another thing to
   justify, and a reason to be rejected.
5. **Audience → Publish app**, then **Prepare for verification**. Fill in the
   scope justification and paste the video link.
6. **Submit**, and answer the reviewer's email. Replies are the slow part: a
   first response typically comes within a few days, and the whole thing takes
   a few weeks if nothing needs changing.

### What to write for the scope justification

Say what the app is, what it does with the data, and why nothing narrower
exists. Something like:

> Sticky Widget is a Windows desktop widget that shows a user's own Google
> Tasks lists as sticky notes on their desktop and lets them add, edit,
> complete, reorder and delete tasks. The `auth/tasks` scope is the only scope
> Google offers that permits writing tasks; `tasks.readonly` would make the
> app read-only, which removes its entire purpose. Task data stays on the
> user's machine and is exchanged only with Google's Tasks API. No task
> content is sent to any other server, and the app collects no analytics.

That last sentence has to stay true. It is true today.

## While verification is pending

Nothing breaks. The app keeps working for existing users, and test users can
still sign in. Publishing takes effect for everyone once approved.

## What does *not* need doing again

Verification belongs to the Cloud project and its consent screen, not to a
release. Shipping new versions of the app needs no approval from Google.

Re-review is triggered only by changing **what you ask for or who you say you
are**: adding a scope, renaming the app, adding or changing the logo, or
changing the homepage or privacy policy domain. Existing users keep working
while that review runs.

## The one number to watch afterwards

The Tasks API allows **50,000 requests per day per project**, shared by every
user of the app. At the current 30-second polling cadence that is roughly
2,900 requests per user per day, so about 15 users. Backing off when a note is
hidden or idle is the change that moves this number, and it is worth making
before inviting a crowd.

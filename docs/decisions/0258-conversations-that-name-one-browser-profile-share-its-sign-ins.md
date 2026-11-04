---
status: proposed
---

# Conversations that name one browser profile share its sign-ins

## Problem

A team's chat bot gives each person a Cloud and moves a group's
conversation to the Cloud of whoever speaks. Alex signs in to
`shop.example.com` in the browser of one group's conversation, with a code
from the phone. In another group the bot opens the shop on the same Cloud,
signed out: each conversation has a browser profile of its own (0193), so
Alex signs in again, once per group.

On a Cloud the sign-in does not last even in its own conversation. The
profile is in the conversation's scratch directory, under the runner's
installation directory, which on a Cloud is `/run/demi`, a memory-backed
`/run`: a Cloud that stops when idle loses every sign-in.

## Decision

An integration's conversation can name a browser profile. Its browser runs
on that profile, and every conversation on the same Host that names the
same profile uses the same browser: a sign-in, a cookie or a site's storage
one of them makes, the others see, as two windows of one Chrome do.

`CreateConversation` and `PATCH /v1/conversations/{id}` take
`browserProfile`, such as `"alex"`; none keeps the conversation's own
profile, as today. The web app's conversations name none and are unchanged.

### The profile is a directory

A named profile is a directory in the home of the Host's user:

```text
~/.demi/browser-profiles/alex/     Chrome's user data of the profile alex
  profile.lock                     held while a Chrome runs on it
```

- It is made when a browser first starts on it, and outlives every
  conversation, the Cloud's stops and its reset, as the rest of the home
  does.
- Demi stores nothing of it: no table, no list. A program lists, copies or
  removes profiles with the Files routes, as any directory
  (`/v1/clouds/{cloud}/fs`). A profile removed while its browser runs is
  signed out only once that browser stops: Chrome keeps what it holds and
  writes the directory again.
- The name is one directory name: not empty, no `/`, not `.` or `..`.

### What each conversation keeps of its own

Several conversations share the profile's Chrome, and each keeps:

- **Its tabs.** `demi browser tab list` lists the conversation's tabs only,
  numbered from its own sequence as today, and a command names only those.
  A popup belongs to the conversation of the tab that opened it.
- **Its live view, view links (0248) and recordings**, which follow its
  tabs as today.
- **Its viewports.**

The downloads and the files the user chose to upload are the Chrome's, in
the profile's `downloads/` and `uploads/`, as they are today, and are
emptied when the Chrome stops. So are the language, time zone and color
scheme its pages see, which the command that started it brought.

A conversation that moves to another Host uses that Host's profile of the
same name: a profile is a directory of one Host.

### Lifetime

- The profile's Chrome starts with the first tab any of its conversations
  opens, and stops when the last tab of all of them closes.
- A conversation's release, when it goes idle or moves, closes its own
  tabs and leaves the others'.
- A crash of Chrome ends every conversation's tabs on it; the next
  `tab new` starts it again on the profile, signed in.
- After a `PATCH` of `browserProfile`, the conversation's next browser
  command closes its tabs on the profile before, as a release does, and
  works on the profile it names now.

A conversation that names no profile keeps 0193's behavior: one profile
per conversation, in its scratch directory, removed when it is archived.

### Design

`CommandContext` (`crates/command-protocol/src/invocation.rs`) gains
`browser_profile: Option<String>`, which the backend fills from the
integration conversation's record (`integration_conversations.browser_profile`).
The browser resident keeps one Chrome per profile directory
(`ProfileBrowsers` in `crates/plugin-browser/command-chrome/src/tabs/environment.rs`),
and each conversation's browser is an environment on it:

- **The profile's Chrome** owns what is one per Chrome: the process tree, the
  profile and its lock, the handoff to another resident (0193), the CDP
  connection and its event pump, the capture extension's connection, the
  downloads and uploads, and the screen. It starts with the first
  environment and retires with the last.
- **A conversation's environment** owns what is one per conversation: the
  tab registry, its tab numbers, the live hub, the recordings, the viewport
  a tab the user opens takes, and the change counter. Its registry follows
  the Chrome's target events and admits a target when the conversation's
  command creates it or when its opener is one of the conversation's tabs.
  An environment that ends closes its tabs, unless its Chrome ends with it.

- **The virtual screen is the profile browser's.** Chrome has one screen
  for all its windows (`Emulation.updateScreen`), which each conversation's
  live view sets today. The profile's browser keeps it at the largest size
  and the highest ratio any of its conversations' viewers asked for, so one
  conversation's viewer never shrinks another's; the ratio only rises
  already.

A conversation with no name is a profile's browser with one conversation's
tabs, so both cases are one code path.

## Spike

Chrome for Testing, headless, on one profile, through CDP:

- **A popup names its opener.** A page opened by `window.open(url)`, by
  `window.open(url, '_blank', 'noopener')` and by a click on a link with
  `target=_blank rel=noopener` each arrived in `Target.targetCreated` with
  `openerId` naming the tab that opened it; only `canAccessOpener` differed.
  So every popup reaches the conversation of its opener.
- **A tab's viewport is its own; the screen is shared.** Each tab has a
  window of its own. `Emulation.setDeviceMetricsOverride` on one tab left
  another tab's size and ratio as they were. `Emulation.updateScreen` sent
  through one tab changed the other tab's `devicePixelRatio` and
  `screen.width` too.
- **A Chrome with no page lives on.** Headless with `--no-startup-window`,
  as Demi starts it, Chrome began with no page, and after its one page was
  closed it ran on for 3 seconds with none and opened a new one. So one
  conversation's last tab closing leaves the Chrome serving another that
  has none yet.
- **A profile removed under a running Chrome.** With a cookie set, the
  directory was removed. Chrome went on: a new tab opened, a second cookie
  was set, and both read back. Chrome made the directory again and wrote to
  it, and closed cleanly.

## Prior art

- Chrome runs one process per user data directory, so two Chromes cannot
  share one
  ([user data directory](https://chromium.googlesource.com/chromium/src/+/main/docs/user_data_dir.md)).
- Browser Use: parallel agents that share sign-ins share one browser
  session, each in its own tabs, since two sessions cannot run on one
  `user_data_dir`.
- Playwright's `launchPersistentContext` names a `userDataDir` the caller
  chooses, and every page of the context shares its storage.

## Alternatives

- **One profile per Host for every conversation.** It changes the web app's
  conversations, which keep one profile each, and leaves the program no way
  to keep two people's sign-ins apart on one Host.
- **Profiles as records Demi stores.** A profile is Chrome's directory; a
  record beside it would be a second owner of the same fact.
- **A Chrome per conversation, with cookies copied between them.** It misses
  site storage, IndexedDB and service workers, and two Chromes would write
  one site's cookies over each other.

## Open questions

None.

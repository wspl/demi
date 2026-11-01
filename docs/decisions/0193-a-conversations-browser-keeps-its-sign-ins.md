---
status: proposed
---

# A conversation's browser keeps its sign-ins

## Problem

Alex asks the agent to check the orders on `shop.example.com`. The agent
opens the page with `demi browser`, shows the sign-in, and Alex signs in by
hand in the work panel. An hour later Chrome crashes on a heavy page. The
next `demi browser tab new` opens the shop signed out, and Alex has to sign
in again, with a code from the phone.

The same happens whenever the conversation's Chrome stops:

- Chrome crashes, or its connection is lost;
- the agent or Alex closes the last tab;
- the conversation goes idle, moves to another Host or detaches the Host,
  each of which releases it there;
- the browser's resident restarts, as when the runner is upgraded.

Chrome keeps cookies and site storage in its profile, and the profile is a
temporary directory that lives as long as the Chrome process: retirement
removes it, and a later start sweeps one whose resident ended.

## Decision

The profile belongs to the conversation, not to one Chrome process. Each
Host keeps one profile per conversation, and every Chrome the conversation
starts there runs on it, so sign-ins, cookies and site storage carry over a
crash, the last tab closing, a release and a restart.

- **Tabs are not restored.** After Chrome stops, the next `tab new` opens a
  fresh tab, as today; only what the sites stored stays.
- **When the profile goes.** With the conversation's recordings: when the
  conversation is archived or deleted. A Host that was away then removes it
  as it connects. A conversation restored from the archive starts signed
  out.
- **Downloads and uploads** are files of one browser and are emptied when it
  stops, as today.
- **A crash loses recent changes.** Chrome writes cookies to disk about every
  30 seconds, and a stop through `Browser.close` writes them at once. A
  crash loses what changed since the last write, such as a sign-in
  completed seconds before it.

### Design

The profile is in the conversation's scratch directory, beside its
recordings, which have the same lifetime
(`crates/runner-jobs/src/scratch.rs`):

```text
<DEMI_HOME>/scratch/<conversation>/
  .recordings/
  .browser/              the conversation's Chrome profile
    profile.lock         held while a Chrome runs on it
    downloads/ uploads/  emptied when that Chrome stops
/tmp/demi-browser-a1B2c3/  the runtime directory, as today
```

```rust
/// The directory of `conversation`'s Chrome profile on this Host.
pub fn browser_profile_in(scratch_root: &Path, conversation: &str) -> PathBuf;
```

- `demi_command_protocol` names the directory, as it names the recordings'.
  The browser resident starts Chrome on it, and the runner removes it.
- An environment holds `profile.lock` from launch to retirement, so two
  residents never run a Chrome on one profile. An upgrade's previous release
  keeps a resident while it holds a conversation's browser, and the next
  release's resident serves the conversation's new commands. A start that
  finds the profile held asks the holder to hand it off by making
  `profile.handoff` in it. The holder looks for that file every 250 ms,
  retires its browser as a release would, and lets the profile go; its tabs
  end. The start waits up to 30 seconds, or until it is stopped, then fails
  as the browser being unavailable.
- A browser starts only after its resident's sweep of what an earlier
  resident left has ended. The lock of a resident that was killed is gone at
  once, but on macOS its Chrome, in a process group of its own, can still run
  on the profile until the sweep ends it.
- Retirement stops Chrome, empties `downloads/` and `uploads/`, and keeps
  the rest; a start empties them too, after a resident that ended without
  retiring. The sweep at a resident's start ends the Chrome processes its
  marker names and removes runtime directories as today, and never removes a
  conversation's profile. A drawing's Chrome keeps its temporary profile.
- Each start rewrites the capture extension in the profile, which may be
  another release's, and sets the language preference in the profile's
  preferences without replacing what the sites and Chrome stored there. The
  preferences are written to a file beside them and renamed over them, so a
  start that ends halfway leaves them as they were.
- `Scratch::forget_recordings` becomes `Scratch::forget_archived`, the
  removal of what an archive ends: the recordings and the browser profile. It
  runs after the residents released the conversation, as today. As a runner
  connects, it now releases the conversations the backend no longer names as
  held before it removes the scratch directories and archived files.
- A Chrome that crashed leaves its profile marked as not closed cleanly.
  Chrome runs headless with no startup window, so it shows no restore prompt
  and opens no earlier tab.

Nothing is stored on the backend, and no message changes. A profile that an
older release left in its temporary directory is swept as today.

## Spike

Whether a profile survives Chrome being killed with the switches Demi
starts it with (`--headless`, `--use-mock-keychain`,
`--password-store=basic`), on Chrome for Testing on macOS. A local page set
an `HttpOnly` cookie with `Max-Age` and a `localStorage` entry, Chrome was
killed with `SIGKILL`, and a new Chrome on the same profile read both back:

- The `localStorage` entry was there after a kill 12 seconds after the
  sign-in.
- The cookie was not there after a kill 12 seconds after the sign-in, and
  was there after a kill 40 seconds after it; the `Cookies` database held
  it only then. Chrome writes cookies in batches.
- The cookie was there after `Browser.close` 2 seconds after the sign-in,
  the close retirement makes before it ends the process tree.
- The new Chrome started at once on the profile of the killed one. Its
  stale singleton lock did not stop it.

## Prior art

- Playwright's `launchPersistentContext` runs a browser on a `userDataDir`
  that outlives it, and a later launch on the same directory starts signed
  in
  ([`class-browsertype.md`](https://github.com/microsoft/playwright/blob/deb7e86201e26cb4a8e25191fce5ef2f7a28aa87/docs/src/api/class-browsertype.md#async-method-browsertypelaunchpersistentcontext)).
- Browser Use starts each browser on a new temporary profile unless a
  `user_data_dir` is given, which then persists
  ([`profile.py`](https://github.com/browser-use/browser-use/blob/b49a68e4060acb226da8f784194fef301c53a6d7/browser_use/browser/profile.py#L557)).
  Demi gives each conversation its own directory, so sign-ins persist without
  one conversation reading another's.

## Alternatives

- **Save the storage state.** Write the cookies and storage out through CDP
  after each change and load them into a new profile, as Playwright's
  `storageState` does. It misses IndexedDB, service workers and the
  sites' other stores, and would be Demi's own copy of what the profile
  already holds.
- **One profile per Host.** Every conversation would share the sign-ins, and
  one conversation's agent would act in another's accounts.
- **Keep the profile until deletion only.** An archived conversation's
  profile, which can grow to hundreds of megabytes of cache, would stay on
  the Host until the conversation is deleted.

## Open questions

None.

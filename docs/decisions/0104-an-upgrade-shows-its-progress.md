---
status: proposed
---

# An upgrade shows its progress, and Demi opens once its data is ready

Supersedes: 0084 (A conversation keeps each content once), the part of
What an older server meets that converts each conversation in the
background after the start and lets an open convert one first.

## Problem

A server is upgraded to a release that converts its conversations, as
0.1.5447 converted 73 of them, 6.6 GB before, the largest 5.7 GB:

* The backend stops for about five seconds and starts again. A page that
  was open shows the spinner by the title (0059) and nothing else, and a
  page loaded then opens from its copy as if all were well.
* The backend reports ready once the control database has migrated, and
  the conversions start. The page's first read of the sidebar asks every
  conversation for its summary at once, and each one not converted yet
  converts for that read: 36 conversions started within five seconds, on
  a server of two cores. The read waits for all of them, so the sidebar, and
  each request behind it, hangs for minutes with nothing on screen to say
  why. A conversation whose agent was running waited 5 minutes for its own
  conversion; the 5.7 GB one took 21 minutes, against 65 to 94 s for the
  same file in 0084's probe.
* A page that stayed open through the upgrade can keep running the old
  build: the user's page did not show the new project menu until it was
  reloaded by hand.

## Decision

### What the user sees

From the backend's first answer that says it updates until it serves, every
page of Demi shows one screen in place of the app, as Home Assistant's does
while its database upgrades:

```text
                         Updating Demi

           ███████████████████░░░░░░░░░░░░░░░░░░░░░░
              Updating conversations: 41 of 73
          2.7 GB of 6.6 GB · About 3 minutes remaining

          Demi opens by itself when the update is done.
```

* The bar shows the bytes done of the databases to update. The first line
  under it names the step, *Updating accounts and settings* for the
  control database, then the conversations done of those to update. The
  second says the bytes done of all, and *Estimating time remaining…* for
  the first seconds, then the minutes left, rounded, or *Less than a
  minute remaining*, as macOS Software Update does.
* While the backend cannot be reached, as between its stop and its start,
  the bar moves and no time is shown.
* When the backend serves, the page loads itself, in the new build.
* A page opened during the update, a fresh one or one from its copy, shows
  the same screen at its first request.
* The seconds between the old backend's stop and the new one's first
  answer show the spinner by the title, as any restart does (0059): the
  new backend answers within seconds of its start.
* A shared conversation's page shows the same screen.
* The app under the screen takes no click, focus or key, so a draft's
  Enter or a shortcut does nothing unseen.
* The screen shows counts and sizes, never a conversation's name or an
  account's: the backend answers it before anyone signs in, so whoever
  reaches the address learns, while it updates, how many conversations the
  server holds and how large they are. A shared server's users see one
  screen.

`demi-server upgrade` prints the same progress while it waits for the
backend, `Updating conversations: 41 of 73 (2.7 GB of 6.6 GB), about 3
minutes left`.

### How it works

```text
demi-server   stop ─► switch ─► start backend ─────────────────► ready
new backend         listen ─► control ─► conversations ─► every route
page          spinner ─► Updating screen ───────────────────────► reload
```

1. The backend listens before it opens any database. Until it serves, it
   answers `GET /api/update`, serves the web app's files, and answers every
   other request, a runner's and a URL trigger's included, `503` with the
   code `updating` while it migrates or converts, or `backend_starting`
   when it has nothing to migrate. A page and a runner take both as a
   backend away for a moment, and a page that reads `updating` shows the
   screen. A shared conversation's page may ask `GET /api/update` too.
2. It measures the databases of an earlier form, migrates the control
   database, then gives every conversation database the current form: the
   conversion of 0084 and any migration in place. Only then does it start
   what reads conversations, such as the resumed turns, the schedules and
   the runners' links, and serve every route.
3. A conversation that fails to convert stays as it was and does not hold
   the server back; the log says why, and its next open tries again, as
   today. A conversation's file that cannot be read at all is left out of
   the update, with a line in the log, and the others serve.
4. `GET /api/update` keeps answering once the backend serves, `none`, so a
   page that shows the screen knows to load itself. Any other answer that
   does not say the backend is away, such as an earlier release's `401`
   after a return to it, loads the page too.

```rust
/// `GET /api/update`, answered without a session.
#[serde(tag = "state")]
pub enum UpdateState {
    /// It serves, or starts with nothing to migrate.
    None,
    Updating {
        step: UpdateStep,
        conversations_done: u32,
        /// The conversations whose databases update.
        conversations: u32,
        bytes_done: u64,
        /// The bytes of every database that updates, control's included.
        bytes: u64,
        /// None until a rate is known.
        left_seconds: Option<u32>,
    },
}

pub enum UpdateStep {
    /// The control database migrates.
    Control,
    /// The conversations' databases convert or migrate.
    Conversations,
}
```

### Faster conversions

* **One at a time, on every core.** The conversions run one file at a
  time, largest first. One thread reads a file's rows, one worker per core
  parses, splits, checks and compresses them, and one thread writes them in
  their order, a few hundred to a transaction. Nothing else opens a
  conversation meanwhile, so no conversion starts beside another.
* **The check stays, in memory.** Every row is still checked before it is
  written, by joining its record with the contents it names, held in
  memory, and comparing the document with the original's, and by
  decompressing its compressed body, instead of reading the row back from
  the new file and decoding it into a block.
* **The dictionary's sample is split on every core**, as the rows are,
  read one row at a time and each row once; only the records' bodies are
  kept for the training.
* **Memory stays bounded.** The rows read and not yet written hold at most
  64 MB of the old file's text, the rows the writer keeps while it waits
  for an earlier one included; a larger row goes through alone. Without
  the bound, a file of 1 MB rows held 3.3 GB at its peak, against 137 MB
  for the conversion before.
* A content two rows hold is compressed once: the first row to reach it
  claims it, and the writer finds it written for the next.

### What an older server meets

Nothing changes for a release that converts nothing: the backend serves a
moment after it listens. A page of a build before this one shows its
spinner through the update and reloads when it reads the new build, as
today.

## Spike

* **Where the 21 minutes went.** The server's log of the 0.1.5447 upgrade:
  73 conversions, 4,064 s of conversion time in 21 minutes, 36 of them
  started within five seconds by the sidebar's read
  (`conversation_summaries` asks each conversation's summary with
  `try_join_all`, and `ConversationDb::read` converts a file it finds in
  the old form). A 9,150-row conversation took 188 s there, and 2.4 to
  15 s alone on a developer's machine. The disk read 2.4 GB/s, so the
  conversions waited on the two cores.
* **Where a conversion's time went.** The release's own backend,
  converting a copy of a 61 MB, 9,150-row conversation alone on one core
  under `perf`: 61% the rows, of which reading each row back and decoding
  it is 24% of the whole, writing it 22% and parsing the old JSON into a
  block 10%; 25% the dictionary's pass.
* **What the change gives.** Optimized builds on a developer's machine,
  converting copies of three conversations of that server, each converted
  file read back row for row equal to the one the old conversion wrote:

  | Conversation | Before | 2 workers | All cores |
  | --- | --- | --- | --- |
  | 582 MB, 54,516 rows | 13.3 s | 4.2 s | 3.1 s |
  | 79 MB, 9,150 rows | 2.0 s | 1.0 s | 0.65 s |
  | 64 MB, 6,682 rows | 2.2 s | 0.84 s | 0.55 s |

  On a server of two cores the conversions take about a third of their
  time, and none of them runs beside another.

## Prior art

* **Home Assistant** runs a migration that cannot run live before it
  starts, and its frontend shows an init page in place of the app,
  *Database upgrade is in progress, Home Assistant will not start until
  the upgrade is completed*, retrying until the server answers
  ([`ha-init-page.ts`](https://github.com/home-assistant/frontend/blob/dev/src/layouts/ha-init-page.ts),
  [`recorder/core.py`](https://github.com/home-assistant/core/blob/dev/homeassistant/components/recorder/core.py)).
  A migration it can run live runs behind a notification instead. Demi
  takes the first: a conversion cannot serve a conversation it has not
  reached, and the second is the hang this record removes. Demi adds the
  share done and the time left.
* **Nextcloud** answers every page with *This instance is currently in
  maintenance mode … This page will refresh itself when the instance is
  available again* while it upgrades, and its web updater streams each
  step, `[3 / 12]: …`
  ([`update.user.php`](https://github.com/nextcloud/server/blob/master/core/templates/update.user.php),
  [`Updater.php`](https://github.com/nextcloud/server/blob/master/lib/private/Updater.php)).
  Demi's screen refreshes itself the same way and shows one bar, not the
  steps.

## Alternatives

* **Open at once, convert behind it.** The app opens as soon as the backend
  starts; a conversation not converted yet shows its own progress in place
  of its transcript and moves to the front of the queue when opened, and
  the sidebar lists it without its status until then. Demi is usable
  sooner, but the sidebar and every other reader of conversations need a
  state for "not converted yet", and a running agent's conversation still
  waits; the upgrade is rare and, once faster, short.
* **Convert ahead, before the switch.** `demi-server` converts each
  conversation while the old release still serves, and the switch
  converts again only those written since. The pause shrinks to the few
  conversations in use, but conversion code would run in two programs and
  against files that change under it. It can come later, with this screen
  unchanged.
* **Keep the background conversion and bound it.** A limit of one
  conversion per core stops the 36 at once, but a sidebar read still waits
  for every conversion before it, with nothing on screen.

## Open questions

None.

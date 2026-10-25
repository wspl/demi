---
status: proposed
---

# An update shows each step it takes, with its progress

Supersedes: 0106 (An administrator updates the server from Settings), the
part that shows an update as downloading, preparing and restarting; and 0104
(An upgrade shows its progress, and Demi opens once its data is ready), the
part that shows only the spinner by the title while no backend answers and
only the data's progress after.

## Problem

An administrator updates a server with five paired devices from Settings ›
Updates. The download takes 20 seconds and shows its bytes. Then the page
says *Preparing Demi 0.1.5500…* for 70 seconds and nothing else:

* In those 70 seconds the server unpacks the release, checks its
  configuration, installs the Cloud image, checks the databases and waits for
  every device to download the new runner. One laptop took almost a minute.
  The page says none of it, so the administrator cannot tell a slow laptop
  from a stuck update.
* `demi-server` knows each step and prints the devices' count, *3 of 5
  devices have the new runner*, but only to its journal, which the page never
  shows.
* Then every page shows *Updating Demi* (0104). Until the new backend
  answers, it shows only the spinner by the title, while the server stops
  its backend, saves every running Cloud, keeps a copy of the databases,
  switches and starts again. With twenty Clouds running, saving them takes
  minutes, and nobody can tell what the server does or whether it hangs.
* The confirmation's second sentence, how many agents are working, arrives
  after the dialog opened, so the dialog grows under the pointer.

## Decision

### What the user sees

From **Update** until the restart, the update's row lists every step the
update takes, in order, each with what it is doing and how far it is:

```text
┌──────────────────────────────────────────────────────────────────────┐
│ Updating to Demi 0.1.5500                       ███████████░░░░░░░░  │
│ Step 7 of 8 · Updating Demi on your devices · 1:12 until it goes on  │
│                                                                      │
│ ✓ Download Demi 0.1.5500                          214 MB in 21 s     │
│ ✓ Unpack the release                              310 MB in 4 s      │
│ ✓ Check the configuration                         Done               │
│ – Download gVisor                                 Already installed  │
│ ✓ Install the Cloud image                         96 MB in 9 s       │
│ ✓ Check the databases                             2 to update        │
│ ◌ Update Demi on your devices       3 of 5 · Waiting for alex’s Mac  │
│ ○ Restart Demi                                                       │
└──────────────────────────────────────────────────────────────────────┘
```

* The list has the same eight lines in every update, so it never grows. A
  step the update does not need says why, as *Already installed* or *No
  changes*.
* A done step says what it did and how long it took. The running step says
  its numbers: the bytes done of all, the rate and the time left for a
  download; the bytes for the unpacking; the time for the image; the devices done of
  all and which ones it still waits for, in one line that ends in *and 1
  more* where the names do not fit; the time before it goes on without them,
  as today after 2 minutes, shows in the status line.
* The bar at the top is the whole update: each step counts by the time it
  usually takes, and a running step with numbers moves it inside its share.
* A failed step is marked ✗ and says why, and the steps after it stay
  unmarked, until the next update.
* At *Restart Demi* every page shows the update's screen, below.
* Every line of the row and the list keeps its height in every state. The
  status row always has two lines, so *Checking for Updates…* keeps the
  version line under it. On a phone, where the controls go under the text,
  they keep one height in every state.
* **Update Now** counts the working agents before it asks, and the
  confirmation opens with its sentences complete.

### The update screen

From the moment the old backend is about to stop until the new one serves,
every page, an administrator's or not, shows one screen in place of the app,
with the rest of the update as a list of the same kind:

```text
                       Updating Demi to 0.1.5500

          ████████████████████████░░░░░░░░░░░░░░░░░░░░░░░░░

          ✓ Stop Demi                                    2 s
          ◌ Save the running Clouds                  7 of 20
          ○ Keep a copy of the databases
          ○ Switch to the new version
          ○ Start the Cloud manager
          ○ Start Demi
          ○ Update accounts and settings
          ○ Update conversations

            Demi opens by itself when the update is done.
```

* The steps, in order:
  1. **Stop Demi.** The backend ends every turn and closes every
     connection.
  2. **Save the running Clouds,** *7 of 20*. The manager stops and saves each
     Cloud that runs, so each boots into the new version.
  3. **Keep a copy of the databases,** *1.2 GB of 2.0 GB*, or *No changes*
     when the new version migrates none.
  4. **Switch to the new version.**
  5. **Start the Cloud manager.**
  6. **Start Demi.**
  7. **Update accounts and settings,** the control database, or *No
     changes*.
  8. **Update conversations,** *41 of 73 · 2.7 GB of 6.6 GB · About 3
     minutes remaining*, or *No changes*, as 0104 says.
* The bar is the whole of this part, as on the Settings row.
* The screen shows counts and sizes, never a Cloud's, a device's or a
  conversation's name, as 0104 says: anyone who reaches the address sees it.
* Between two steps the server may not answer for a moment, as while the
  new backend takes the address. The screen then keeps the last list it
  read and says nothing new, and it never empties or changes size.
* When the new version does not start, step 6 or 5 is marked ✗ with *Demi
  0.1.5500 did not start*, a line *Returning to 0.1.5469* follows, and the
  page reloads into the old version once it serves, whose Settings row says
  why.
* When a step after the stop fails, as a switch on a full disk, nothing
  serves: the step is marked ✗ with its reason, the title says *Couldn’t
  update Demi to 0.1.5500*, and the line below says *Demi is unavailable
  until an administrator finishes the update on the server*. The screen
  stays until the next `demi-server` command finishes the move.
* A page opened in the middle of the update shows the same list, the steps
  done marked done, from its first answer.

### How it works

The steps:

```rust
/// One step of an update while the server serves, in the order it runs.
#[serde(rename_all = "camelCase")]
pub enum UpgradeStepKind {
    Download,
    Unpack,
    CheckConfiguration,
    DownloadGvisor,
    InstallImage,
    CheckDatabases,
    UpdateDevices,
    Restart,
}

/// How far one step is.
#[serde(tag = "state", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum StepState {
    Waiting,
    Running { started: Timestamp, progress: Option<StepProgress> },
    Done { took_ms: u64, summary: StepSummary },
    Skipped { why: SkipReason },
    Failed { reason: String },
}

/// The numbers of a running step.
pub enum StepProgress {
    Bytes { done: u64, total: u64, per_second: Option<u64>, left_seconds: Option<u32> },
    Devices { done: u32, total: u32, waiting: Vec<String>, gives_up_at: Timestamp },
    Clouds { done: u32, total: u32 },
    Conversations { done: u32, total: u32, bytes_done: u64, bytes: u64, left_seconds: Option<u32> },
}

/// All of one update's steps, which `demi-server` keeps as it takes them.
pub struct UpdateRecord {
    to: String,
    from: String,
    upgrade: Vec<UpgradeStep>,
    restart: Vec<RestartStep>,
    returning: bool,
}
```

* `UpgradeRun` becomes one status: `None`, `Updating { to, steps }` or
  `Failed { to, reason, steps }`. `Downloading`, `Preparing` and
  `Restarting` go: they are steps of the list.
* `demi-server` tells the backend each step as it starts and ends, and the
  numbers of a running step at most once a second, over the management
  socket as today. It tells the whole record each time, so a backend that
  missed a notice has the full state with the next one. The notices become
  `Progress { update }` and `Failed { reason, update }`.
* The installed `demi-server` does the download and the unpacking; the new
  release's does the rest (`upgrades.md` § Fetch). The image's step shows
  the time it runs, and the image's size once done: the manager's import
  reports no progress, and a base it has already is *Already installed*.
* The devices' numbers come from the backend's answers to `demi-server`'s
  request to download the runner ahead, which say each device's state.
* **While no backend runs, the new release's backend answers the pages in
  its update screen's mode.** Between the old backend's stop and the new
  one's start, `demi-server` runs `demi-backend --update-screen <record>` as
  a transient systemd unit with the backend unit's user, capability and
  configuration. It opens no database: it listens on the backend's address
  (`DEMI_BACKEND_LISTEN`), answers `GET /api/update` with the steps, every
  other API request `503` with the code `updating`, and serves the web app's
  files from the new release, so a page loaded then opens on the screen.
  `demi-server` stops it just before the new backend starts, and the backend
  answers from then on, as 0104 says. The screen is the backend's own code,
  so no second HTTP server says what the backend says.
* **The Clouds' count.** The manager writes how many Clouds it has saved of
  those it saves, `{ "saved": 7, "saving": 20 }`, to `drain.json` in its
  runtime directory as its stop goes on; `demi-server` reads it.
* **The copy's bytes.** `demi-server` copies the databases itself, in
  chunks, and counts the bytes.
* **One file holds the steps.** `demi-server` writes the record to
  `update.json` in the backend's data directory at every step. The next
  `demi-server` of the move, after an interruption, goes on from it; the
  update screen reads it at every request; the new backend reads it as it
  starts, when its `to` is the backend's own version, takes *Start Demi* as
  done, adds its own steps, and removes the file once it serves.
* `UpdateState::Updating` carries `to` and the list in place of `step`:

  ```rust
  pub enum RestartStepKind {
      StopDemi,
      SaveClouds,
      KeepDatabases,
      Switch,
      StartManager,
      StartDemi,
      UpdateControl,
      UpdateConversations,
  }
  ```

  Each step has the state of a Settings step, `Waiting`, `Running`, `Done`,
  `Skipped` or `Failed`, with its numbers: Clouds, bytes, or conversations
  and bytes with the time left. `returning` says the new version did not
  start and the server goes back to `from`. `UpdateStep` and the
  `Restarting` state go.
* The first update to this version runs its preparation under a backend of
  an earlier one, which does not read the new notices: that update shows
  the earlier one's words until the stop, and the list from the update
  screen on.

## Spike

None: every step and its numbers exist in `demi-server` today, and the
devices' count exists in the backend. New are the manager's count of saved
Clouds, which its drain has as it saves them one by one, and the backend's
update screen's mode, its own listener without its storage, on an address
the stopped backend left free.

## Prior art

* [Home Assistant](https://github.com/home-assistant/core/blob/dev/homeassistant/components/update/__init__.py)
  reports an update's `update_percentage` while it installs, one number for
  the whole update.
* [Discourse's Docker manager](https://github.com/discourse/docker_manager)
  shows the upgrade's log as it runs, with a progress bar.
* GitHub Actions lists a job's steps with a mark and a duration each, the
  running one expanded.

Demi takes the step list with marks and durations, as GitHub Actions has it,
and keeps one bar for the whole, as Home Assistant has it, but not the log:
the log's lines are for an operator, and the list says what they say in the
user's words.

## Alternatives

* **More specific words in the one line,** *Waiting for 2 devices*. Better
  than today, but the user still cannot see what came before, what is next
  or how long each took.
* **The journal's lines on the page.** They say everything, in the operator's
  words, and they grow, which moves the page.
* **Systemd socket activation for the backend,** so the address stays open
  across the restart and requests wait in its queue. No error reaches the
  page, but nothing answers either: the screen would wait as blind as
  today.
* **A page that guesses the steps from the time.** It would claim progress
  the server has not made.

## Open questions

None.

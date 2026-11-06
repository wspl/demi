---
status: accepted, superseded in part by 0281
---

# A turn Demi resumes after a restart shows as running until it runs

## Problem

Alex's server updates while three of Alex's conversations run. Before the old
backend stops, it marks each running turn, and the new backend, ready a second
later, takes each mark and resumes that turn by itself. A turn runs again only
once its tree is restored, and the restore waits for the runner of the
conversation's device to come back and for the commands it left running to be
taken up: on a server with a paired device, 20 to 40 seconds.

Alex's page reconnects in those seconds and shows each of the three as failed:

- The sidebar's dot is red.
- The transcript ends with a red record, "The turn was interrupted before it
  finished."
- The dock offers Resume.

Then each turn runs again, the record goes, and the dot turns blue. Alex sees
an error that needs nothing from him, and may press a Resume that races the
one Demi is about to run.

## Decision

**From the moment the backend takes a turn's mark until that turn runs again,
the conversation shows as running, and its transcript says it is resuming.**

```text
Sidebar                           Conversation
 ● Write the release notes         …the agent's output before the restart
 ● Fix the import                  ◌ Resuming
                                  ┌──────────────────────────────────────┐
                                  │ Ask anything…                        │
                                  └──────────────────────────────────────┘
```

- The sidebar's dot is the running one.
- The transcript hides the interruption record, as after a Resume the user
  pressed, and its tail row says Resuming instead of Requesting, since what
  the user waits for is Demi, not the provider. The row has no timer: the page
  does not know when the wait began, and a reload would start it again. The
  dock offers no Resume. The composer stays as it is.
- When the turn runs, the page goes on as after a press: its request to the
  provider shows as Requesting.
- When the resume does not happen, the conversation shows what it shows today:
  the red record, the red dot, and Resume. This is when the resume fails, or
  when the turn was found to have ended or changed meanwhile, such as by a
  message the user sent.

A turn Demi does not resume by itself stays red from the start, since it waits
for the user: a crash marks nothing, and the `resumeAfterRestart` preference
off leaves each mark to the user.

The design to build from:

```rust
/// Where a conversation stands.
pub enum ConversationStatus {
    Running,
    Compacting,
    /// A turn a shutdown interrupted that Demi resumes by itself: from the
    /// start that took its mark until the turn runs again or the resume
    /// gives up.
    Resuming,
    Interrupted,
    Error,
    Stopped,
    Completed,
    Idle,
}
```

- The shard keeps the conversations it is resuming, in memory, in
  `restart_resume.rs`: a conversation enters when the start reads its mark with
  the preference on, and leaves when its resume settles, however it settles.
  Each entry and exit marks the conversation's summary changed.
- A summary's status is `resuming` while the conversation is in that set and
  its tree is not running. The set is the one owner of the fact, and nothing
  is stored, since the fact lasts no longer than the process.
- The page maps `resuming` to the sidebar's running state, and keeps whether
  the summary says `resuming` on the conversation as `resuming`.
- While it does and the root is idle, the transcript list takes the record
  as a pressed Resume takes it, the tail row's kind is `resuming` beside
  `requesting`, and the dock offers no Resume. When the turn runs, the row
  rolls from Resuming to Requesting: the lane tells the root's phase as it
  changes, and the summary is read again only after its change is announced.
- The row's words, by the dock's Resume in each language:

  | Language | Resume | The row |
  | --- | --- | --- |
  | en | Resume | Resuming |
  | de | Fortsetzen | Wird fortgesetzt |
  | es-419 | Reanudar | Reanudando |
  | fr | Reprendre | Reprise en cours |
  | ja | 再開 | 再開中 |
  | ko | 재개 | 재개 중 |
  | pt-BR | Retomar | Retomando |
  | zh-Hans | 恢复 | 正在恢复 |
  | zh-Hant | 恢復 | 恢復中 |
- An older page reads `resuming` as idle, a grey dot, until it reloads to the
  new version, as every page does after an update.

## Spike

The restart of a backend on a development machine, with two conversations
streaming from a stand-in model, resumed both within a second, so the page
never showed the red state. With the preference off, the same restart left
both on the red record, the red dot and Resume, which is what a server's
page shows for the 20 to 40 seconds before its resumes run. A server's log of
one update showed five turns marked, the backend ready 9 seconds later, and
the five resumed 24 to 37 seconds after that, each after its runner's
commands were taken up.

## Prior art

- [JupyterLab](https://jupyterlab.readthedocs.io/en/latest/api/types/services.Kernel.Status.html)
  tells a kernel the server restarts by itself, `autorestarting`, from one
  that will not be restarted, `dead`, and shows only the second as a failure.
  Demi draws the same line between a turn it resumes and one that waits for
  the user.

## Alternatives

- **Resume before the runner's commands are taken up.** The turn would run
  with commands the agent cannot see, and the wait protects what they print.
- **Infer the state on the page from a running status, an idle root and an
  error at the end.** A root whose turn failed while one of its commands still
  runs looks the same, and its error would be hidden.
- **Keep the red record and add a note that it resumes soon.** The user still
  reads an error first, and Resume stays offered.

## Open questions

None.

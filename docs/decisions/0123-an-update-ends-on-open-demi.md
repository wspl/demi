---
status: proposed
---

# An update ends on Open Demi

Supersedes: 0104 (An upgrade shows its progress, and Demi opens once its data
is ready) and 0111 (An update shows each step it takes, with its progress),
the parts that load the page by itself once the new backend serves.

## Problem

Alex watches the update screen while the server updates to 0.1.5500. *Start
Demi* is still running when the page loads itself into the new version:
*Update accounts and settings* and *Update conversations* never show as
done, and Alex cannot tell whether they ran, how long they took, or whether
the update finished at all. Two things load the page:

- The screen asks `GET /api/update` every two seconds, and the new backend
  answers `none` as soon as it serves: the steps it took are gone, and the
  screen loads the page at that answer.
- The app under the screen reconnects meanwhile, reads the state of the new
  backend, finds a new web build, and loads the page itself.

## Decision

The screen shows the whole update done and waits for the user to open Demi,
as an installer ends on a button rather than closing itself.

```text
                     Updated Demi to 0.1.5500

        ██████████████████████████████████████████████████

        ✓ Stop Demi                                    2 s
        ✓ Save the running Clouds                     1:40
        ✓ Keep a copy of the databases        2.0 GB · 14 s
        ✓ Switch to the new version                    0 s
        ✓ Start the Cloud manager                      3 s
        ✓ Start Demi                                   4 s
        – Update accounts and settings   No changes needed
        ✓ Update conversations                        2:10

                           [ Open Demi ]
```

- **Done.** Once the new backend serves, every step shows done, the bar is
  full and the title says *Updated Demi to 0.1.5500*. **Open Demi** loads the
  page in the new version.
- **Returned.** When the new version did not start and the server runs the
  version it left again, the list keeps the failed step, the title says
  *Couldn’t update Demi to 0.1.5500*, the line below says *Demi 0.1.5469 is
  running again, with its data as it was.*, and **Open Demi** loads the page
  in that version.
- **While it runs**, the line says *Open Demi when the update is done.* and
  the button's room stays empty; the line and the button keep their place in
  every state, so nothing moves when the update ends.
- Nothing loads the page by itself while the screen shows: the app under it
  takes no part, its own load on a new build included.
- A page opened after the update opens Demi, with no screen, as today.

### How it works

- `UpdateState::None`, the answer of a backend that serves, says which
  release it runs and, after the update or the migration it started from,
  every step of it done, with what each did:

  ```rust
  pub enum UpdateState {
      None {
          /// The release the backend runs.
          version: String,
          /// Every step of the update this backend started from, done; none
          /// for a start that updated nothing, or a backend started again
          /// since.
          updated: Option<Vec<RestartStep>>,
      },
      Updating { to: String, from: Option<String>, returning: bool, steps: Vec<RestartStep> },
  }
  ```

- The backend keeps the record and its own steps' results in memory once it
  serves, until it stops.
- The screen takes the update as done when the backend runs the release the
  update moved to, and as returned otherwise, whether or not it heard the
  return. A backend that started again since names no steps: the screen then
  shows the steps it last saw unfinished as done, with no numbers. After a
  return, the step it last saw running shows as failed.
- A backend of an earlier release, after a return to it, answers the route
  `401` or `404`, which the screen reads as returned.
- The page's own load on a new build waits while the update screen shows;
  **Open Demi** loads the page.

### What an older backend or page meets

A page of an earlier build reads the answer of a backend that serves as it
did, `none`, since it ignores the fields it does not know, and loads itself.

## Spike

None: the record and the backend's own steps exist; only how long they are
answered changes.

## Prior art

- macOS's installer and Windows Setup end on a summary with a button
  (**Close**, **Finish**) instead of leaving by themselves.
- [Home Assistant](https://github.com/home-assistant/frontend/blob/dev/src/layouts/ha-init-page.ts)
  reloads by itself once its database upgrade is done; Demi departs from it
  so the user sees that every step finished.

## Alternatives

- **Hold the done list for a few seconds, then load.** A user who looks away
  for those seconds still misses it, and one who reads it waits for a timer.
- **Mark the remaining steps done on the page alone.** It would show *done*
  without what each step did or how long it took.

## Open questions

None.

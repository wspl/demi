---
status: proposed
---

# Catching up spins at the transcript's end

Supersedes: 0059 (Connecting is a spinner in place of the title's Rename
button), the spinner's place by the title and what it waits for. The tail
row's Connecting and the connection banner stay removed.

## Problem

Alex switches back to a conversation that kept working while it was not
shown. The page shows its copy on disk at once, and the conversation looks
finished: the last reply, its time, an idle composer. A second later the
socket opens, the page reads what changed, and three commands and a new
reply appear under it. Nothing said that the page was behind.

The title's Rename button turns into a spinner, but only when the socket has
not opened for two seconds, and the read after the socket opens, which is
where the new blocks come from, shows nothing at all. Where it does show,
the spinner sits by the title, far from where the new blocks appear, and
0059 says itself that it is easy to miss.

## Decision

While the page is behind the conversation's session, a spinner stands
centered over the transcript's bottom edge, right above the dock: where the
blocks it waits for will appear.

```text
  │ The fix is in, and the tests pass.        │
  │ ⧉ ⑂ ⇪  3 minutes ago                      │
  │                                           │
  │                    ◌                      │
  │ [ 2 Services ]                  346 tok/s │
  │ ╭───────────────────────────────────────╮ │
  │ │ Ask anything…                         │ │
```

* **What it waits for**: the page shows the conversation but does not hold
  what its session says now. Its socket is not open, at an opening or after
  it was lost, or the socket is open and the root's transcript is still
  being read again. Once both are current, it goes.
* **Only waits that last**: a wait of less than half a second, as at most
  openings of a conversation the page keeps connected, shows nothing.
* **Nothing moves**: it is drawn over the transcript, not in it, so no row is
  added and nothing scrolls. It stands above the dock's top part, whichever
  that is: a card waiting for a decision, the chips row or the composer. The
  transcript keeps its band clear above that part whether it shows or not,
  so it never covers the last line; under the dock veil the band is there
  already, and over a card taller than the veil the transcript ends that
  much higher.
* **Seen wherever the transcript is scrolled**: it stays at the bottom edge,
  so a reader scrolled up into the history sees it too.
* **The one sign**: the title's Rename button, and on a phone the title's
  chevron, stay as they are; nothing by the title spins.
* **Not for what does not connect**: an archived conversation, a share and a
  pane that shows Loading or Failed in place of the transcript show none.
  The Empty pane shows it: No messages may be out of date, as for a
  conversation sent to from another device while the page was behind.

The design to build from:

* `session-status.ts` owns the rule, `behindSession(load, catchingUp)`:
  the transcript or its Empty pane shows, and its socket is not open or
  `catchingUp`. `ChatSession` reads it with a `catchingUp` prop that the
  page fills from `!runtime.caughtUp`, and holds it for
  `CATCH_UP_WAIT_MS = 500` with the existing `useHeldFor`; a specimen that
  composes its own dock reads the same rule. `SOCKET_WAIT_MS` and the two
  title spinners go.
* `SessionSurface` adds the band, `catchUpBand(spinnerPx)`, the dock's
  8 px step and the spinner, to what the transcript keeps clear above the
  dock and what stands over it.
* The spinner is `IndeterminateSpinner` at the transcript's icon size,
  `role="status"`, labeled with the existing "Loading conversation…".
  `session.connecting` is no longer used and leaves every locale.
* `SessionDock` places it: absolutely, centered over the transcript column,
  its bottom a step above the dock's top part, over the dock veil so it
  reads at full strength. The scroll control keeps its place at the chips
  row's end, so the two never meet.
* The gallery's session specimens gain a catching-up state that holds the
  spinner, and the phone scene whose socket is lost for six seconds shows it
  above its dock instead of by its title.

No protocol, stored data or backend change: the page already knows both
facts (`conversation.load` and `ConversationRuntime.caughtUp`).

## Spike

* **Room at the transcript's end.** Measured in the gallery's session
  specimen: the transcript ends `COMPOSER_CLEARANCE_PX`, 16 px, above the
  dock veil, and the last row's content reaches its row's bottom. A spinner
  in that band would touch the last row; one lower in the band would be
  faded by the veil. So the spinner is drawn by the dock, over the veil,
  not inside the list.
* **How long catching up takes.** In a server's page log, a conversation
  the page had let go showed its copy 44 ms after it was opened; its read
  answered 2.5 s later and its socket opened at 3.4 s. Opened again a
  minute later, its socket answered after 1 s. A conversation the page
  keeps connected (0090) is current at once, so half a second shows the
  spinner for the first kind and never for the second.

## Prior art

* Element Web, the Matrix client, shows a spinner at the end of the
  timeline while it fetches newer events (`MessagePanel`'s bottom spinner,
  [element-web](https://github.com/element-hq/element-web)). It adds the
  spinner as a list item, which moves the rows; Demi draws it over the edge,
  as 0059's motivation requires.
* Telegram says Updating in the chat's title bar, as 0059 did, and the user
  found that spot easy to miss.

## Alternatives

* **Keep it by the title, sooner**: the same spot the user misses.
* **A row at the transcript's end**: 0059's reason against it holds; the
  row comes and goes and every row moves by its height.
* **Inside the end clearance**: 16 px leaves no step between the spinner
  and the last row.
* **Words, such as Updating…**: more to read for a state that passes on its
  own, as 0059 says.

## Open questions

None.

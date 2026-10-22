---
status: accepted
---

# A transcript moves only for new content, and smoothly

## Summary

While a conversation's transcript follows its end, the rows on screen move
only when content is added, removed or grows, and then they slide to their
places over 200 ms instead of jumping in one frame. What a row shows and how
tall it is no longer depend on the session's phase or on the kind of row that
held its place before.

## Motivation

A user watches a conversation while the agent works. Nothing new is said,
yet the transcript jumps:

* When a message is sent, the last reply's footer disappears and comes back,
  moving every row by 33 px twice.
* When a command gives way to thinking, the last row shrinks by 8 px: the
  thinking row is estimated at 20 px and measures 28 px.
* When a reply ends, its footer appears only once the session is idle, and
  pushes every row up.
* A new row pushes the rows above it up in a single frame.

Each comes from a state change that is not new content: a footer shown by the
session's phase; a row that takes over another row's key, with that row's
size and open state; the tail's row removed and another added at a different
height. The list, following the bottom, then moves everything at once.

## Guide-level explanation

The user sends a message. The bubble slides in at the bottom, and the reply
above it slides up by the bubble's height; the reply keeps its footer.

```text
  before:  [reply + footer] -> [reply] -> [reply + footer]   jumps twice
           [bubble]            [bubble]    [bubble]
  after:   [reply + footer] -----------------------------> unchanged
           [bubble slides in, the reply above slides up]
```

The agent then requests, runs a command and thinks: the last row says
Requesting, then the command, then Thinking, and keeps its height while it
does. When the reply's text is complete, its footer is there; text still
streaming when the stream ends shows the rest at the pace it was showing.

## Reference-level explanation

This changes only `web-ui`; no route, frame or stored value changes.

```ts
/** What decides which blocks end their reply (`assistant-footer.ts`). */
interface ReplyEndInput {
  phase: SessionPhase
  /** The server says it waits for the model, or a call is being written. */
  continuing: boolean
  /** A message being sent is the user's next message from the moment it is sent. */
  sending?: boolean
}

/** How long rows take to move to their places (`row-motion.ts`). */
const ROW_MOVE_MS = 200

/** Where each drawn row stood on screen, by key, and the scroller's offset. */
interface RowsBefore {
  tops: Map<string, number>
  scrollTop: number
}

/** The rows' painted positions now, a running move included. */
function rowsBefore(scroller: HTMLElement): RowsBefore
/** Moves each row from where it stood in `before` to where it is now. */
function moveRows(scroller: HTMLElement, before: RowsBefore): void
```

* **Footer.** A reply's footer shows from the moment the backend marks the
  reply's text complete (`forkable`) and the agent does not go on: the
  server does not say it waits for the model, and no call is being written.
  It stays when the next message is sent.
* **Tail row.** The row that says what the agent is doing, Requesting, a call
  or thinking, keeps its height while what it says changes.
* **Size.** A row's size is the size it measured once drawn, never an
  estimate by its kind; a row that takes another's key keeps its own
  measurement.
* **Open state.** A row the user opened stays open as the block it shows,
  named by the block, not by the key it is drawn under (`open-runs.ts`).
* **Move.** When rows change place, each slides from where it stood to where
  it now is over `ROW_MOVE_MS` = 200 ms (`row-motion.ts`):
  * A move starts from the position last painted on screen, never from a
    position that was computed but not drawn; several layout updates within
    one frame start from the first.
  * A row whose own height is animating, such as a fold, gets no move of its
    own; the rows below it follow it frame by frame.
  * A row that grows by new content, such as streamed text, moves the rows
    below it in the same frame: they stay attached to it and never slide, so
    no row ever overlaps another. Rows slide only when a row is added or
    removed, and when the list scrolls to follow its end.
  * A move never enlarges the scrollable area: the rows' container clips
    vertically, so a sliding row cannot scroll the list.
  * A move still running is replaced by one from where its row stands; a move
    longer than the view is not animated; with reduced motion nothing slides.
* **Stream end.** Text not yet shown when its stream ends shows at the pace
  it was showing (`useStreamReveal.ts`).

## Upgrade and version skew

None: the change is in the page alone, and an open page of the earlier build
keeps its behavior until it reloads.

## Performance

* One transform animation per row that moves, on the compositor; no row is
  laid out more often than before.
* Reading the painted positions costs one `getBoundingClientRect` per drawn
  row per layout update.

## Drawbacks

* A row added while another still slides in shortens the first slide.

## Rationale and alternatives

* Fix each jump where it happens: each fix keeps the cause, a row's look
  derived from something other than its content, and the next state change
  jumps again.
* Animate the scroll position instead of the rows: the rows that do not move
  relative to each other would still jump against the bubble coming in.

## Prior art

* Telegram's and iMessage's transcripts slide earlier messages up as a new
  one comes in, and keep each message's look fixed once drawn.
* The FLIP technique (First, Last, Invert, Play), described by Paul Lewis
  (aerotwist.com, "FLIP Your Animations"), animates rows from their painted
  positions to their new ones; Vue's `TransitionGroup` uses it for its move
  transitions (vuejs.org, "Move Transitions"). This design applies it to a
  virtualized list, whose rows are positioned by the virtualizer rather than
  by the layout.

## Unresolved questions

None.

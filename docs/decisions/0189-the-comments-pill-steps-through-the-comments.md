---
status: proposed
---

# The comments pill steps through the comments

Supersedes: 0092 (A comment and a side question stand under the line they are on) and 0096 (An edit of a
sent message shows and changes its comments), the list a click on the
*2 Comments* pill opens.

## Problem

Alex leaves three comments on a long reply, then scrolls down to the
composer to write the message they go with. To check what the second
comment says, Alex clicks *3 Comments*. A list opens over the transcript
with each quote and comment, a second copy of the boxes that already stand
under the reply's lines, with the same Edit and Delete. The list shows the
words cut from their context, and to see the comment where it stands Alex
closes the list and scrolls up to find it.

## Decision

**A click on the pill brings the next comment into view and marks it.**
The first click goes to the first comment in reading order, the order the
conversation reads them (0092), the next click to the second, and a click
after the last goes back to the first. The transcript scrolls the
comment's box into view, and the box flashes the mark a found row flashes
(`found-highlight`, as a search result and a receipt's block show), which
fades within two seconds. Nothing opens over the transcript.

```text
  …the retry waits 30 seconds after the first failure…
  ┌────────────────────────────────────────────────────────┐ ← marked
  │ 💬 Is this the same timeout as the upload's?    🗑 ✎  │
  └────────────────────────────────────────────────────────┘
  …

  ( 📎 ) [ 💬 3 Comments  × ]  Ask anything…              ( ↑ )
           └ click: the next comment
```

- **Edit and Delete** happen on the box under the reply's line, which has
  them already. The pill's × still removes every waiting comment.
- **The next one.** The pill remembers the last comment it showed: the
  next click shows the one after it in reading order. A comment added or
  deleted meanwhile takes its place in that order; when the last one shown
  is gone, the next click starts from the first.
- **A comment whose reply the page does not hold**, on an earlier page of a
  long conversation, is read in first, as a search result's block is, then
  shown and marked.
- **The tooltip** says *Next Comment*; the pill keeps its label.
- **An edit of a sent message** (0096): the pill counts the edited
  message's comments and steps through their boxes, which stay under their
  lines marked Sent. While the edit is open, those boxes offer Delete,
  which takes the comment out of the edit: its box leaves the reply until
  the edit ends, and comes back when the edit is canceled. While the edit is
  sent the boxes offer nothing, as the pill offers no × then.
- **A phone** does the same: the pill's tap scrolls to the next comment,
  and its sheet is gone.
- **A reply that leaves the conversation.** Regenerate, an edit of a sent
  message and a switch to another version replace the replies after the
  message. The comments waiting on those replies go with them once the
  backend has taken the change, and a toast says *2 Comments Removed with
  Their Replies*. Their quotes are no longer on screen, the pill could not
  show them, and the model would read words from an answer it no longer
  holds. A comment on a reply the page does not hold, far back in a long
  conversation, stays: where that reply stands is not known.
- **The mark** fills the comment's box with its own rounded corners.

### Design

- `web-ui`: `WaitingComments.vue` keeps the pill alone; its popover and
  its use of `CommentItem` go. It emits `next` with the id of the comment to
  show, which it chooses from its `comments` (already in reading order) and
  the id it showed last.
- `ChatSession.vue` provides a comment revealer that the pill injects
  through the composer slot, as the origin revealer is provided. It asks
  `AgentMessageList` to show the comment: the list brings the block's row
  into view with its existing `revealAndMark` path and then flashes the
  comment's box (`highlightFound` on the box's place, with scrolling, since
  the box may lie below the row's top). A block the list does not hold goes
  through the existing `reveal` event to the host, which reads the window
  around it; the box is flashed once it appears.
- `ReplyComments.vue` draws the boxes of an edited message's comments from
  the edit while it is open, so a comment deleted from the edit has no box,
  and gives them Delete while the edit is editable. A sent comment's box
  keeps Delete's room while no edit is open, so the button coming and going
  moves nothing.
- `commentsReplaced` in `comments.ts` names the waiting comments on the
  replies after a message by their place on the path in the windows the page
  holds; the store drops them after a Regenerate, an edit or a version switch
  succeeds.
- `found-highlight` takes the element's own radius from `--found-radius`,
  which the comment's box sets.
- Nothing stored or sent changes.

## Spike

None: the reveal of a block, the read of a block outside the window and the
mark all exist and are used by search results and receipts.

## Prior art

- **Chrome's find bar**: Return goes to the next match, wraps from the last
  to the first, and marks the current one in place.
- **VS Code** Go to Next Problem (F8) moves to the next problem in the
  file, wrapping at the end, and shows it where it stands.
- **Microsoft Word** Review › Next Comment and **Google Docs**' next-comment
  shortcut move to the next comment in the document and select it in
  place.

## Alternatives

- **Keep the list, and let a click on an item scroll to its comment**, as
  Google Docs' comment panel does. The list repeats the boxes the reply
  shows, with the same actions, and covers the transcript the user is about
  to look at.
- **A counter with up and down arrows** (*2 of 3*), as Chrome's find bar
  has. It needs a second control and a state that only matters while
  stepping; one click per comment covers the case of a handful of comments.

## Open questions

None.

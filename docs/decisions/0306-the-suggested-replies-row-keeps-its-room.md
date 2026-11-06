---
status: accepted
---

# The suggested replies row keeps its room under every resting answer

Supersedes: 0182 (A reply offers the user's likely next messages), the rule
that a page opening a conversation already resting shows the row only when
suggestions are kept for its last answer.

## Problem

An agent finishes a turn in a conversation the user is not looking at. The
page holds the conversation while the turn runs, writes its copy on disk,
and lets it go once the turn ends. The small model writes the suggestions a
few seconds later, after the page let go, so the copy holds the answer but no
suggestions for it.

The user switches to the conversation. The page shows the copy: the answer
with its footer and nothing under it. A moment later the conversation's
session tells the page that suggestions are kept for that answer, the row
appears, and everything above it moves up by the row's height, about 36 px.
Nothing in the conversation changed, yet the transcript jumps after it is
shown. Agents that work in the background end turns all the time, so the jump
comes with almost every switch.

A copy can fall behind its suggestions in other ways too: another device
showed the turn end, or the page closed while they were written.

## Decision

While the conversation rests after an answer and Suggested Replies is on,
the answer's row stands, with one line's room, whatever the page holds of
its suggestions. The row is filled with pills once they are known; when
none are known, or none came, it stays empty. Nothing under or above the
answer moves when they arrive, wherever the page learns of them.

A row that is never filled stays empty for good: under an answer from
before suggested replies, and under one whose suggestions request failed,
which keeps none. The page cannot tell either from suggestions still being
written, and an empty row looks the same as one where none came.

```text
  Merged and pushed; the branch is deleted.
  [copy] [fork] [share] [regenerate]   2 min ago
  (one line's room, empty until the pills arrive)
  ───────────────────────────────────────────────
  [ Ask anything...                           ]
```

The rest of 0182 holds: the row belongs to the last answer only, goes as
the next turn starts, and does not stand when Suggested Replies is off, in
an archived conversation, on a shared page or in a subagent.

`ChatSession`'s `suggested` gives the last answer `replies: null`, the state
"not known yet", whenever the kept suggestions name another block or none
are kept; the page no longer needs to have seen the turn end. Nothing is
stored or sent differently; an older page keeps the old rule until it
reloads.

## Spike

The jump was measured on a page that let a conversation go before its
suggestions arrived: on the switch back, the last row was 2340 px tall at
mount and 2376 px 37 ms later, its text unchanged, the 36 px being the row's
room. With the row standing at mount, the switch moves nothing.

## Prior art

- [Open WebUI](https://docs.openwebui.com/features/chat-conversations/chat-features/follow-up-prompts)
  shows its follow-up prompts under the answer once they are generated.
  Demi keeps their room from the start instead, as its rule that a change of
  state never moves the layout asks.

## Alternatives

- Keep a conversation open until its suggestions arrive, so its copy holds
  them: fixes only the copy of a page that saw the turn end, and still jumps
  after another device ended it or the page closed meanwhile.
- Draw the row only once the session is connected: the answer still moves
  when the row appears, only later.

## Open questions

None.

---
status: accepted
---

# A transcript is shown from a bounded window, around any message, local copy first

## Context and Problem Statement

[Agent sessions, and one way to keep their data in sync](0009-agent-sessions-and-sync.md)
keeps each agent's transcript as runs with holes between them, on the page
and on disk, and Demi already follows Telegram's clients in most of how it
reads, merges and validates them. Compared with Telegram's iOS and Android
clients item by item, four things differ:

- **Memory grows without bound.** Every conversation opened since the page
  loaded keeps its connection and every run of every transcript it read in
  memory, until the conversation leaves the list. Scrolling a long transcript
  to its start keeps all of it. In Telegram a history view holds a window
  around its anchor, at most a fixed number of messages on each side, drops
  what falls outside it, and lets it go when the chat closes; the local
  database keeps the rest.
- **A subagent's transcript cannot open at a message.** A search result in
  an early part of a long subagent's transcript reads its latest page, then
  one page before another until it reaches the message. Only the main
  agent's transcript reads around a message. Telegram opens any history
  around any message.
- **A conversation opened cold shows Loading although its copy is on disk.**
  The page clears the conversation, shows Loading, then reads the copy and
  shows it. Telegram shows the local copy as the first frame, and Loading
  only when it has nothing to show.
- **A read nobody needs any more runs on.** A page of history the list asked
  for keeps loading after the user scrolled elsewhere or switched
  conversations. Telegram cancels the read of a hole no view needs, after a
  short delay so a scroll back does not start it again.

## Considered Options

* Follow Telegram's clients on all four: a bounded window per transcript
  shown, conversations not shown let go, reads around any message, the local
  copy first, and reads cancelled once no view needs them.
* Bound memory by a total size across conversations, evicting the oldest
  runs.
* Keep holding everything, and rely on the browser's memory.

## Decision Outcome

Chosen option: follow Telegram's clients, because a window per view bounds
memory by what is on screen, however many conversations or blocks there are,
and the copy on disk already holds the rest. A total-size budget would still
hold blocks no view shows, and would evict a transcript on screen to keep one
that is not.

* **A window per transcript shown.** For the main agent, each subagent and
  each side question the list shows, the page holds the blocks around what is
  on screen, up to a limit on each side, and the latest blocks, which the live
  session keeps changing and the conversation's status reads, as Telegram's
  chat list keeps each chat's last message. A block that falls outside both
  leaves memory; the copy on disk keeps it, and scrolling back reads it from
  there first. A change to a block the page does not hold is not held, as
  today.
* **A conversation not shown is let go.** Once a conversation is no longer
  shown, and is not among the few shown most recently, the page closes its
  connection, drops its transcripts from memory and stops writing its copy.
  Opening it again shows the copy, then reconnects from the edges the copy
  holds. The list still shows its status, which comes from the list's own
  channel.
* **Any transcript opens around any message.** A subagent's or a side
  question's transcript opens at a message the way the main agent's does: it
  reads the page around the message and shows it there, and joins it with the
  other parts it holds as reads reach them.
* **The local copy first.** Opening a conversation keeps what the page shows
  until the copy on disk is read, then shows the copy at once, and the live
  session replaces it as it opens. Loading shows only when there is no copy.
* **A read nobody needs is cancelled.** A read for a part of a transcript is
  cancelled shortly after no view needs it any more, such as after a scroll
  away or a switch to another conversation; a view that needs it again before
  then keeps the read under way.

```text
 opening a conversation the page read yesterday

 before                          after
 ┌─────────────────────────┐     ┌─────────────────────────┐
 │                         │     │ Fix the flaky test      │
 │        Loading…         │     │ I found the race in …   │  copy on disk,
 │                         │     │ …                       │  at once
 └─────────────────────────┘     └─────────────────────────┘
 then the copy, then the          then the live session's
 live session                     changes, in place
```

### Consequences

* Good, because the page's memory follows what is on screen, not how long
  the page has been open or how far the user scrolled.
* Good, because a search result or a link opens any transcript at its message
  in one read.
* Good, because a conversation read before opens with its content, without a
  Loading frame.
* Bad, because a conversation the page let go reconnects when shown again,
  and shows a turn that ran meanwhile only once its session's changes arrive;
  the few shown most recently stay connected for quick switching.
* Bad, because scrolling back past the window reads from disk, or from the
  backend for a part the copy does not hold, where today the page holds it.

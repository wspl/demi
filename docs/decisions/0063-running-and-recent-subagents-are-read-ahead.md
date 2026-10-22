---
status: proposed
---

# The subagents a user is likely to open next are read and followed before they are chosen

Supersedes, in part: 0009, where a page follows only the nodes it shows and
ends a node's follow when its tab closes; 0055, where what only a closed view
needs is read when it opens; and 0044, whose estimate of the page's memory
counts eight closed subagent windows kept.

## Problem

A user opens a conversation whose agent has spawned five subagents, and
chooses one in the Agents window. The window shows a spinner while the page
reads that subagent's latest page, up to a thousand blocks and about a
megabyte, and only then draws it. Each first choice of a subagent waits
for its own request, though the page knew the subagent was there since the
conversation opened.

## Decision

The page reads the subagents a user is likely to open next as soon as the
root's view is drawn, and follows each on the conversation's socket, as it
follows the root. Choosing one of them shows it at once, from what the page
holds:
its latest page, its request's output and its cache chart, which come in
the one read of a node (0055).

- **Which:** every running subagent on the current path, and every one that
  ended since the user's last message, failed, stopped or finished, since
  its report is what the user opens it for. A conversation with a thousand
  subagents reads the few since its last message. One that ended before it,
  picked from Completed, is read when chosen, as today, and followed while
  it shows.
- **Since the user's last message:** the time of the root's latest user
  block, which the page keeps once it has held it, though its window later
  drops the block; only a newer user block replaces it. A turn the agent
  was woken for has no user block and belongs to the message before it. A
  page that opens in the middle of a turn longer than its window takes the
  time of the first block it holds.
- **When:** once the conversation has opened (its socket open or
  reopening), after the root's latest page is drawn, so the root never waits
  for them; each shows its copy on disk first, then is read, one at a time,
  running ones first, then the rest by when they ended, newest first, as the
  Agents window orders them. A subagent that starts later is read and
  followed when it starts.
- **Choosing** a subagent changes which one the window shows; one not read
  yet goes to the front of the reads, behind the one in flight. The one the
  window shows is read and followed whatever the conversation's state, while
  its socket connects or after an opening failed.
- **Leaving the set**, its turn past and its window not showing it, a
  subagent is unfollowed, its read in flight is cancelled, and its
  transcript, failures and chart are let go. An opening that fails empties
  the set but keeps the subagent the window shows; the set is read again
  once the conversation opens.
- **A read that fails** is tried again, from one second doubling to thirty,
  as any node's read, without holding the reads behind it; the window shows
  it loading meanwhile.

```ts
/**
 * The subagents read ahead: running ones on the current path, and those
 * that ended since the user's last message, in the Agents window's order.
 * Derived from the agents' entries; never stored.
 */
function readAheadSubagents(conversation: Conversation): string[]

/**
 * The nodes the conversation's socket follows besides the root: the
 * subagents read ahead, the one the window shows, and every listed side
 * question not folded. Derived; never stored.
 */
function followedNodes(conversation: Conversation): string[]

interface Conversation {
  /** The subagent the Agents window shows; null when it shows none. */
  shownSubagent: string | null // replaces shownSubagents
}

// conversation-client: a follow answers once the node's first read has
// answered ('read'), or once it was tried, answered or failed ('attempt'),
// which the reads ahead wait for, so a failure never holds the next.
followNode(node: string, until: 'read' | 'attempt'): Promise<void>
```

The side questions are read with the root's opening, as today; the
subagents only after it, so they never compete with the root's first
screen.

Nothing changes on the server or the wire: a page follows a node by the
`Follow` frame it already sends for a chosen subagent, and reads it by
`GET /conversations/:id/agents/:node/transcript?latest`.

**Cost.** Each subagent read ahead keeps its latest page in the page's memory
while it is read ahead, up to about a megabyte for a large one, so five large
ones take about 5 MB more, with no fixed count: a turn that spawns thirty
reads thirty. Each of the three conversations the page keeps open reads its
own, and every page read is also copied to the page's disk for opening
offline. A subagent from before the last message, opened again, shows its
copy on disk and is read again, where a closed window's transcript stayed in
memory before. A running subagent sends its patches on the conversation's
socket while it runs, whether or not it is chosen; a page that falls 4096
steps behind is disconnected and reconnects, which eight subagents streaming
to a page on a 30 kbit/s link with 300 ms of latency did not reach in 45
seconds. The server's recent patches of a running node are kept already,
for every page that follows it.

## Spike

None.

## Prior art

- Next.js's `<Link>` prefetches the page a link leads to once the link is in
  view, so following it shows at once
  ([docs](https://nextjs.org/docs/app/api-reference/components/link#prefetch)).
- TanStack Query prefetches a query the user is likely to need next, and
  keeps it fresh like any other
  ([docs](https://tanstack.com/query/latest/docs/framework/react/guides/prefetching)).

Demi departs from both in following what it prefetched: a subagent changes
while it runs, so a page read ahead without its changes would be stale.

## Alternatives

- **Read on hover.** A touch screen has no hover, and a running subagent
  read on hover would still need its changes.
- **Read only the running ones.** A subagent that just ended is opened as
  often, to read its report.
- **Read every subagent the Agents window lists.** A long conversation lists
  every subagent it ever ran, a thousand in some, which would take hundreds
  of megabytes and as many requests on each opening.
- **Keep reading on choice.** Every first choice waits for its request.

## Open questions

None.

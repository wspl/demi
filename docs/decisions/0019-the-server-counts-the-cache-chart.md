---
status: accepted
---

# The server counts the cache chart

## Context and Problem Statement

A context card shows the cache hit rate of its agent's recent calls and a
chart of it. The page reads the agent's last 400 calls when the card opens
(decision record 0009 made them a collection read when shown), groups them
into the chart's points and counts the rate itself. Until that read answers,
the card has no rate and no chart, and when it answers they push the rest of
the card down. Closing the card drops the calls, so every opening waits and
jumps again.

The rate and the chart's points are facts of the agent's calls, which the
server holds; the page only draws them.

## Considered Options

* Keep the read and hold the chart's room while it runs.
* The server counts the rate and the chart's points and sends them with what
  the page already receives for the agent.

## Decision Outcome

Chosen option: the server counts them, because a fact the server owns is
counted by the server and announced, and a card that needs no read cannot
wait for one.

* For each agent, the server keeps the rate of its recent calls and the
  chart's points: for each point its call numbers, its rate, the time from
  its first call to its last, the causes of its misses, whether a compaction
  came before it, and its price; for a point of one call, also what that
  call's tip says.
* They come with the agent's node when the page subscribes to it, and change
  as its calls do, announced like the node's messages. A card opens with them
  at once.
* The page draws what it receives and counts nothing. The read of an
  agent's calls and the page's grouping of them go.
* This supersedes the part of decision record 0009 that made the calls a
  collection read when a card is shown.

### Consequences

* Good, because a context card opens whole and never grows when a read
  answers.
* Good, because the chart's grouping lives in one place, beside the calls.
* Bad, because every subscribed node carries its chart, a few kilobytes,
  whether or not its card opens.

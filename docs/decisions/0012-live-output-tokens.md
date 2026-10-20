---
status: accepted
---

# Live output tokens and speed beside the chips

## Context and Problem Statement

While an answer streams, nothing tells the user how much the model has
written or how fast. The provider reports a request's output tokens only
when its response ends. The chips row above the composer holds the Agents
and Running chips on the left and the scroll-to-bottom control at its right
end.

## Considered Options

* The agent's server counts the request it runs as the model streams, and
  sends the reading with the agent's state; the page shows it.
* The page counts from the transcript's patches it receives.
* Show nothing until a request ends.

## Decision Outcome

Chosen option: the agent's server counts, because it owns the stream: every
page, one that watched the whole request or one that opened or reconnected
in the middle of it, shows the same reading with nothing to rebuild, and the
server sees tool calls' arguments as they stream, which the page does not.

### What the server keeps

* For the request the root agent runs: the output tokens so far and the
  time of its first token.
* While the request streams, the count is an estimate of what the model has
  streamed, its text, its thinking and its tool calls' arguments: about 4
  characters a token for Latin text, as Claude Code counts, a token for each
  CJK character, and 50 tokens for each tool call's framing, which the
  provider counts and never streams.
* When the response ends, its own output tokens replace the estimate.
* A new request, a retry of a failed one included, starts from zero.
* The reading goes out with the agent's state, at most twice a second and
  at once when a request starts or ends, and the conversation's snapshot
  carries it. It is gone when the turn ends.

### What the page shows

```text
 [2 Agents] [1 Running] (↓)                  ↓ 1.2K tokens · 48 tok/s
 ┌──────────────────────────────────────────────────────────────────────┐
 │ Ask anything…                                                     ■ │
 └──────────────────────────────────────────────────────────────────────┘
```

* At the right end of the chips row, led by a downward arrow: the count, as
  `1.2K tokens`, and the speed, the count over the time since the request's
  first token. The speed shows from the request's first second on, since
  less than that reads as thousands of tokens a second. The reading ends
  level with the send or stop button.
* Within a request the count rolls to each new value over about half a
  second, up or down; with reduced motion it changes at once. A new request,
  or another conversation's reading, shows at once, with no roll.
* When the turn ends, the last reading stays for 3 seconds, then leaves in
  a short fade.
* On a row too narrow for the chips, the scroll control and the reading,
  the speed goes first, then the whole reading; the chips keep their room.
* The scroll-to-bottom control moves to the left part of the row, right
  after the chips; with no chip it stands at the row's left end.

### Consequences

* Good, because the user sees the model working and how fast, the same in
  every tab and after a reconnect.
* Bad, because the count during a request is an estimate, which moves to
  the response's own count when the request ends.

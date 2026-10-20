---
status: accepted
---

# A subagent's context card shows the subagent's own cost

## Context and Problem Statement

The context card of the main conversation shows what the conversation cost:
the total, the price of each kind of token, the share of each profile and the
cache hit rate of the main conversation's recent calls. A subagent's context
card shows no cost, so a user who watches one subagent cannot tell what it
costs or whether its cache serves it.

The backend gives the page one cost per conversation (`GET
/conversations/:id/cost` and the `cost` frame). That cost holds a price per
agent but the price of each kind of token, and the calls, only for the
conversation as a whole and its main conversation. Nothing gives one
subagent's kinds or calls.

This record covers the subagent's own cost. It leaves out its descendants'
cost and a Profiles part in a subagent's card.

## Considered Options

* Read one agent's cost on its own: the cost route takes an agent, and the
  `cost` frame carries every agent's turn call.
* Add every agent's kinds and calls to the conversation's cost that the page
  already reads.
* Compute a subagent's cost in the page from the frames it receives while
  the subagent window is open.

## Decision Outcome

Chosen option: read one agent's cost on its own, because the page reads only
what the card it opens shows, and the main conversation's read stays as
small as it is.

* The cost route takes an optional agent. With one, it answers that agent's
  own cost in the same shape: its totals, its kinds, and its turn calls
  with their misses and compactions. It names no other agent.
* The `cost` frame carries the turn call of any agent, not only of the main
  conversation, with the agent it belongs to. A page that holds the
  conversation's cost adds what it adds today. A page that holds an agent's
  cost adds a frame of that agent and counts every frame, so a missed frame
  still makes it read again.
* The page reads a subagent's cost when it opens the subagent's window, as
  it reads the conversation's cost when it opens the conversation, so the
  card never shows the cost arriving late.
* The subagent's card shows the same Cost section as the main conversation's,
  without the Profiles part.

  The subagent's card with its Cost section open:

  ```text
  ┌────────────────────────────────────────┐
  │ Context window                       ⌄ │
  │ ...                                    │
  ├────────────────────────────────────────┤
  │ Cost  $4.20                          ⌃ │
  │ Input        $0.30  Output       $0.90 │
  │ Cache write  $0.60  Cache read   $2.40 │
  │                                        │
  │ Cache hit rate  96%                    │
  │ ▇▇▇▆▇▇▇▇▁▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇▇ │
  │  10        20        30        40      │
  └────────────────────────────────────────┘
  ```

  Closed, the section shows the subagent's price and hit rate side by side,
  as the main conversation's does.

### Consequences

* Good, because the conversation's cost, which every open of a conversation
  reads, does not grow with the number of subagents.
* Good, because one shape of cost serves both cards, and the frame stays the
  one way a change of cost is announced.
* Bad, because opening a subagent's window makes one more read.
* Modules touched: `backend-database` (usage), `backend-http` (the cost
  route), `conversation-socket-protocol` (the `cost` frame), and in the page
  the cost state of `web` and the subagent window's card in `web-ui`. No
  data to migrate: the ledger already records each row's agent.

---
status: proposed
---

# The output speed can be hidden

## Problem

Over the message field, a conversation shows the model's output: "↓ 1.2k
tokens · 85 tok/s" while a request runs, then "Last turn: 82 tok/s".
A user who does not watch the numbers sees them change all the time, and
nothing turns them off.

## Decision

Settings › Agent › Messages gains the switch Output speed, after
Suggested replies, on by default: "Shows the tokens a model writes and how fast, above the message
field." Off, neither reading shows, the one while a request runs nor the last
turn's speed, in the conversation's dock and in the subagents' panel alike.

```text
  on:   [Agents 2]                    ↓ 1.2k tokens · 85 tok/s
        ( Ask anything...                                  ↑ )

  off:  [Agents 2]
        ( Ask anything...                                  ↑ )
```

The reading keeps its room unseen, so the row stands as it did with the
switch on: a chip that comes or goes while the agent works moves nothing.

The choice is the account's, beside Suggested replies':

```ts
interface Preferences {
  // as before
  /** Absent means on. */
  outputSpeed?: boolean
}
```

`ChatSession`, `SessionDock` and `SubagentPanel` take `outputSpeed`, and
`RequestOutputMeter` takes `visible`. A page older than the switch ignores
the field and shows the reading; an older backend drops it, so the switch
stays on.

## Spike

None.

## Prior art

VS Code's [status bar](https://code.visualstudio.com/docs/getstarted/userinterface)
items can each be hidden from the bar's context menu, and macOS's Control
Center settings choose which items show in the menu bar; both leave a
reading of the app's state to the user's choice. Demi departs in putting the
switch in Settings, beside the conversation's other choices around the
message field, since a menu on a reading that is hidden cannot bring it
back.

## Alternatives

- Hiding only the last turn's speed: two readings of one row then need two
  rules; one switch hides the row's output whole.
- A menu on the reading itself to hide it: once hidden, nothing on screen
  brings it back.

## Open questions

None.

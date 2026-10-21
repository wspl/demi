---
status: accepted
---

# A move applies at once and stops nothing

## Context and Problem Statement

Moving a conversation to another project, directory or Host waits for its
agents to stop. Three parts each own a piece of that wait:

* `demi host move` records the move and returns at once, though nothing has
  moved yet.
* The backend applies a recorded move only once the whole agent tree is idle.
* The page stops each subagent, then each terminal one by one, then the
  root, waits for idle, and only then asks for the move.

So a move can take as long as the agents keep working, a command says it
moved when it did not, and the page can offer Move rather than Stop and Move
while a message it sent is still on its way. Checking the directory also
asks the Host twice, one question after the other.

The agents do not need to stop. A conversation's Host and working directory
already reach each agent with its next request, as a workflow change does.

## Considered Options

* Keep waiting for idle, with one owner of the wait and its progress.
* Apply a move at once, and let running work end where it started.

## Decision Outcome

Chosen option: apply it at once, because nothing a move changes needs the
agents to stop, and a move the user asked for should take effect when they
ask.

* A move is one backend operation that the page and `demi host move` both
  call. It checks the destination with one question to its Host, changes the
  conversation's project, Host and directory, and answers when that is done.
  The sidebar shows the conversation in its new place at once.
* Nothing is stopped or waited for. Commands already running finish where
  they started and report as usual. Each agent's next request names the new
  Host and directory, and every command started after the move runs there.
* The move dialog has one action, Move, in place of Stop and Move and Move
  and Tell Agent.
* Each move writes one log line with its id, the Hosts, how long each of its
  steps took and its outcome, and no conversation content.

```text
  Move to "billing-api" on Work Mac

  before:  [ Stop and Move ]  -> stop subagents, terminals, root
                               -> wait for idle -> move   (seconds to minutes)
  after:   [ Move ]           -> moved                     (one Host question)
```

### Performance

* A move costs one question to the destination's Host and one database
  write: about 20 ms on a local Host and one round trip, about 0.3 s, on a
  Host 300 ms away, where checking the directory took two round trips before
  and the wait for idle took as long as the agents ran.
* Nothing grows with the conversation's size: measured moves of a
  4,500-block and a small conversation took the same time.
* A Host that does not answer bounds the move by the question's deadline;
  the move then fails with why, and nothing has changed.

### Consequences

* Good, because a move takes effect at once, and what a command or the page
  says about it is true.
* Bad, because an agent in the middle of a task runs its next step in the
  new directory, which its earlier steps did not use; the agent learns of
  the move with that request.

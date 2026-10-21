---
status: accepted
---

# The page keeps no clock of a wait

Supersedes, in part: 0021 (a wait is the server's), where it counts a wait
this page started from the user's press.

## Context and Problem Statement

The transcript's last row says Requesting while an agent waits for its
model, with how long it has waited. Decision 0021 counts that time from the
user's press when this page sent the action that began the wait, and from
the server's start otherwise. To tell which, the page records each press,
every action carries an id, and the server names the action that began each
wait, so the page can match the two. That is a protocol and a store of
presses to move the start of a count by the time a message takes to reach
the server, usually under a second.

## Considered Options

* Keep counting from the press.
* Show two different things: an action not yet answered, without a time,
  and the server's wait, with the time the server counts.

## Decision Outcome

Chosen option: two things, because the page cannot know how long a wait it
has no answer about has lasted, and the server knows exactly how long the
waits it holds have lasted.

* While an action this page sent, such as a message that starts a turn, a
  Retry, a Resume or a side question, has no answer from the server yet, the
  row says Requesting with no time.
* Once the server tells that the agent waits for its model, the row says
  Requesting with the time the server counts from the wait's start; the
  page shows it and keeps no time of its own.
* The page records no presses; actions carry no id for this, and the server
  names no action that began a wait.
* The rest of 0021 stands: nothing in the transcript makes a wait, and the
  offline copy keeps the calls being written.

```text
  After Send, until the server answers:   ( ) Requesting
  Once the server tells the wait:         ( ) Requesting for 3s
```

### Performance

* Nothing new is sent or stored; the page drops its record of presses and
  each wait's action id leaves the agent entry.

### Consequences

* Good, because the page's part is a word while it waits for an answer, and
  every time shown is the server's.
* Bad, because the first moment after Send shows no time, and the time
  shown starts from when the server began the wait, not from the press.

---
status: accepted, superseded in part by 0028
---

# A wait is the server's

## Context and Problem Statement

While an agent runs, the transcript's last row says Requesting with the time
it has waited, from when the agent asked the model until the answer starts
to show. The server knows when each wait begins and ends, and tells the page
on every connection.

The page shows the row from its own guess instead: a running turn whose
transcript ends with finished output is taken as a wait, and when the server
has told no start the clock counts from when the view appeared. The guess
goes wrong when the transcript lacks a part:

* The page's offline copy of the agents names the version it was taken at
  but leaves out the calls being written. On reopening, the server sends only
  what changed since that version, so a call the model is writing stays
  missing, and the page reads its absence as a wait.
* Each switch to the conversation draws the view again, so the guessed clock
  starts from zero each time.

So returning to a conversation whose agent is writing a long call shows
Requesting, counting from zero, again on every return, while the same model
request goes on.

## Considered Options

* Fix the offline copy alone.
* The page shows a wait only from the server's word or an action of its own,
  and an offline copy holds exactly the version it names.

## Decision Outcome

Chosen option: the second, because the copy is one of several ways the
transcript can be incomplete, such as a first opening before the
transcript's tail arrives, and the guess fails on each of them.

* The row says Requesting only while the server says the agent waits, or
  while an action this page sent, such as a message that starts a turn or a
  recovery, waits for the server to confirm it. Nothing in the transcript
  makes a wait.
* The clock counts from the start the server tells, or from when this page
  sent its action. The start is held with the conversation, not with the
  view, so switching away and back keeps it.
* The offline copy of the agents holds them as they were at the version it
  names, the calls being written included. A wait's time is the one part it
  leaves out, since the server tells every wait again on each connection.

### Consequences

* Good, because a return to a conversation shows the wait the agent is in,
  with its real time, or the call it is writing.
* Bad, because a wait the server has not told yet, between a page's opening
  and its connection, shows Connecting rather than Requesting.

---
status: accepted
---

# A subagent reports by message

## Context and Problem Statement

A subagent writes its whole report as its answer and ends its turn. A test
it started is still running, so its session stays open. A minute later the
test ends and wakes it; it answers "The test passed." and ends again, now
with nothing running. Its session closes, and the parent receives "The test
passed." as the child's result: the report it wrote first is never
delivered.

Today the answer that closes a child's session is its result, and Demi
cannot tell an answer that ends the work from one that ends a turn to wait.
Any wakeup after the report replaces the report.

## Considered Options

* Deliver the answer of every turn the child ends.
* A handback call that fixes the result and ends the child's run.
* No result: the child sends its report as a message, and its end is only
  a notice.

## Decision Outcome

Chosen option: no result, because sending a message to the parent already
exists, and a report that is a message cannot be replaced by a later
answer.

* A child reports with `demi agent send parent`, whenever it has something
  the parent needs. Its answers stay in its own transcript and are sent to
  no one.
* A child's session closes when it does today: when nothing it started
  still runs. A turn it ends to wait for its commands or children is not an
  end.
* When the session closes, the parent receives a notice that names the
  child and how it ended: finished, failed or stopped. A failed child's
  notice keeps Demi's failure reason; no notice carries the child's own
  words.
* The commands a child leaves running are not stopped and nothing is done
  with their end.
* `demi agent show` of a closed child says how and when it closed, with no
  result.
* A parent that needs what a child did but did not send reads the child's
  history (a design of its own).

The parent's transcript:

```text
  Message from agent: Investigate the slow page
    The page waits 2.1 s on the first query, because …
  Agent finished: Investigate the slow page
```

### Performance

* No change: one message and one notice reach the parent, as the result and
  its completion did.

### Consequences

* Good, because nothing a child does after it reports can change what the
  parent received.
* Good, because a report, a question or a warning reach the parent the same
  way.
* Bad, because a child that ends without sending leaves its parent only the
  notice, and the parent must read its history.

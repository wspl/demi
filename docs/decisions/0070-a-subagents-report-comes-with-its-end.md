---
status: proposed
---

# A subagent's report comes with its end, and holds everything it wrote since it was spawned or last resumed

Supersedes, in part: 0037 (a subagent reports by message), where a child
reports with `demi agent send` and its end is a notice that carries none of
its words.

## Problem

A parent asks a subagent to investigate a slow page. The subagent finds the
cause, sends it with `demi agent send parent`, and ends its turn: the
message wakes the parent, which answers the user and ends its own turn.
Seconds later the subagent's end arrives as a notice of its own and wakes
the parent again, for nothing it needs to do. A parent that aborts a
subagent is woken in the same way by the notice that it was aborted, which
`demi agent abort` already told it.

## Decision

**The report comes with the end.** When a subagent ends, as today when
nothing it started still runs, its parent gets one notice: how it ended,
finished, failed or stopped, and its report, every text the subagent wrote
since it was spawned or last resumed, whole and in order. A message the
parent sends while the subagent works starts no new report: the report
still holds every text since the subagent was spawned or last resumed.
Turns it ended to wait for its own commands or children are not ends, as
today, and their texts are part of the report; its thinking and its
commands' output are not, and the parent reads them with
`demi conversation history --agent`.

A command it started with `service: true` does not keep it from ending,
where today it does; when it ends, its services are stopped, as an abort
stops what a subagent runs, without waiting for them. Any other command it
started still keeps it from ending, as today: work still running is not
delivered.

```text
  Agent finished: Investigate the slow page
    The page waits 2.1 s on the first query, because …

    The test passed.
```

This is what the parent's model reads; the page shows the notice as the
one-line receipt it shows today. A failed subagent's notice keeps Demi's
failure reason beside its texts.

**A subagent its parent aborts sends no notice:** the answer of
`demi agent abort` says it stopped, and the subagents that close along
with it send none either. A subagent the user stops from the
page sends its parent one notice, stopped, with its texts so far, and so
does one that Demi stops with the conversation's own turn, as a Cloud reset
does.

A subagent spawned before this change keeps, in its transcript, the block
that told it to report with `demi agent send`. Resumed, it may still send
its report as a message, and its parent is then woken twice, as today, with
nothing lost.

## Spike

None.

## Prior art

Not consulted.

## Alternatives

- **The report as a message, as 0037 chose.** Every report wakes the parent
  twice, once for the message and once for the end.
- **The last text alone as the report.** A subagent that ends a turn to
  wait for a test and then writes "The test passed." loses the report it
  wrote before, which is why 0037 refused it.

## Open questions

None.

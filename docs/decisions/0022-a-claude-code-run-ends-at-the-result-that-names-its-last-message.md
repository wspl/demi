---
status: accepted
---

# A Claude Code run ends at the result that names its last message

## Context and Problem Statement

The Claude Code provider keeps one CLI process per session and writes each
request's new user messages into it. The CLI answers messages that reach it
at different times in turns of their own, and ends each turn with a
`result` line. A run must end at the result that answers the last message it
wrote: a run that ends earlier leaves the rest of the answer to the next run,
which shows it in the wrong place or not at all.

Today the run counts. The CLI prints back each user message it takes into a
turn (`--replay-user-messages`), and a result ends the run only once every
message the run wrote was printed back. But the CLI merges the messages that
queue while it is busy into one turn, and prints that turn back once.

For example, four subagents finish at the same moment, and their four
receipts are written to an idle CLI. The CLI answers the first alone, then
answers the other three together in one turn. The run counted four messages
and saw two printed back. At the last result it still waits for two more,
which never come: the CLI is idle and says nothing until it gets input. The
session stays in its model request, the page shows Requesting, and every
later message waits behind it until someone presses Stop.

Two things make this possible:

- Which messages a turn consumed is a fact the CLI owns, and the provider
  guesses it from the lines it sees. The exceptions the count already needs,
  such as a message the CLI answers itself without the model, are more
  guesses of the same kind. The fake CLI the tests use prints back every
  message, which the real CLI does not, so the tests cannot catch it.
- The count lives on the process and carries over from run to run, and only
  the count reaching zero ends the wait. A drift in one run hangs the next,
  even one that writes no message.

## Considered Options

* Ask the CLI: give each user message a client id, and end the run at the
  result that names the newest id written to the process.
* Keep counting, and count each member of a merged turn from the lines the
  CLI prints back.
* End the run at a deadline after a result that leaves messages uncounted.

## Decision Outcome

Chosen option: ask the CLI, because the CLI already reports which messages
each turn consumed. Every `result` lists, in `user_message_uuids`, the client
ids of all the user messages the turn consumed, including every member of a
batch it merged. Counting merged members would keep guessing a grouping the
CLI may change at any release, and a deadline waits for time and still guesses
wrong for a slow turn.

* Each user message the provider writes carries a client id, and the process
  keeps the newest id written to it until a result names it.
* A result that names the newest id ends the run. A result that does not is
  an earlier turn's, whether it names other messages or none, such as a turn
  a resumed session's input started: its response is passed on and the run
  reads on. A failed result that does not name it closes the process, as a
  failed earlier turn does today. With no id outstanding, as in a new process
  that resumed its input, the first result ends the run.
* The count, its exceptions and `--replay-user-messages` are removed.
* The fake CLI in the tests answers as the real one does: it merges the
  messages that queue during a turn and lists them in its result. Tests cover
  a merged turn, a merged turn that calls a tool and ends in the next run, and
  a further run in the same process.
* A check with the real CLI (Claude Code 2.1.296, run as Demi runs it)
  confirmed the field: three messages written while the first turn ran gave a
  result naming the first, then one result naming the other two; a turn that
  called a tool and a slash command the CLI answered itself each named their
  message.

### Consequences

* Good, because the run ends when the CLI says the last message was answered,
  however the CLI groups messages into turns, so a merged batch can no longer
  hang a session.
* Good, because one rule replaces a count and its exceptions.
* Bad, because the run depends on a field the CLI's output marks optional. A
  CLI release that stops writing it would leave runs waiting again, so a new
  CLI version is checked for it before Demi installs it.

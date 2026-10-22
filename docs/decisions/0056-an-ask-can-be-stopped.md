---
status: accepted
---

# A side question can be stopped

## Summary

While a side question is being answered, its card offers Stop, which ends
the answer where it is, as the message field's Stop ends a turn. The card
keeps what was answered so far, says it was stopped, and still takes a
follow-up.

## Motivation

A user selects words, types "try it again" and presses Ask by mistake. The
side question starts working on the main conversation's files and cannot be
stopped: the card shows its status line until the answer ends, and the
message field's Stop stops the main turn, not the side question.

## Guide-level explanation

```text
  ┌ ? Start-up Predictability ─────────────── ■  ↗  × ┐
  │ Is that always true?                              │
  │ ◌ Reading files…                                  │
  └───────────────────────────────────────────────────┘

  after Stop:
  ┌ ? Start-up Predictability ────────────────── ↗  × ┐
  │ Is that always true?                              │
  │ No. Caching usually makes sta…                    │
  │ Stopped                                           │
  │ Reply                                             │
  └───────────────────────────────────────────────────┘
```

* While the question works, a square Stop icon stands in the card's header,
  before ↗ and ×, with the tooltip Stop; it goes when the answer ends or is
  stopped. The card's body has no row for it, so nothing in it moves.
* Stop ends the side question's turn and every command it started, at once.
* The card then shows what was answered so far, if anything, and a muted
  Stopped under it; Reply stays where it is in an answered card.
* Esc in the card does not stop it; only the button does.
* × on a card still working on its first answer stops it and removes the
  card, as if it had not been asked.

## Reference-level explanation

### Route

```text
POST /api/conversations/{id}/side-questions/{side}/stop
  204  the side question's turn is stopped, or nothing was running
  404  no side question {side} in conversation {id}
```

The route stands beside the side question's other actions (`ask`,
`messages`, `retry`, `fold`) and takes the same per-side start lock. The
card sends its actions on one side question one after another, so a Stop
pressed before the question or a follow-up reached the side question is
sent once it has, and stops it.

### Behavior

* Stop ends the side question's turn as the message field's Stop ends the
  main turn: the turn records its stop (an `abort` block) after what it
  wrote so far, the first message included when nothing else was written.
* Stop also stops every command the side question started. It asks each to
  stop and returns once the stop is recorded; it never waits for a command
  to end or for its Host, so a Host that is away delays nothing.
* A command ended by this stop is reported as stopped by the side question:
  its end opens no new turn, and the side question stays stopped.
* A side question with nothing running answers 204 and records nothing.
* The side question stays open to a follow-up, which starts a new turn.
* A stopped turn is never told to the main agent: the main agent hears of a
  side question's turns that answered, as today, and of no turn that was
  stopped, the question before its answer included.
* × on a card whose question has no answer yet, its first turn still
  working, stops the side question and deletes it with its card; × on any
  other card folds it into the margin, as today.
* The deletion is `DELETE /api/conversations/{id}/side-questions/{side}`,
  under the same lock: it stops the side question, deletes its node, its
  transcript and its card, and pushes the removal to the conversation's
  pages as one change of the agents' list, an `agent` frame whose entry
  says it was removed, at the next revision. A page that cannot apply it
  reads the list again, as for any change it cannot apply. A 404 answers a
  deletion another page made first, and the card counts it as done.

```rust
struct AgentEntry {
    // Existing fields stay.
    /// The agent was removed; sent once, in the change that removes it, and
    /// never in a whole list
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    deleted: bool,
}
```

* A conversation that opens again after a backend restart takes up the
  stops still in progress of its closed side questions, as it does for its
  agents.

### Page

* `AskCard` shows Stop in its header while the last turn works and its last
  block is not `abort`.
* A turn whose last block is `abort` shows what it answered and Stopped, and
  never the working status line, also while a follow-up is on its way.
* After Stop the focus moves to the card's Reply.
* The words are the existing Stop and Stopped strings, the transcript's own.

## Upgrade and version skew

An older page offers no Stop and does not know `deleted`: a card another
page deleted stays on it, idle, until it reloads.

## Performance

None.

## Drawbacks

* The card's Stop ends the commands its turn started, where the message
  field's Stop leaves started commands running: a side question is asked
  aside, and a user who stops it means all of it.
* A deletion removes the record of a command still stopping, whose Host is
  away: if the backend restarts before the Host returns, nobody asks that
  command to stop again.
* A side question that answers between the click on × and the deletion is
  deleted though it answered.

## Rationale and alternatives

* Stop by the socket's `AbortSubagent` frame: it has no answer, and a Stop
  sent before the side question exists was dropped silently.
* Stop the side question with the message field's Stop: the field's Stop
  belongs to the main turn, which may be running too.
* Stop at the end of the status line: while the answer streams there is no
  status line, so Stop takes a row of its own, which goes when the answer
  ends and moves the card.

## Prior art

* ChatGPT and Claude show Stop while an answer streams, and keep the partial
  answer; Notion AI's inline box shows Stop while it writes.

## Unresolved questions

None.

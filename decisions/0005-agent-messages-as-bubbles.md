---
status: accepted
---

# A subagent shows the messages it receives as bubbles

## Context and Problem Statement

A subagent's view shows the brief that started it as a message bubble, the
way the user's messages look in a conversation. Every later message it
receives from another agent shows as one line, such as *Message from main
agent*, and its words open only on a click. A user who reads a subagent's
view cannot see what it was told while it worked without opening each line.

This record covers how a message from another agent shows in the view of the
agent that receives it. It leaves out the brief, which is already a bubble,
and how a message looks in the sender's view.

## Considered Options

* A message shows as a bubble, unless its sender is below the receiver in the
  agent tree.
* Every message between agents shows as a bubble, in the main agent's view
  too.
* Keep every later message as one line.

## Decision Outcome

Chosen option: a bubble unless the sender is below the receiver, because a
message from above or beside an agent is an instruction it works from, as the
brief is, while a report from below is a result it reads.

* A message from the receiver's parent, or from any agent other than one below
  the receiver, such as a sibling, shows as a bubble with its words, like the
  brief.
* A message from an agent below the receiver, its child's message or report
  included, keeps its line as now. The main agent's view therefore does not
  change.
* A bubble from the main agent shows no name, as the brief does. A bubble from
  any other agent shows the sender's name in small text above it, as a group
  chat names its members, and the name opens the sender as the line's name
  does now.

```text
 subagent view
                           ┌────────────────────────────────┐
                           │ Tidy the release notes …       │  brief
                           └────────────────────────────────┘
  Ran 3 commands for 12s
                           ┌────────────────────────────────┐
                           │ Also keep the old heading.     │  from main agent
                           └────────────────────────────────┘
                                             Review the notes
                           ┌────────────────────────────────┐
                           │ The table on line 40 is broken.│  from a sibling
                           └────────────────────────────────┘
  ◇ Finished: Check the links                                 from its child
```

### Consequences

* Good, because a subagent's view reads like a conversation: what it was told
  and what it did, in order, without opening anything.
* Bad, because a long message takes the room of its words where a line took
  one row.
* Modules touched: where a run ends in `agent-store`, which a bubble's message
  now ends as the user's words do, and the transcript's rows in `web-ui` and
  their gallery specimens. The protocol already carries each message's sender.

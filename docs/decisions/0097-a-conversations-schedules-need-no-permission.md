---
status: proposed
---

# A conversation's schedules need no permission

Supersedes: 0083 (A conversation has schedules), the part that the agent
creates, changes and deletes schedules with the user's Manage Schedules
permission.

## Problem

Alex asks the agent to check the build every morning at nine. The agent runs
`demi schedule create`, and the conversation stops on a card that asks Alex to
let the agents of this conversation manage schedules. Alex has just asked for
the schedule, and has to approve it again before it exists.

The permission came from automations, which started a new conversation each
run on a device and in a project the automation named: an agent that made
one could start work elsewhere. Since 0083, a schedule only wakes the
conversation it belongs to, with a message its agent wrote. That is what the
agent does on its own when it waits for a command and its end wakes it, which
needs no permission.

## Decision

**The agent creates, changes and deletes its conversation's schedules
without asking, whatever their trigger, a URL included.** No card asks for
Manage Schedules, and the permission is gone. The user sees every schedule where 0083 shows them, under the
composer, and turns one off or deletes it there.

The design to build from:

* `backend-user-shard`: the `demi schedule` group registers no permission,
  and `create`, `edit` and `delete` require none. `MANAGE`, its action and
  its description go, and the commands' summaries no longer say that they
  need the user's permission. The group's entry comment says what guards it
  now: it acts on its own conversation's schedules only.
* `backend-database`: the migration from 0.1.65, which renamed the grants
  and the pending requests of `automation.manage` to `schedule.manage`,
  deletes them instead, as it does those of `conversation.organize`. A
  database already past it, only a development build's, keeps its rows of
  `schedule.manage`, and nothing removes them.

A page or a runner of the previous release sees no difference: the card it
would have shown is never asked for.

## Spike

None.

## Prior art

None: the decision removes a guard and adds no capability.

## Alternatives

* **Keep the permission, asked once per conversation.** It still stops the
  first schedule on a card the user's own request made unnecessary.
* **Keep it for a URL trigger only.** A URL trigger gives the conversation
  an address on the internet: whoever holds the address and its secret wakes
  the agent, and the call's body reaches it as a file in its scratch
  directory, up to the trigger's daily limit. It still wakes only this
  conversation, and its body is data the agent reads, not a command, so it
  needs no more than the other triggers.

## Open questions

None.

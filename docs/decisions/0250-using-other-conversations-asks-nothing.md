---
status: accepted
---

# Using other conversations asks nothing

Supersedes: 0205 (A permission covers a whole capability), the row
Conversations and its category `conversation.use`; 0207 (An agent starts a
conversation), its Permission.

## Problem

Alex asks a conversation to hand a task to another one, or to read what an
earlier conversation found. Before the agent can list, read, send or start
anything, a card asks Alex to allow Use Your Other Conversations. Alex asked
for exactly that, so the card only stops the work until Alex comes back to
click Allow. The same happens in every new conversation, since the
permission lasts for one conversation only.

What the permission guards stays in the user's view: every conversation is
the user's own, in the user's sidebar, where the user sees what a message
started, stops it, and archives what should stay out of reach.

## Decision

**An agent lists the user's conversations, reads their transcripts, sends to
them and starts new ones without asking.** No card asks for Use Your Other
Conversations, and the permission is gone, as 0087 did for renaming and 0097
for schedules. An archived conversation is still listed nowhere and receives
nothing.

The design to build from:

- `backend-user-shard`: the `demi conversation` group registers no
  permission, and `list`, `send`, `new` and `history --conversation` require
  none. `CONVERSATIONS`, its action and its description go, and with them the
  grant `deliver` gave the conversation a message reaches. The summaries no
  longer say that reading another conversation asks the user. The entry
  comment of `organize.rs` says what guards the group now: it reaches only
  the user's own conversations that are not archived.
- `backend-database`: a migration of the control schema deletes the grants
  of `conversation.use` and the requests for it, undecided or decided and
  not yet delivered: none would mean anything now. The control schema's
  comment says that only earlier releases decided `reached`, which is a
  new schema text, so the migration runs.
- `web-ui`: the category's words in each locale and the gallery's receipt of
  it go. A receipt in a transcript keeps the words it was recorded with, so
  an earlier decision about the category still reads as it did.

A page or a runner of the previous release sees no difference: the card it
would have shown is never asked for.

## Spike

None.

## Prior art

None: the decision removes a guard and adds no capability.

## Alternatives

- **Keep the permission, asked once per conversation.** It still stops the
  first command of every conversation on a card the user's own request made
  unnecessary.
- **Ask only to read another conversation**, since what it reads goes to
  this conversation's model provider. The user picks that provider for their
  own conversations, and a read only moves the user's own words between
  them.

## Open questions

None.

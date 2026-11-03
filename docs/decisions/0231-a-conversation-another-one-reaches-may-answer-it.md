---
status: proposed
---

# A conversation another one reaches may answer it

Supersedes: 0205 (A permission covers a whole capability), the part that
asks a conversation for `conversation.use` when another conversation has
sent to it or started it.

## Problem

Alex allows Use Your Other Conversations in the conversation *Release
prep*, and its agent sends *Check the login test* to the conversation
*Stabilize the login test*. That agent does the work and wants to report
back with `demi conversation send`, and stops for a card: Allow This
Conversation to Use Your Other Conversations? Alex answers a second card
for the same exchange. The same happens to a conversation that *Release
prep* started with `demi conversation new` and that should report back,
and to one that needs to read *Release prep*'s transcript to do what it was
asked.

## Decision

A conversation that another conversation's agent sends a message to, or
starts, holds `conversation.use` from then on, as if Alex had allowed it.
It answers, reads the conversation that reached it, and does whatever the
category covers, without a card.

- Only an agent's message grants it: `demi conversation send` and `demi
  conversation new`, which run only in a conversation that holds the
  category. A message the user types grants nothing.
- Only `conversation.use`: the conversation needs it to answer. Credentials
  and devices stay as they are, each asked on its own.
- The grant is the same as one the user gives: it is kept for the
  conversation's life.
- A request for the category that waits in the reached conversation is
  allowed when the message arrives. Its row in the conversation says why,
  not that Alex allowed it: "Allowed this conversation to use your other
  conversations, since another conversation reached it". The agent that
  asked reads the same: the decision is `reached`, beside `allowed` and
  `denied`.
- The card that asks for the category says so, after what the agents may
  do: "A conversation they send to or start can do the same."

The design to build from:

```rust
// crates/backend-database/src/permissions.rs
impl ControlService {
    /// Grants `category` to `conversation`, as a user's Allow does, and
    /// allows the undecided requests for it.
    pub async fn grant_permission(&self, conversation: ConversationId, category: &str)
        -> Result<Vec<StoredRequest>, StorageError>;
}

// crates/shared-types/src/agent_message.rs
pub enum PermissionOutcome {
    Allowed,
    Denied,
    /// Another conversation's agent reached the conversation.
    Reached,
}
// The control schema's permission_requests.decision admits 'reached'.

// crates/backend-user-shard/src/conversation/organize.rs
// The delivery `send` and `new` share grants CONVERSATIONS to the conversation
// it reaches before the message is admitted. The requests it allows are
// delivered to their agents as a decision is.
```

The page and the backend ship together, so no page meets a `reached`
decision it does not know.

## Spike

None: the design adds a grant through the path a user's decision takes.

## Prior art

- macOS attributes a process to its responsible app: a helper an app
  starts uses the app's privacy grants, and the user is not asked again for
  it.
- Claude Code runs a subagent under the permission settings of the session
  that started it.

## Alternatives

- Letting a reached conversation send to the conversation that reached it
  without the category, and nothing else: its agent often needs to read
  that conversation's transcript to answer, which asks again, and it brings
  back a per-object rule that 0205 removed.
- Sending without a permission at all, as 0140 decided: 0205 brought it
  under the category because a message runs another conversation's model.
- Asking in the sending conversation's card for both: the card cannot know
  which conversations the agent will reach.

## Open questions

None.

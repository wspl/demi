---
status: proposed
---

# Sending a message to another conversation asks no permission

Supersedes: 0088 (An agent sends a message to another conversation), the
permission `conversation.send` its first send asks.

## Problem

Alex's conversation coordinates the others. Each new conversation Alex
starts waits on a card, *Allow this conversation to send messages to your
other conversations*, the first time its agent answers the coordinator,
though the answer is only words the recipient reads as a message from
another agent and that the user sees in the recipient's view.

## Decision

`demi conversation send` asks nothing: any agent of a conversation sends a
message to the main agent of another conversation of the user's that is
not archived, from its first send.

- Reading stays asked: `demi conversation list` and `demi conversation
  history --conversation` ask Read Your Other Conversations
  (`conversation.read`) once per conversation, since what they read goes to
  the reading conversation's model provider.
- An agent that may not list finds a recipient by the id another
  conversation's message named, or the user gave.
- The category goes from the `conversation` group
  (`crates/backend-user-shard/src/conversation/organize.rs`), and its words
  from the page's catalogues (`packages/web-ui/src/permissions/locales`).

What older data meets: a grant of `conversation.send` stays stored and
allows nothing a conversation lacks. A request for it that waits for an
answer shows its card with the category's id, as a card for any category
no longer declared does. Its answer reaches the agent that asked as any
answer does; a Deny tells it the user refused sending, though its next
send goes through.

## Spike

None.

## Prior art

Claude Code's cross-session messaging sends with `SendMessage` without
asking the sending session, and keeps permissions per session: the
receiving session's own prompts and rules apply to whatever a message asks
for ([cross-session messaging](https://code.claude.com/docs/en/cross-session-messaging)).
opencode-plugin-peers, an open-source plugin modeled on it, lets the
instances of one OS user find each other and exchange messages, its trust
boundary being that user, with no prompt to send
([opencode-plugin-peers](https://github.com/jkrandom-sudo/opencode-plugin-peers)).

Claude Code departs from Demi in its inbound controls, by which a receiving session
holds or refuses a message; in Demi every conversation is the user's own,
and the recipient's permissions already stand between a message and what
it asks for.

## Alternatives

- **Keep the category, allowed by default.** A switch nobody turns off
  is a card's words with nothing behind them.
- **A migration that removes the grants and the waiting requests,** as
  earlier categories went. A schema's version is its SQL's digest, and
  this change edits no table, so it would need an edit made only to run
  it; what the rows it leaves can do is one card, answered once.

## Open questions

None.

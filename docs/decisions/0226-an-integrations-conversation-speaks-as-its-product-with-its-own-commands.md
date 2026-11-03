---
status: proposed
---

# An integration's conversation speaks as its product, with its own commands

## Problem

A team's chat bot answers in a group. Its agent's system prompt opens with
"You are Demi, an agent that works for the user on their computers: a Demi
Cloud machine or the devices they paired", and its guide says the work panel
shows the working directory. The bot is neither: it serves several people
in a chat and has no work panel. Its workflow's instructions can only follow
that sentence, so the model reads two identities. The bot also runs a
nightly conversation that only tidies its memory notes; that conversation
has every command the bot declared, though it should have only the memory
ones.

## Decision

### The identity

`CreateConversation` and `PATCH /v1/conversations/{id}` take `identity`,
the sentence that opens the main agent's system prompt in place of Demi's
own (`INSTRUCTIONS` in `conversation/product.rs`), such as "You are the
Acme team's assistant in its group chat."; none keeps Demi's.

* The agent server takes the identity from one source per conversation
  with the guide, instead of one string for every node, and reads both as
  the conversation's tree opens. A `PATCH` of `identity` gives every live
  agent of the conversation the new one from its next request, as a change
  of the user's instructions reaches the next request; that request misses
  the provider's prompt cache once.
* A subagent opens with the conversation's identity, and its profile's
  instructions follow it, as they follow Demi's today.
* The guide that follows it keeps the runtime's rules. For an integration's
  conversation the first rule ends at "a context block names both. The
  directory stays between turns.", without "and the work panel shows it",
  which is a display convention of the web app's; the user's own
  conversations read the rule as today.

### The commands

`CreateConversation` and `PATCH` take `commands`, the roots of the
integration's declaration the conversation's agents have, such as
`["memory"]`; none has them all, as today. The conversation's toolset holds
those roots and its revision names them, so two conversations with
different roots never share a toolset. A root the declaration no longer has
is left out, as a declaration that drops a root drops it from every
conversation.

A `PATCH` of `commands` gives every live agent of the conversation the
roots from its next command, and the list of commands in its system prompt
from its next request, as a `PATCH` of `identity` does; that request
misses the provider's prompt cache once. A program that moves one group's
conversation to the next version of its commands, under a root of its own
such as `chatbot-next`, then has an agent that reads and runs the same
commands. A new declaration and the user's switch of a plugin still change
the commands of every live conversation at once and leave the list in their
prompts until each next opens (0008), so that one change does not miss every
conversation's cache.

The integration's capabilities (0149) stay its own, for every
conversation.

## Spike

None. `INSTRUCTIONS` is one string the agent server gets at its start
(`conversation/mod.rs`); the guide is already a source per root, read as
a tree opens (`agent-server/src/server/mod.rs`), which the identity follows.

## Prior art

* OpenAI's Assistants and Responses APIs take the whole system prompt from
  the caller, with nothing of the platform's before it.
* Claude Code's `--system-prompt` replaces its default prompt, while
  `--append-system-prompt` adds to it.

Here the identity is replaced and the runtime's rules stay, because the
agent cannot use Demi's commands without them.

## Alternatives

* **The identity in the workflow's YAML**: a project's workflow file would
  then replace the identity of the user's own conversations too.
* **Command sets per key prefix**: the program names
  a conversation's roots where it makes it, and needs no rule over its own
  keys.

## Open questions

None.

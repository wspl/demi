---
status: proposed
---

# An agent renames its conversation without asking

## Problem

A user asks an agent to keep its conversation's title on what the
conversation is doing now, such as "Photo album: thumbnails". The first
`demi conversation rename` in each conversation stops on a card, "Allow this
conversation to organize conversations?", and waits until the user answers
it. A title is the conversation's own, the user sees it change in the
sidebar at once and can rename it back, so the card asks the user to guard
nothing.

## Decision

`demi conversation rename <title>` renames the conversation it runs in at
once, in every conversation, with no card and no permission to grant.

* The Organize Conversations permission goes: its category
  (`conversation.organize`), its words in every language, and the line the
  command's help has for it.
* A migration deletes its stored grants and every request for it still
  stored: one that waits for an answer, and one whose answer a restart kept
  from its agent. Its command ended with the restart, and no command needs
  the answer now.
* A receipt of an earlier answer keeps its words: the page shows the action
  the request stored, in English.
* The command still renames only the conversation it runs in, and a blank
  title is still refused.

## Spike

None.

## Prior art

ChatGPT, Claude and Codex title a conversation without asking, and so does
Demi's own generated title.

## Alternatives

* Allow it by default and keep the permission for a user to turn off: a
  setting for a change the user undoes with one rename.
* Ask once per user instead of once per conversation: still a card for
  something that needs no guard.

## Open questions

None.

---
status: proposed, superseded in part by 0285
---

# Suggested replies outlive a restart

Supersedes the memory that held suggested replies in
[0182](0182-a-reply-offers-the-users-likely-next-messages.md).

## Problem

An answer offers "Merge it", "Show me the diff first" and "Add a test for
the old cookie name". The server is upgraded, which restarts the backend.
The user opens the conversation: the page first shows the conversation as it
kept it, the three pills included, and a moment later the backend's fresh
metadata, which the restart emptied of them, takes them away. Every upgrade
makes every conversation's pills flash and vanish, and leaves its last
answer offering nothing.

## Decision

The backend stores what a suggested replies request kept, the answer it
follows and its replies, in the control database, one row per conversation,
which the next answer's replace and the conversation's deletion removes. The
conversation's metadata reads it there. A restart keeps it, so the page's
copy and the backend's agree and nothing moves as the page connects.

An archive stops the conversation's request and keeps the row; the page
offers an archived conversation's none, and its last answer offers them
again once it is restored.

## Spike

None.

## Prior art

- The draft (`conversation_drafts`) is kept the same way, one row per
  conversation in the control database, removed with the conversation.

## Alternatives

- The page shows no suggestions from its own copy until the backend
  confirms them: the pills come a moment after the conversation, moving
  what is under them, and every restart still loses them.

## Open questions

None.

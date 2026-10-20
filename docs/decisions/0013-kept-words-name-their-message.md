---
status: accepted
---

# Kept words name their message and hold only text

## Context and Problem Statement

A compaction keeps the user's own messages word for word beside its
summary, up to 20,000 tokens, and carries them into every later
compaction. Each kept message holds its images and its attachments' tags,
and nothing says which message of the conversation it was. The model
cannot go back to a kept message's surroundings, and the kept images and
tags take room that text needs.

## Considered Options

* Each kept message names the message it came from and keeps only its
  text, with a placeholder for each image and attachment.
* Keep messages as they are.
* Let the summary list the user's messages instead, as Claude Code does.

## Decision Outcome

Chosen option: name the message and keep only text, because the user's
words stay exact, as Codex keeps them, and the model can read anything
the placeholders stand for from the conversation's history.

* Each kept message starts with the index of the message it came from,
  the index that `demi conversation history read` and `open` take.
* An image is kept as `[Image]` and an attachment as `[Attachment]`, where
  each stood in the message. Their bytes, names and ids stay in the
  history, where the message's index finds them.
* A message carried from an earlier compaction keeps its index.
* The note before the kept messages says what the index is for.
* A conversation compacted before this change has its kept messages
  rewritten the same way when the backend upgrades it.

```text
The earlier part of this conversation was compacted. … Each kept message
starts with its index, which `demi conversation history read` and `open`
take.

[#30276]
The logo still does not show after a reload. [Image]

<summary>
…
</summary>
```

### Consequences

* Good, because the kept words are exact and each leads back to its
  context in the history.
* Good, because images and attachments no longer take the kept words'
  room.
* Bad, because the model sees a kept image only after it opens the
  message from the history.

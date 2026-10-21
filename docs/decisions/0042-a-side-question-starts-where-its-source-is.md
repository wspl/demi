---
status: accepted
---

# A side question starts where its source is

## Context and Problem Statement

A user asks a side question in a conversation of tens of thousands of
blocks. Before its model is called, the backend copies the reply's whole
path, hundreds of megabytes of blocks with their blob and attachment
references, into a new node: tens of seconds on a small server, seconds on
a fast laptop, nearly all the time to the model. A conversation's fork
copies the same way, through the same function. Nearly all of what they
copy is history a compaction already summarized, which the model never
reads again.

An edit, a Regenerate or a retry copies nothing: within its node, it adds
rows that follow an existing row, and the rows before stay where they are.
Settled rows are never rewritten or deleted.

## Precedents

* Codex's newer threads fork by reference: the fork names its source and
  the position its history ends at, and adds only its own records. A
  thread a fork references cannot be deleted alone, and reads and searches
  pass through the reference (openai/codex#35220).
* Within one conversation, Claude Code, Open WebUI, LibreChat and Pi keep a
  tree of messages, each naming its parent, and share the ancestors.
* Across conversations, most copy: Claude Code, OpenCode, OpenHands, Open
  WebUI and LibreChat.

## Considered Options

* Keep copying.
* A side question starts at its source's row; a conversation's fork
  copies.
* Every branch, a conversation's fork included, references its source.

## Decision Outcome

Chosen option: a side question starts at its source's row, because the
rows it would copy are already settled and never change, as an edit's
branch already shares them.

* A side question makes a node whose path starts at a row of its source
  node: the rows up to it are the source's, and the
  node's own rows follow. No row before the source's window, the history
  from its last compaction, is copied. What the source does later,
  going on, an edit, a Regenerate, does not change the rows the branch
  starts from.
* The blocks of the source's window, through the asked reply, are the
  node's own rows. The source may not have saved them yet, and a compaction
  bounds them, so they do not grow with the conversation.
* Reading a node's history reads its source's rows up to that row, then
  its own, the same way for the page, the model and the history commands.
* A node another node starts from is deleted only with it.
* A conversation's fork, into another conversation's database, still
  copies, as now; it calls no model, and its cost is paid once.

### Performance

* A side question writes one node, the blocks of the source's window, and
  the few runs that name the source's rows: its creation no longer grows
  with the conversation.
* A read of a branch's history reads through one more node, by the same
  index.

### Risks

* Every reader of a node's path must follow the start. One that reads only
  the node's own rows shows the branch without its history.

### Consequences

* Good, because a side question in a long conversation starts as fast as
  in a short one.
* Bad, because a node's history may live in two nodes.

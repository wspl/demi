---
status: accepted
---

# A conversation opens whole

Supersedes, in part: 0009 (agent sessions and sync), where it says nothing
bundles a first screen.

## Context and Problem Statement

Opening a conversation shows its transcript first, then fills the rest in as
separate reads come back: the model's name, saved messages, attached Hosts,
permission requests, workflows, the work panel's tabs, and each attachment a
reply names. The socket's opening carries the transcript, the agents and the
metadata, but the metadata names only a revision for each list, so the page
reads each list on its own after it. With 300 ms between the page and the
server the conversation takes 1.6 s to stop changing, and an attachment's
card turns into a picture twice its height, pushing the reply down.

The backend holds all of these already; nothing in them needs a Host.

## Considered Options

* Keep the reads and reserve room for what they bring.
* The opening carries every fact the conversation's view shows, and a later
  change carries its new value.

## Decision Outcome

Chosen option: the opening carries them, because the facts are the
backend's, and a view assembled from reads that land apart changes shape
however much room it reserves.

* The socket's opening carries, with the transcript and the agents, the
  conversation's saved messages (each as the composer lists it: its first
  lines, its size and its attachments; the whole text is read when one is
  used), attached Hosts, permission requests,
  workflows, the work panel's tabs, and the description of each attachment
  the transcript's shown messages name (name, kind, size, and a picture's
  width and height). A transcript page that arrives later carries the
  descriptions of the attachments it names.
* A later change of any of these is announced by its owner with its new
  value, under its revision, on the same socket. The page reads none of them
  by a route.
* The model catalog is the account's: it comes with the product's state the
  page holds before any conversation, and its owner announces a new catalog.
  The selected model's name and abilities are known when the conversation
  opens.
* The offline copy holds the same facts with the version they were taken at,
  so an opening from the copy shows the whole conversation at once, and the
  socket then sends only what changed since.
* The conversation's view shows when its facts are all in, from the copy or
  from the opening, never a part of them.

```text
  Before (300 ms away)                After

  0 ms    transcript                  0 ms    transcript, model, saved,
  300 ms  model name                          hosts, permissions, panel
  600 ms  saved, hosts, permissions           tabs, attachment cards at
  900 ms  attachment card -> picture          their size
  1.6 s   panel tabs, workflows
```

### Performance

Measured on a set of long-used conversations (the largest twenty and the
median):

| What the opening adds | Largest | Median |
|---|---|---|
| Attachment descriptions in the shown tail (0 to 7) | 1.1 KB | none |
| Saved messages, Hosts, permissions, workflows, tabs | 3.7 KB | under 0.1 KB |
| Offline copy per conversation | 4.1 KB | 0.2 KB |

* The model catalog, about 11 KB for an account with several providers,
  moves from its own read to the product's state.
* The backend reads these lists with indexed queries that touch no
  transcript rows, on a database of several gigabytes too.
* Risk: saved messages can hold up to 256 KiB each; carrying them whole
  could add megabytes to an opening, so the opening carries each one as the
  composer lists it.
* Removed: one round trip per list and per attachment after the opening,
  about 1.3 s of the 1.6 s a conversation took to stop changing 300 ms away.

### Consequences

* Good, because a conversation appears once, in its final shape.
* Good, because an attachment's picture has its size before its bytes, so
  nothing below it moves.
* Bad, because the opening of a conversation with many attachments in view
  grows by a few hundred bytes for each.

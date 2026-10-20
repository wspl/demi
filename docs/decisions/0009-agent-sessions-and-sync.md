---
status: accepted
---

# Agent sessions, and one way to keep their data in sync

## Context and Problem Statement

A conversation's main agent and its subagents are rows of the same kind in
storage, but every layer above treats them differently:

* The main agent has its own socket frames, page fields and offline copy.
* A subagent has `subagent_*` frames, a different record type, one window of
  messages that is dropped when its tab is hidden, and nothing on disk.
* A subagent's metadata comes only from the subagent list, and its cost is
  read by number while everything else uses its id.
* "4 Agents, 3 Running" is pieced together on the page from several frames.

Opening a subagent tab therefore loads from scratch each time, and every fix
to loading, caching or updating is made twice or only once.

## Considered Options

* Every agent, main or sub, is the same agent session at every layer, and
  every collection the page shows is kept in sync one way: a version per
  collection, patches that follow it, and a reset when they cannot.
* Keep the two paths and add a cache to the subagent's.

## Decision Outcome

Chosen option: one kind of agent session, kept in sync one way, because the
page then has one path to load, cache and update for all of them, and the
user waits for nothing it has seen before.

### The model

```text
 list entry     every conversation, for the sidebar: title, archived,
                pinned, status, read position and unread, created and
                updated, Host, permission requests waiting, automation run
 metadata       an open conversation only: model settings, workflow,
                working directory, shares, latest attachment number,
                plugins changed, waits only for its Host, the queue, and
                every agent's metadata, the root's included: phase, round,
                waiting, cost, context use, calls being written, profile,
                parent
 collections    each with a version of its own, read when shown: the draft,
                saved messages, the work panel, attached Hosts, permission
                requests, each plugin's conversation state, and the calls
                behind a context popup's cache chart
 node messages  one transcript per agent, the root's like any other
```

* A conversation's tree holds the main agent at its root, and subagents and
  side questions below it; every node is one agent session, and a side
  question's metadata is its node's.
* What the page shows of a conversation's agents, such as "4 Agents,
  3 Running", comes from the conversation's metadata.
* The list entry carries only what the sidebar shows. The versions of a
  conversation's collections come with its metadata, so only a page that
  shows the conversation hears of their changes.
* The queue, the calls being written and the waiting time live while the
  conversation runs: they come over its connection and never go to disk.

### The interfaces

```text
 GET  /conversations                            the conversation list
 GET  /conversations/:id                        one conversation's metadata,
                                                its agents' included
 GET  /conversations/:id/agents/:node/transcript
 WS   /api/sync                                 one per page: the
                                                conversation list
 WS   /conversations/:id/stream                 one per open conversation:
                                                changes to its metadata, and
                                                the messages of the nodes the
                                                page subscribes to
```

* A node is named by its id everywhere; the root's id is the conversation's.
* On a conversation's connection the page subscribes to each node it shows,
  the root too, and ends the subscription when it stops showing it; only
  subscribed nodes' messages are sent.
* The routes and frames they replace go: `/subagents`, `/cost?agent=`,
  `?node=`, the root-only transcript frames and the `subagent_*` frames.

### Keeping a collection in sync

Every collection follows the same rules: the conversation list, a
conversation's metadata, each of its collections, and each node's messages.

```text
 each collection carries a version: an epoch and a revision

 a change arrives with its revision
   ├ it follows the page's revision   apply it
   ├ revisions are missing            ask for the changes since the page's
   └ the server cannot say            reset: what the page holds stays on
                                      screen, marked as a hole, and the shown
                                      part is read again
```

* The owner of a collection announces its changes; the page never polls.
* A change to one agent's metadata is sent as that agent's entry, never as
  the whole conversation.
* On opening, the page shows what it holds and sends each collection's
  version; the server answers each with its changes or a reset. Nothing
  bundles a first screen.

### What a conversation shows of its Host

Everything the conversation's side of the page shows that comes from a Host
reaches the page one way: the backend keeps it, and the page asks only the
backend. This holds for each such fact, today a directory's instruction
files and its workflows, and for any added later.

```text
 runner                      backend                       page
   watches exactly what  ──▶   reads the fact again,   ──▶   reads the kept
   a fact depends on,          stores it with when it        value; hears a
   reports a change            was read; a new value         new version like
                               raises the version            any collection
```

* A request is answered from what the backend holds; it never waits on a
  Host or wakes one. A Host that is offline, or a stopped Cloud, leaves the
  last value, with when it was read.
* A fact not read yet answers as unknown, and its first value is announced
  when it arrives.
* What the backend holds survives a restart of the backend, and is dropped
  once no conversation uses it.
* The runner's watch names only what the fact depends on, so a change to
  another file reads nothing again.
* Which paths a reply names are files is a fact of the reply: the backend
  reads it once through the Host when the reply ends and keeps it with the
  reply, as a reply's attachments are kept; nothing watches those paths.
* The work panel shows the Host's own files and changes, read from the Host
  when shown; it is outside this rule.

### A transcript on the page

```text
 node m3, by block index
   [1 ──── 40]   hole   [120 ──── 180]   hole   [300 ──── latest]
    first                                                 latest
```

* The page records which runs of blocks it knows to be whole; the gaps
  between them are holes. Lookups are by binary search.
* A new block joins only a run that reaches the latest block; otherwise it
  starts a run of its own.
* Only holes a shown window touches are filled, each once at a time.

### What keeps it fast

* A page of a transcript is bounded by its size, wherever requests begin and
  end; one long request is several pages. Every block names the request it
  belongs to, so any window, however it starts, finds each edit's request.
* Transcripts travel in the light form the page shows: a tool's output and a
  model's thinking are read when opened, never sent with the page.
* An agent's metadata carries no result: a result is its transcript's last
  answer, and the parent's transcript holds it as delivered.
* A cost total is kept as calls are recorded, so no read and no new call
  scans the usage ledger. The calls behind a cache chart are read when the
  popup opens, the recent ones only.
* Opening a conversation waits for nothing it does not show first: its
  messages show while its cost and other collections arrive.

### Memory and disk

* A view subscribes to one node around one place. It shows what the page
  holds there at once, then only changes.
* Memory holds the shown windows. Closing a tab ends its subscription, and
  the keyed sessions of `0008-plugin-architecture.md` keep recent ones.
* The disk (IndexedDB) holds every collection the page has read: the
  conversation list, each conversation's metadata and collections, and each
  node's blocks with its runs, read by node and block index one window at a
  time.
* A block is written once it is complete, never per streamed token.
* The disk is bounded at 500 MB per browser, or what the browser grants if
  less; the node viewed longest ago goes first, deleted in small batches
  when the page is idle, since one large deletion holds every read back
  until it ends, and a full disk evicts rather than fails.
* The disk copy replaces the offline copy of the latest window. The old copy
  is dropped, not migrated.

### Consequences

* Good, because a subagent tab, a reload and a return show what was seen at
  once, and catch up by changes alone.
* Good, because the main agent, subagents and side questions share one path
  for loading, caching and updating.
* Bad, because every layer between storage and the page changes at once.
* Modules touched: the agent server's frames and tree announcements, the
  backend's conversation routes, sync parts and kept Host facts, the runner's
  watch subjects, the protocol, and the page's conversation store,
  transcript views and offline storage.

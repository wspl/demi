---
status: proposed
---

# A conversation keeps each content once, and a format change rewrites no content

## Problem

Alex works with one agent for three weeks. The conversation's database
grows to 5.7 GB, while what it says, each distinct text and image once,
compressed, is about 1 GB. Then:

* An upgrade copies the 5.7 GB file before it migrates it, and a rollback
  copies it twice more: the file it replaces aside, and the snapshot back
  (`crates/server/src/data.rs`). A 40 GB server ran out of disk in the
  middle of a rollback, and neither release could start.
* The migration decodes and rewrites every row of the file in one
  transaction. Its write-ahead log reached 3.5 GB, and a version that
  collected the rows first ran out of 4 GB of memory at every start.
* Deleting a branch or a subagent frees nothing: the file never shrinks.

The size has two causes. First, a block is one JSON document that holds
every form of what it says, and each form is written out in full:

| Form in the block | Of the 3.56 GB of tool calls | Copy of |
| --- | --- | --- |
| The vendor's record of the session (`entries`) | 1.87 GB | The result text, word for word (0.35 GB); the input; the reasoning and its signature in reasoning blocks |
| Images inside the vendor's record, as base64 | 0.43 GB | Images the CLI re-encoded, so they match no medium of the block |
| The view's output (`view.chunks`) | 0.53 GB | The result text, without its first lines |
| The request's changed files (`view.requestFiles`) | 0.31 GB | Every earlier call's list, once more on each call |
| The result the model read (`output`) | 0.48 GB | Nothing: this is the one needed |

Second, a side question copies the rows of its source's window, the blocks
since the source's last compaction, as rows of its own: in the measured
conversation 185,265 of its 523,616 rows were such copies, 1.36 GB of the
file.

## Decision

Nothing on screen changes. A conversation takes about a fifth of the disk
it takes today, so an upgrade copies a fifth as much, and nothing of a
conversation it converts; a rollback copies only what migrated in place,
and deleting frees space.

### The store is lossless and the only one that knows

Every block reads back equal to the one written, and a session's file
written from it is the same byte for byte. Keeping each content once is
how the store encodes a block, and nothing outside
`crates/backend-database` sees it: the session, the providers and the
pages work with whole blocks as today. A rule that would read back
something else, such as keeping a request's file list on its latest call
only, is no part of the encoding.

### Contents and records

A **content** is bytes a block holds that something produced: a message's
text, a call's input and result, reasoning and its signature, a medium in
the vendor's record, a view's output, a file list. It is kept once per
conversation under the SHA-256 of its bytes, compressed, and never
changes. Which values of a block are contents:

* A string of at least 256 bytes. A shorter one costs less inline than its
  reference and the lookup.
* A string that is canonical base64 is kept as the bytes it encodes, and
  encoded again when it is read. No stored value holds base64.
* A string that is a piece of another content of the same block, as a
  shell view's output is a piece of the call's result, is kept as the
  range of that content it is, followed by the few bytes it adds.
* A block's model selection and a shell view's file lists (`files`,
  `runFiles`, `requestFiles`) are kept as their JSON. The lists of one
  request repeat from call to call until a call changes a file.

A **record** is the block's JSON with each content replaced by a marker
that names it, a string beginning with the control character U+0001:

```text
U+0001 c <sha256>                          a string
U+0001 b <sha256>                          base64, of the bytes kept
U+0001 j <sha256>                          a JSON value
U+0001 s <sha256> : <from> : <to> : <tail> a piece of a string, and a tail
```

A string of the block that begins with U+0001 is kept with another U+0001
before it, so no string reads as a marker. The record's body is compressed
with zstd and a dictionary the conversation trains on its own records once
it has a few hundred, and its columns hold what queries read: the block's
type, id, turn, command, status and the like, written from the body, never
differing from it.

```text
conversations/<conversation>.sqlite   blocks, nodes, path_runs, … (records)
                                      contents(id, hash, bytes)
                                      dictionaries(id, bytes)
blobs/<user>/<sha256>                 media and uploads, as today
```

* Records and contents are tables of the conversation's one database, so a
  block's record and its contents are written in one transaction, and a
  read sees both as they stood at one moment.
* A table beside the records lists the contents each record names, and
  one beside a share's copied rows the contents each copy names. A content
  is removed in the transaction that removes the last record or copy that
  names it, and the database refuses to remove a content still named.
* `contents` is a rowid table with the hash as a unique column. As a
  `WITHOUT ROWID` table keyed by the hash, each content over about 1 KB
  overflows and leaves its overflow page partly empty, which made the file
  half as large again.

### Forks

* A side question keeps the rows of its source's window as rows of its own,
  as today, since its session compacts and rewrites them as its own; it
  names the rows before the window and copies nothing of them, as today. A
  copied row costs only its record: its contents are the source's.
* A Fork into a new conversation copies the records of its source's rows
  up to the place it was forked at, and the contents they name, as the
  stored bytes, without decoding either. A conversation stays complete in
  its own database, as a Fork's command outputs, attachments and edits are
  copied today.

### Format changes

* A record carries its format. A release decodes each format it still
  knows and converts it as it reads; it writes only the current one.
  Contents have no format.
* A schema change adds or drops columns and never decodes every row. A
  format a release stops reading is first rewritten by maintenance, in the
  background, a few records to a transaction, so no write-ahead log grows
  with the conversation; no upgrade waits for it, and no content is
  rewritten.

### Space and upgrades

* A conversation's database uses incremental auto-vacuum, so a deletion
  and a collection return their pages.
* An upgrade copies the databases that migrate, as today, each a fifth of
  its size today.
* A rollback moves the databases it replaces aside by renaming each, and
  puts the snapshot's back by a hard link for one that converts, which no
  release writes then, and by a copy for one that migrates in place; today
  it copies both ways, so it needs room for two more copies of every
  database that migrated, at the moment the disk is fullest. The snapshot
  stays whole: an upgrade whose start fails returns, and when that return
  is interrupted the next command tries the upgrade again without a new
  snapshot, so a later return needs the same one.
* A return to a release from before this one is carried out by that
  release's own `demi-server`, which copies the snapshot back onto each
  database. A copy truncates its target first, so onto a database the
  snapshot still shares by a hard link it would empty both. The installed
  `demi-server` therefore gives each such database, one not converted yet,
  a copy of its own before it hands the return over.

### What an older server meets

The release converts each conversation's file into the new form, in the
background after it starts, most recently used first. A
conversation opened before its turn converts first, and the open waits for
it: well under a second for most, and minutes for the 5.7 GB one
([Spike](#spike)).

* The conversion streams the rows and keeps one in memory at a time. It
  writes a new file beside the old one, reads every row back and compares
  it with the original, and only then renames the new file over the old
  one. A conversion interrupted before the rename starts over; the old file
  is untouched until then.
* The conversion never changes the old file, so the upgrade's snapshot
  holds it by a hard link, which costs no space: a rollback returns it, and
  its space is returned when the snapshot is removed.
* The conversion reads the file of the release before this one as it is,
  and names in each row what that release's migration would have renamed.
  A file of an older release migrates first, as today, so the snapshot
  copies it.

## Spike

Measured on a copy of one 5.7 GB conversation, converted by a probe that
streams its 523,616 rows:

* What it holds: tool calls 3.56 GB, reasoning 1.0 GB (the vendor's record
  0.57 GB of it, the signature 0.29 GB). The result text inside the
  vendor's record equals the block's `output` byte for byte for every call
  sampled.
* Strings: 2,867 MB of strings of 256 bytes or more are 1,078 MB once
  each. The model selections and file lists are 599 MB in 881,772 values,
  and 53 MB in the 25,700 distinct ones. Of the 507 MB of views' output,
  341 MB is a piece of the call's result followed by at most two bytes;
  the rest is kept as a content of its own.
* Bodies: the records' JSON without its contents is 1,257 MB, 475 MB with
  zstd alone and 183 MB with a dictionary trained on 3,000 of them.
* Sizes, without the view's output and the file lists: 5.73 GB became
  1.12 GB, the records 409 MB and the contents 707 MB (413,861
  text contents, 894 MB compressed to 450 MB; 360 images, 99 MB). The
  conversion took 65 to 94 s and 58 MB of memory at most. Every sampled row
  read back equal to its original.
* Reads, warm and cold: the latest page of 1,000 blocks took 0.04 s and
  0.17 s, against 0.02 to 0.05 s and 0.09 s from the single file; the
  session file after the last compaction, 259 blocks, 11 ms against 5 ms;
  the whole path's, 47,698 blocks, 1.8 s and 4.0 s against 0.85 s and
  5.3 s, at the same peak memory. A warm read pays for decompression and a
  lookup per content; a cold read is faster, as it reads far less. The
  session files written from both were identical byte for byte.
* Collection: finding every reference took 6.3 s at 45 MB of memory.
  Deleting all 1,236 nodes but the root returned 336 MB of records in a
  2.8 s vacuum, and collecting the 362,905 contents nothing named then took
  23 s and returned 580 MB in a 9.3 s vacuum.
* A contents table keyed by the hash `WITHOUT ROWID` came to 1,044 MB
  against 707 MB as a rowid table.

The release's own conversion of the same copy, every row read back and
compared, in a development build:

* 5.73 GB became 1.25 GB: the records 402 MB with their indexes (their
  bodies 203 MB), the contents 713 MB with their index (432,155 contents,
  1,133 MB compressed to 584 MB), and the tables of what each record names
  28 MB.
* It took 14 minutes, against the probe's 65 to 94 s in an optimized build
  without the read-back; an optimized build of the release was not
  measured.

## Prior art

* Codex persists a session as an append-only JSONL rollout of the items the
  model reads, and marks the events that only draw the screen, such as a
  command's end, as never persisted. A fork or a revert starts a new
  rollout that names a position in the one it comes from (`history_base`)
  instead of copying its history. A background job compresses rollouts older
  than seven days with zstd and replaces each by a rename
  ([`codex-rs/rollout/src/policy.rs`][codex-policy],
  [`recorder.rs`][codex-forks], [`compression.rs`][codex-compression]).
* LangGraph's checkpointers keep each channel's value once per version in
  `checkpoint_blobs`, keyed by thread, channel and version, and a
  checkpoint names versions, so a value that did not change is not stored
  again ([`checkpoint-postgres/.../base.py`][langgraph]).
* Git stores each object once under its hash, compressed, and every tree
  and commit names objects; OCI images share content-addressed layers
  between images and keep each container's changes alone.

This record departs from them in keeping contents in SQLite per
conversation rather than in files, because most contents are a few
kilobytes and a file each would cost a block and an inode apiece; and from
Codex in copying a Fork's history into the new conversation, so that no
conversation's files outlive it for another's sake.

[codex-policy]: https://github.com/openai/codex/blob/main/codex-rs/rollout/src/policy.rs
[codex-compression]: https://github.com/openai/codex/blob/main/codex-rs/rollout/src/compression.rs
[codex-forks]: https://github.com/openai/codex/blob/main/codex-rs/rollout/src/recorder.rs
[langgraph]: https://github.com/langchain-ai/langgraph/blob/main/libs/checkpoint-postgres/langgraph/checkpoint/postgres/base.py

## Alternatives

* **Collect contents by a pass over the records.** No table of references
  to keep, but a pass reads every record, 6.3 s for the measured
  conversation, and a content stays until the next pass after its last
  record went.

* **Remove the copies and keep one file.** Referencing the vendor's
  record's strings, storing the CLI's images as blobs and keeping one
  list per request saves about 2 GB of the 5.7 GB. Every format change
  still rewrites every row, an upgrade still copies the whole file, and the
  next form added to a block is the next copy.
* **Compress the rows of today's file.** Five times smaller, but every
  query that reads a field of the JSON needs a column first, which is the
  split's work without its other gains, and a migration still rewrites
  everything.
* **Typed records, a struct per kind of block.** Every change of a block's
  shape would be a change of the records, and each kind needs its own
  split and join; the encoding of a block's JSON does the same for every
  kind at once.
* **A request's file list on its latest call only.** It saves no more than
  keeping each distinct list once, and it changes what a settled row reads
  back each time a later call is written.
* **A side question that names its source's window instead of copying it.**
  It would save the copies' records, about a third of the records, but a
  node's rows that another node holds are settled: the side question could
  no longer compact or rewrite its own window, and its next compaction
  would fail.
* **A Fork that names its source in another conversation.** It copies
  nothing, but every read of the history, the search index and the
  session file would span conversations, and deleting a source would have
  to wait for its last Fork, of another user's too. The copy costs the
  compressed records and contents of one path.
* **Records and contents in two files.** An upgrade that migrates the
  records would copy only them, 0.4 GB of the 1.1 GB. But no transaction
  spans two files in write-ahead-log mode: a write would commit its
  contents first and leave them behind when it failed, and a read would see
  each file at another moment.
* **A dictionary that ships with a format.** It would be trained on
  conversations that cannot be committed, or on invented ones that share
  little with real records.
* **One contents database per user.** Separate conversations would share
  contents, but a deleted conversation's contents would wait for a
  collection over every conversation, every conversation's writes would
  queue on one file, and the file would grow with all of them.
* **Contents as files in the blob store.** Most are a few kilobytes: a
  file each costs at least a 4 KB block and an inode, and on S3 a request
  each.

## Open questions

None.

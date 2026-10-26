---
status: proposed
---

# The offline copy outlives a release that keeps its format

## Problem

Alex's server takes a new release twice a day. After each one, Alex's page
reloads into the new web app and drops its whole copy on disk, because every
record of the copy names the build that wrote it. Every conversation Alex
then opens is read from the server again: on a server a 370 ms round trip
away, the first opening of each takes 3 to 9 seconds of Loading
conversation…, where a copy shows it in under 100 ms.

In a server's page log over two days, the openings the copy served showed
their first rows after 87 ms on average, and the ones read from the server
after 3.3 s. After the day's release, two of Alex's first three openings were
read from the server, though the blocks the copy keeps had not changed
shape.

## Decision

The copy is dropped when what it holds can no longer be read, not when the
build changes, and only the kind of record that changed shape goes.

* **Each kind of record has a format**: a hash of the JSON Schema of the
  schema its records are read with (`z.toJSONSchema`), computed once when
  first asked for:
  * the account's record: the product state's parts it keeps and the user;
  * a conversation's record: its metadata, its agents and its attachments'
    descriptions;
  * a node's record and its blocks together: its runs, length, version,
    instructions and summaries, and the block with its failure facts.
* **A record names its kind's format** where it names its build today, and
  a record of another format is dropped before its shape is read, as a
  record of another build is today. An account's record of another format
  drops that record alone, where today it drops the whole copy. A release
  that changes the agents' shape drops the conversations' records and keeps
  every block.
* **A conversation whose record went but whose root's blocks stay** shows
  the blocks from the disk at once and reads its metadata, its agents and
  its root's latest page, as one with no copy does: the descriptions of the
  attachments its replies name are in its record alone, and the latest page
  brings them.
* **What the backend counts in memory** keeps the run that counted it. A
  conversation's record names the backend's run its metadata was read in,
  and its permission requests' and plugin states' revisions are of that run,
  so the counts of a later run, which start again, replace them.
* **A plugin's state is in the plugin's own shape**, which no schema of the
  page's describes, so no format sees it change. A conversation's record
  names the build that wrote its plugin states, and a record of another
  build keeps the rest and drops them; the plugin reads its state from the
  backend.
* **A change of meaning that keeps the shape**, such as a field whose unit
  changes, raises the database's version, which drops every store, as a new
  layout does today. Nothing else needs a hand-kept number.
* **What the copy holds of a session** is followed as it is today: a version
  of a session the backend no longer runs, as after its restart, is answered
  as stale or rebased, and the page reads again.
* `cacheBuild` and the records' `build` field go; the development server,
  which names no build, keeps or drops its copy by the same rule.

## Spike

* **Whether every record's schema has a JSON Schema.** `z.toJSONSchema`
  converts all of them; the block's is 82 KB and the product state's 62 KB.
  They take about 10 ms together, once per page load.
* **How often a release changes a format.** The formats computed at each of
  the last 16 releases: of the 15 updates between them, the block's format
  changed in 3 and the agents' in 3, together in one; 10 changed neither.
  So the blocks, nearly all of the copy, would have stayed through 12 of the
  15 updates, where today none stays through any.

## Prior art

* TanStack Query's persisted cache drops a stored cache whose `buster`
  string differs, and its guide passes the build's hash
  ([persistQueryClient](https://tanstack.com/query/latest/docs/framework/react/plugins/persistQueryClient#cache-busting)),
  as Demi does today. Demi derives the string from what the copy stores
  instead, so a release that keeps the shape keeps the cache.

## Alternatives

* **A version kept by hand, raised when a record changes.** A change that
  forgets it is read by a schema that no longer fits, which drops records one
  by one as they fail to read, or misreads one whose fields only moved. The
  shape already says when it changed.
* **Migrating records between formats.** The copy is a cache that one read
  replaces; a migration would be code to keep for each change.

## Open questions

None.

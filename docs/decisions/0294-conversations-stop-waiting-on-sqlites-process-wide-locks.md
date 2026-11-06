---
status: proposed
---

# Conversations stop waiting on SQLite's process-wide locks

## Problem

A user runs 300 conversations at once on one backend, each a turn of a few
tool calls. A turn that takes 23 seconds alone takes 44 seconds, and a
request for a conversation's state, such as
`GET /api/conversations/{id}`, waits up to 4 seconds. The shard's thread is idle 86% of that time; the
backend's threads are waiting on mutexes inside SQLite.

Two causes, both process-wide, so each conversation slows every other:

- The bundled SQLite is compiled with `SQLITE_ENABLE_MEMORY_MANAGEMENT`,
  which libsqlite3-sys turns on. It puts the page cache of every connection
  in the process in one pool under one mutex. SQLite's memory statistics, on
  by default, take another mutex on every allocation.
- At most 64 writer connections are open at once, and the least recently
  used closes first. With 300 conversations writing, nearly every write
  closes another conversation's writer and opens its own. Opening and closing
  a database file takes the mutex of SQLite's Unix file layer, which every
  read transaction also takes for its memory barrier, and an open parses the
  schema again.

## Decision

Nothing changes on screen or in the API; turns and requests wait less.

- SQLite compiles without `SQLITE_ENABLE_MEMORY_MANAGEMENT` and with
  `SQLITE_DEFAULT_MEMSTATUS=0`, through `LIBSQLITE3_FLAGS` in
  `.cargo/config.toml`, so every build, a developer's and the release's,
  compiles the same SQLite. Each connection then has a page cache of its own.
  The two functions this gives up, `sqlite3_release_memory` and
  `sqlite3_memory_used`, are called nowhere in Demi.
- A conversation's writer stays open while its conversation is live on its
  shard, that is while its agent tree is in memory. The tree's store holds
  the writer and releases it when the tree is disposed, once no page has
  been attached and no agent has worked for ten minutes. Only the writers of
  conversations that are not live, opened for a rename, a read of an older
  transcript and the like, share the 64 places and close least recently used
  first.

```rust
struct Writers {
    /// The held files: none's writer closes while it is held. A file is
    /// here or in `open`, never in both.
    held: HashMap<DatabaseFile, Held>,
    /// The others, the least recently used closing first.
    open: LruCache<DatabaseFile, tokio_rusqlite::Connection>,
    closed: bool,
}

struct Held {
    holds: usize,
    /// None until a call opens it.
    writer: Option<tokio_rusqlite::Connection>,
}

impl ConversationDb {
    /// Keeps the writer open until the hold drops; the writer still opens
    /// at the first call.
    pub fn hold(&self) -> WriterHold;
}
```

A hold that comes for a writer in `open` moves it to `held`; the last hold
to drop moves it back to `open` as the most recently used. The open files
the writers take are then bounded by the live conversations, three files each
(the database, its write-ahead log and its shared memory), as their memory
already is, beside the 64 others. The backend raises its open-file limit to
the system's maximum at start, so the bound is the system's.

## Spike

Load runs of 2 minutes on one backend in release mode, every conversation
of one user and so on one shard, with a fake model that streams a turn
of four tool calls and a reply, 23 seconds alone:

| Conversations at once | Build | Turn p50 / p90 | Probe request p99 | Mutex waits in a 5 s stack sample |
| --- | --- | --- | --- | --- |
| 150 | before | 23.2 / 30.0 s | 403 ms | |
| 150 | flags | 22.7 / 27.2 s | 107 ms | |
| 150 | flags, 1,024 writers | 22.6 / 26.4 s | 81 ms | |
| 300 | before | 43.7 / 51.5 s | 3,809 ms | 39,536 |
| 300 | flags | 24.4 / 28.7 s | 658 ms | 30,156 |
| 300 | flags, 1,024 writers | 23.1 / 26.2 s | 145 ms | 272 |

The writer limit of 1,024 stood in for "every live conversation's writer
stays open". With it, 300 conversations used 3,400 open files and 570
threads. The stack samples showed that a conversation read without a writer,
on a connection of its own, as a summary is, took under 1% of the time, so
those reads keep opening their own connection.

## Prior art

- workerd, Cloudflare's runtime for Durable Objects, keeps one SQLite
  database per object and bounds what is open by activity, not by count: an
  object idle for ten seconds is shut down and its container dropped after
  seventy
  ([`src/workerd/server/server/actor.rs`](https://github.com/cloudflare/workerd/blob/main/src/workerd/server/server/actor.rs)).
  Demi keeps the ten minutes it already gives an idle tree.
- SQLite's page cache (`pcache1.c`) gives each connection a cache of its own
  unless `SQLITE_ENABLE_MEMORY_MANAGEMENT` is defined, which shares one
  among all; the shared one exists so that `sqlite3_release_memory` can free
  pages of any connection. rusqlite users such as
  [khive-db](https://docs.rs/crate/khive-db/0.12.0/source/tests/sqlite_compile_options.rs)
  build the bundled SQLite with the same two flags through
  `LIBSQLITE3_FLAGS`.

## Alternatives

- A larger fixed writer limit, or one derived from the open-file limit.
  Either closes the writer of a conversation that is running once enough
  others run, which is the thrash this removes, or keeps the writers of
  conversations idle for days open.
- A pool of read-only connections per conversation. The reads that open
  their own connection were under 1% of the samples.
- Leaving the flags and only keeping writers open. The flags alone took the
  300-conversation turn from 44 to 24 seconds.

## Open questions

None.

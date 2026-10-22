---
status: proposed
---

# One log tells what happened, in the backend, on every Host and in the page

Supersedes: the Host's log read (`GET /devices/:id/log`,
`crates/runner/src/host_log.rs` § How lines get in), the server log read
(`GET /server/log`, `crates/backend-http/src/diagnostics.rs`) and the page's
error reports (`POST /page-errors`), which this record replaces.

## Problem

A user switches back to a conversation and its Requesting timer starts again
from zero. Nothing failed, so nothing was logged: the backend's lines for
that minute are the HTTP requests' (`request_id`), and the page, where the
timer lives, sends only its failures. The log could not say why.

Other recent problems had the same shape: a command left silent for ten
minutes did not wake its agent; a yamux stream stalled; a subagent's
transcript loaded older pages without end. Each was a wait that never ended
or a path taken for the wrong reason, and none left a line.

Measured on the development server over the last seven days (2026-10-15 to
2026-10-22):

| What | Value |
|---|---|
| Lines | 709,486 (about 80,000 a day), 245 MiB |
| By level | info 702,721; warn 6,485; error 279 |
| Per-request lines (`demi_backend_http::request_id`) | 496,002 (70%) |
| Usage and Claude Code timing lines | 169,042 (24%) |
| Busiest second | 314 lines |

In the code, 445 events are warn or error and 178 are info or debug. The
backend's lines carry request, user, conversation and turn IDs, but a Host's
lines are text with a source and a conversation, in a file on the Host that
is lost with a deleted Cloud, and the page's are not kept at all.

## Decision

Demi keeps few tests, so a problem is diagnosed from the log, after the fact,
without reproducing it. Today the log records mostly failures, and three
places keep it apart: the backend's database, a file on each Host, and the
page, which keeps nothing. After this change the backend's `log.sqlite` holds
every line, the backend's, each Host's and each page's, under the same IDs.
Each unit of work ends with one line that says how it went, and each state
change, wait and choice of path has its line, so a problem that raises no
error still leaves a trace. Whoever diagnoses reads the database on the server
with `sqlite3`; the two read routes are removed.

### An example

A user reports that the Requesting timer restarted when they switched back
to a conversation. The agent diagnosing it runs, on the server:

```sh
sqlite3 -json /opt/demi/data/backend/log.sqlite "
  SELECT at, origin, level, event, fields FROM events
  WHERE conversation_id = 'cv_1f3k...' AND at BETWEEN 1761140000000 AND 1761140120000
  ORDER BY at"
```

and reads, in order, the page's, the backend's and the Host's lines of that
minute:

```text
page     info  view.opened          {"view":"conversation","from":"settings"}
page     info  read.answered        {"route":"/conversations/:id/transcript","version":512,"ms":84}
backend  info  socket.opened        {"follows":2}
backend  info  follow.answered      {"value":"transcript","held":512,"current":530,"sent":"patches"}
page     info  timer.started        {"timer":"requesting","since":"page","elapsed_ms":0}
```

The last line shows the page started the timer from its own clock instead of
the backend's request start: the cause, without reproducing it.

### What is logged

These rules decide what is written at `info`, the level the server keeps by
default; `debug` and `trace` add detail that `DEMI_LOG` turns on.

1. **A unit of work ends with one line** that says how it went: its kind, its
   IDs, its outcome, its duration, and the counts that describe it. The units
   are an HTTP request (as today), a socket's life, a turn, a model request
   (provider, model, tokens in and out, time to the first token, outcome), a
   Host operation (operation, Host, outcome, duration), a command (its exit,
   its duration, whether it moved to the background, how often it woke its
   agent), a compaction pass, a page read and a page socket's life.
2. **A wait that may not end logs its start and its end** under the same ID:
   a call waiting for a Host, a permission request, a provider's first byte, a
   command's wake timers, a socket waiting to reconnect. A start with no end
   is the trace of a wait that never ended.
3. **A state machine logs every transition**, with `from`, `to` and the
   cause: an agent node's status, a command's state, a runner connection's
   state (connected, tolerating a drop, offline), a followed value's state,
   the page's connection state.
4. **A choice of path logs which path and why**, where the paths differ in
   what the user sees: a follow answered with patches, the whole value or
   `stale`; a request retried; a fallback taken.
5. **A limit that was hit is logged**: a queue that dropped lines, a rate
   limit, a buffer that overflowed, a page that was cut.

A duration is written by whoever timed it, as an integer field named for
what it measures and ending in `_ms`, such as `ms`, `first_token_ms` or
`queue_ms`; a unit whose time splits into waiting and working writes each
part. It is never left to the difference of two lines' `at`: the page's, the
backend's and a Host's clocks differ, a start line may have been trimmed or
dropped, and a duration in one field is summed and compared with one query.

An event that happens many times a second, such as a transcript patch, a
stream delta or a scroll frame, is `debug`; at `info` its unit's end line
carries its count. At `info` a day stays near 400,000 lines, which 14 days
and 2 GiB hold.

What is never logged, as today: a request's or a message's body, the text a
user typed, a model's output, page content, a file's contents, a credential
or a secret's value, a cookie, a token. A line names things by ID and
counts their sizes.

### One line, wherever it comes from

```rust
/// One line of the log, as the backend stores it, whatever wrote it.
pub struct LogLine {
    /// Milliseconds since the Unix epoch, by the writer's clock.
    pub at: i64,
    pub origin: LogOrigin,
    pub level: LogLevel,
    /// What happened, as `noun.verb` in the past tense, such as
    /// `command.ended` or `follow.answered`; the `tracing` target for a
    /// backend event that names none.
    pub event: String,
    pub request_id: Option<String>,
    pub user_id: Option<String>,
    pub conversation_id: Option<String>,
    pub turn_id: Option<String>,
    /// The Host whose runner wrote it, or that a backend line is about.
    pub host_id: Option<String>,
    /// The page load that wrote it, or whose request a backend line serves.
    pub page_id: Option<String>,
    /// The span's and the event's other fields: IDs, counts, durations,
    /// states. Text a person wrote never goes here.
    pub fields: serde_json::Map<String, serde_json::Value>,
}

pub enum LogOrigin { Backend, Runner, Page }
```

`log.sqlite`'s `events` table gains `origin`, `host_id`, `page_id` and `seq`,
and `message` becomes `event`, with indexes on `(host_id, at)`, `(page_id, at)`
and, for the cursor of a Host's lines, `(host_id, seq)`; `origin` and `level`
are text, so that a query reads as the examples here do. The log is not state: a database of the old schema is
removed and created again, as a database the writer cannot open is today.

The writer keeps lines for 14 days and at most 2 GiB, in place of 7 days and
512 MiB; the hourly trim keeps the newer, deleting in batches of 10,000
lines so that the writer is never held for more than a fraction of a
second. `DEMI_LOG_KEEP_DAYS` and
`DEMI_LOG_MAX_SIZE` (bytes, or a number with `MiB` or `GiB`) set other limits,
as `DEMI_LOG` sets the level.

### The Host's lines reach the backend

A runner's `tracing` events become `LogLine`s with `origin: Runner` and their
fields, kept in the Host's two files as today (4 MiB each), which now hold
JSON `LogLine`s with the file's `seq`. The files are a buffer: they keep the
lines while the backend cannot be reached.

```rust
/// Backend to runner, after the hello: send the lines after `since`, then
/// each new line as it is written. `since` is the highest `seq` the backend
/// stored for this Host, none for a Host it has no line of.
LogFollow { since: Option<u64> }

/// Runner to backend: lines in `seq` order. `missed` counts the lines the
/// files no longer held, or that the queue dropped, since the last frame.
LogLines { lines: Vec<HostLogLine>, missed: u64 }

pub struct HostLogLine { pub seq: u64, pub line: LogLine }
```

The backend stores each frame's lines with the Host's ID and its `seq` (a
column of runner lines), and logs `log.missed` with the count when it is not
zero. A runner is a Host's program, so the backend does not take its word for
who a line is about: the line's user is the Host's owner; a request, a page or
a turn ID the line names is not taken, since nothing can check it. The follow
resumes from the highest `seq` stored for the Host, so the runner sends no line
below it. The runner sends
a frame of at most 32 KiB (a line over it is alone in its frame): a frame being
written cannot be interrupted, and the messages behind it wait for it. Log frames
go on the runner connection's lowest-priority queue, behind its control and
output queues, so a slow uplink sends a command's exit or an answer before
a log frame; a command's output itself never crosses the connection (the
backend reads it from the Host's kept output).

A line of a resident service's standard error that is not a `LogLine` is kept
with `event: "stderr"` and its text in `fields.text`, as today's lines are.

### The page's lines reach the backend

The page names its load with an ID, `pg_` and 16 characters as a request ID
is made, and sends it as `X-Demi-Page` on every request and as `page=` in a
socket's URL. The backend puts it on the request's span, so the backend's
lines of a request the page made carry the page's ID.

```ts
/** `POST /log`: lines the page wrote, oldest first. */
interface PageLog {
  page: string
  lines: Array<{
    at: number
    level: 'debug' | 'info' | 'warn' | 'error'
    /** `noun.verb`, as the backend's. */
    event: string
    conversationId?: string
    /** Scalars only; nothing is cut. */
    fields: Record<string, string | number | boolean | null>
  }>
}
```

- **Answer:** `204`, with `X-Request-Id` as every answer. `400` for a body
  that does not parse. The route has no limit of its own on lines, their
  fields or their rate, and nothing is cut: Demi can work with the log as it
  comes, and the body is held to the backend's JSON body limit like any
  other. The page sends its lines in requests under half of it.
- **When the page sends:** every 5 seconds while it has lines, and at once
  for an `error` line. A page that is signed out, or waits for its password
  to be set (`401`, `403`), sends nothing and keeps its lines until it is
  signed in. When the page is hidden (`pagehide`, or `visibilitychange` to
  hidden, whichever comes first) it sends one request with `keepalive`, its
  lines cut to 16 KiB: a browser caps the `keepalive` bodies in flight of a
  page at 64 KiB in total, and the drafts and the preferences the page saves
  as it closes are sent that way too and need the rest. What does not
  fit is counted in a `log.dropped` line the next load sends, once.
- **What the page logs** follows the five rules above, for its own units:
  each read and its answer, its sockets and its connection state, a view's
  opening and what opened it, a followed value's version, a timer's start and
  where its start time came from, a list's scroll anchor being corrected.
- `reportError` writes an `error` line and shows the `X-Request-Id` of the
  request that carried it, as it shows the one `/page-errors` answered today.
- `POST /page-errors` is removed.

### Reading the log

`GET /server/log` and `GET /devices/:id/log`, their protocol types and the
runner's `log_read` are removed. Whoever diagnoses, a person or an agent with
a shell on the server, reads `log.sqlite` with `sqlite3`, by conversation,
turn, request, Host, page or time. `docs/delivery/diagnosis.md` says where
the file is, its columns and a few queries.

### Older parts

- **Old and new runners and backends.** The wire version changes (`57`): a
  runner and a backend of different versions refuse each other's hello, and
  every upgrade replaces the runners. The runner's lines stay in its files
  while it cannot connect, and are sent after the hello that works.
- **An open page of the old build.** Its `POST /page-errors` gets `404`,
  which it ignores; it reloads into the new web app after the upgrade.
- **The data.** `log.sqlite` of the old schema is removed and created again.
  The development server loses its seven days of log at the upgrade.

## Spike

The code is throwaway and outside the repository.

1. **The writer.** The server log's layer and writer as they are, with this
   record's columns and indexes, fed by four threads through `tracing`, on a
   database already holding 1.1 million lines (14 days):

   | Lines a second | Stored | Dropped | Commit p50 / p99 |
   |---|---|---|---|
   | 314 (today's busiest second) | 312 | 0 | 1.5 / 24 ms |
   | 5,000 | 4,999 | 0 | 46 / 59 ms |
   | 20,000 | 19,997 | 0 | 120 / 153 ms |
   | 40,000 | 32,384 | 19% | 118 / 183 ms |

   The new columns and indexes cost nothing measurable. A query by
   conversation takes 5 ms, by page 1 ms, by Host for an hour under 1 ms; a
   filter on `event` or inside `fields` scans the table, 0.2 s and 1 s at
   1.1 million lines. A line is about 385 bytes, so 2 GiB holds about 5.6
   million. Deleting a tenth of a full 2 GiB log at once holds the writer
   3.3 s, hence the batches. A turn of ten commands, a Host and ten minutes
   of a page write about 300 to 400 lines under the five rules, and about
   1,200 if each transcript patch were an `info` line, hence the rule on
   frequent events. Measured on a laptop's SSD, not the server's disk.
2. **Log frames beside a command.** On today's runner connection, a command
   writing 64 MiB took 1.11 to 1.39 s with no log frames and with 1 MiB or
   10 MiB a second of 258 KiB log frames on the control, output or a new
   lowest-priority queue: no difference beyond the runs' noise. The output
   does not cross the connection; what a log frame can delay is a message
   behind it while it is written, hence frames of 32 KiB on the lowest
   queue.
3. **The page's last lines.** `fetch` with `keepalive` on `pagehide` or
   `visibilitychange` delivered on closing a tab, reloading and navigating,
   in Chrome 153 and 155 and in Safari 27. A plain `fetch` was lost in
   Safari every time. Both browsers refused a `keepalive` body over 64 KiB,
   and a request taking the page's `keepalive` bodies in flight over 64 KiB
   in total. Not run: Safari on iOS, Firefox, quitting the browser.

## Prior art

- **Wide events at a unit's end.** Stripe's canonical log lines; Coder's HTTP
  middleware, which adds fields during a request and writes one line at its
  end (`coderd/httpmw/loggermw/logger.go`); Honeycomb's wide events. Demi's
  per-request line already works this way; this extends it to the other
  units.
- **What `info` holds.** Kubernetes' logging conventions put steady-state
  information, HTTP requests and state changes at its default level, and
  details below it.
- **Durations split by waiting.** Zed's collaboration server records each
  message's queue and processing time on its span (`crates/collab/src/rpc.rs`).
- **A remote process's lines join the local log.** Zed's remote server
  writes JSON lines that the client parses into its own log, keeping a line
  that does not parse with a prefix (`crates/remote/src/transport.rs`). Demi
  does the same with the runner, into the database, and replays what a
  disconnect held back.
- **A front end's lines.** Grafana Faro batches every 250 ms or 50 items;
  PostHog limits a client to 10 events a second and tags requests with a
  session ID header; Sentry keeps a ring of 100 breadcrumbs. Demi batches
  every 5 seconds, tags requests with the page's ID, and keeps no ring:
  lines are sent as they come, because the problems are silent.
- **Departure.** None of these let the logs of a backend, its remote
  processes and its pages be queried together by one ID in one place without
  a separate log stack; Demi can, because it owns the database.

## Alternatives

- **Keep each Host's lines on the Host and read them through the runner**
  (today). A Host that is offline, asleep or deleted cannot be read, which is
  when its lines are wanted, and a problem across the backend and a Host is
  read in two places by two clocks.
- **Send the page's lines only with an error**, as Sentry's breadcrumbs. The
  problems this is for raise no error.
- **OpenTelemetry and a log stack** (a collector, Loki or similar). A user
  who installs Demi would have to run them; Demi already has its database.
- **A `demi logs` command or a query route.** An agent with a shell on the
  server reads SQLite with any query it needs; a route would offer a few
  filters and need keeping.

What doing it costs:

- More lines to write and to keep, for everyone, to diagnose problems that
  most users never meet.
- A Host's lines now leave the Host. They hold IDs, counts and states, never
  content, but they are on the server.
- Reading the log needs a shell on the server.

## Open questions

None.

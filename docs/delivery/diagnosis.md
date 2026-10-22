# Diagnosing a problem from the log

Demi keeps few tests, so a problem is diagnosed from the log, after the fact,
without reproducing it. The backend's `log.sqlite`, in its data directory
(`<DEMI_BACKEND_DATA>/log.sqlite`), holds every line of the backend, of each
Host's runner and of each page, under the same IDs. Whoever diagnoses, a person
or an agent with a shell on the server, reads it with `sqlite3`. No route reads
it.

For example, a user reports that the Requesting timer restarted when they
switched back to a conversation. The agent reads the lines of that conversation
for the minute, in order:

```sh
sqlite3 -json "$DEMI_BACKEND_DATA/log.sqlite" "
  SELECT at, origin, level, event, fields FROM events
  WHERE conversation_id = 'cv_example' AND at BETWEEN 1761140000000 AND 1761140120000
  ORDER BY at"
```

and sees the page's, the backend's and the Host's lines together, each by its
own writer's clock.

## The columns

The `events` table has one row per line, whatever wrote it:

| Column | Holds |
| --- | --- |
| `at` | Milliseconds since the Unix epoch, by the writer's clock: the page's, a Host's or the backend's differ |
| `origin` | `backend`, `runner`, `page` or `machine_manager` |
| `level` | `trace`, `debug`, `info`, `warn` or `error` |
| `event` | What happened, as `noun.verb` in the past tense, such as `command.ended`; the `tracing` target for a backend event that names none, with its message in `fields.message` |
| `request_id` | The HTTP request, a socket's upgrade included. A page's line carries the request that brought it to the backend |
| `user_id`, `conversation_id`, `turn_id` | The user, conversation and turn the line is about |
| `host_id` | The Host whose runner wrote the line, or that a backend line is about |
| `page_id` | The page load that wrote the line, or whose request a backend line serves |
| `seq` | A runner line's number in its Host's files; the backend asks for the lines after the highest it stored |
| `fields` | JSON: the span's and the event's other fields. A duration is an integer in a field that ends in `_ms` |

A line never holds a request's or a message's body, text a user typed, a
model's output, page content, a file's contents, a credential or a cookie. It
names things by ID and counts their sizes.

## Queries

By request, the ID that a toast or an error shows:

```sh
sqlite3 -json "$DEMI_BACKEND_DATA/log.sqlite" \
  "SELECT at, origin, level, event, fields FROM events WHERE request_id = 'req_example' ORDER BY at"
```

By page, what one page load wrote and what the backend did for its requests:

```sh
sqlite3 -json "$DEMI_BACKEND_DATA/log.sqlite" \
  "SELECT at, origin, level, event, fields FROM events WHERE page_id = 'pg_example' ORDER BY at"
```

By Host, an hour of its runner's lines and the backend's lines about it:

```sh
sqlite3 -json "$DEMI_BACKEND_DATA/log.sqlite" \
  "SELECT at, origin, level, event, fields FROM events WHERE host_id = 'host_example' AND at > 1761140000000 ORDER BY at"
```

The warnings and errors of the last hour:

```sh
sqlite3 -json "$DEMI_BACKEND_DATA/log.sqlite" "SELECT at, origin, event, fields FROM events
  WHERE level IN ('warn', 'error') AND at > (strftime('%s', 'now') - 3600) * 1000 ORDER BY at"
```

The slowest requests, by the duration each wrote itself:

```sh
sqlite3 "$DEMI_BACKEND_DATA/log.sqlite" "SELECT json_extract(fields, '$.duration_ms') AS ms, request_id,
  json_extract(fields, '$.path') FROM events WHERE event = 'request.answered' ORDER BY ms DESC LIMIT 20"
```

A query by conversation, request, Host or page uses an index. A filter on
`event` or inside `fields` reads the whole table, which takes a second or so
on a million lines.

## What the log keeps

- **Level.** The server keeps `info` and above by default; `DEMI_LOG` selects
  more, as `info,demi_backend_http=debug` does. A page's lines are held to it as
  the target `page`, so `info,page=debug` keeps the page's `debug` lines only.
  An event that happens many times a second, such as a transcript patch, is `debug`, and its unit's end
  line at `info` carries its count.
- **Time and size.** 14 days and 2 GiB, the newer kept. `DEMI_LOG_KEEP_DAYS`
  and `DEMI_LOG_MAX_SIZE` (bytes, or a number with `MiB` or `GiB`) set other
  limits. The hourly trim deletes in batches of 10,000 lines, so the writer is
  never held for long.
- **A limit that was hit.** The log says so itself:
  - `log.dropped`: lines the writer's queue had no room for, or that a page
    could not send, with their count.
  - `log.missed`: lines a runner no longer held in its files when the backend
    asked for them.
  - `log.trimmed`: a trim, with the lines it deleted and how long it took.
  - `log.recreated`: a database that could not be opened or was of another
    schema, which is removed and created again, since the log is not state.

## How lines get in

- **The backend's** `tracing` events become lines through a layer, and a writer
  thread commits them every 200 milliseconds.
- **A runner's** events are kept in two files on its Host (4 MiB each), and the
  backend asks for them after the hello with `log_follow`, from the highest
  `seq` it stored for the Host. The runner sends them in frames of 32 KiB,
  since a frame being written cannot be interrupted, behind its answers and
  its commands' exits. A Host that is offline keeps its lines in its files
  until it connects again. The backend stores them as the Host's owner's: a
  request, page or turn ID a runner names is not taken.
- **A command package's service** writes each `tracing` event as one JSON line
  of the log on its standard error (`demi_command_sdk::log_to_stderr`), and its
  runner keeps the line as it is among its own, so a service's line names its
  conversation and invocation as any other. A line of its standard error that is
  not one, as a panic's report, is kept with the event `stderr` and its text in
  `fields.text`.
- **The Cloud machine manager's** `tracing` events go to its journal, and each
  of `info` and above is also sent as a line on every connection the backend
  has open to it; the backend stores it with the origin `machine_manager`. A
  line made while no connection is open stays in the journal alone.
- **A page's** lines go to `POST /log` every 5 seconds while it has lines, at
  once for an error, and in one request when the page is hidden, which the
  browser finishes after the page is gone and which is small (16 KiB) because
  a browser caps a page's `keepalive` bodies in flight at 64 KiB in total,
  and the drafts and preferences saved as the page closes need the rest. A
  page that is signed out keeps its lines until it is signed in. Every request of a page carries its
  ID in `X-Demi-Page`, and every socket in its URL's `page` parameter.

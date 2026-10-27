---
status: proposed
---

# A resident's start outlives the stream that asked for it

## Problem

Alex opens a simulator's panel for the first time on a Mac whose runner has
not yet downloaded `demi.mobile`, closes the panel within a second, and opens
it again. The panel shows "Downloading… 0 B of 82 MB" and never moves. Each
time Alex opens it again it fails, and the runner's log says `native runtime
was cancelled`, until the runner restarts. The same happens when a server
upgrade changes a package's program while a panel is open: in a test of ten
upgrades with a panel open, three left the panel stuck.

Two gaps cause it:

1. **The runner never learns that the backend gave up an opening stream.** A
   user stream that goes through the backend opens with `resident_open`. While
   the runner starts the stream's resident, which may mean downloading its
   program, the stream is opening: its pipes connect only once it opened, and
   only the connection's end cancels it. When the page goes away meanwhile,
   the backend drops the stream and, by design, stops answering its requests
   for the program's location. Nothing tells the runner.
2. **A resident's start asks only the first caller.** Callers that want the
   same resident share one start. The start locates the program through the
   first caller's resolver, which asks the backend on behalf of that caller's
   stream, and keeps it after that caller left. The cache's shared download
   already hands the question to the next caller still waiting, but the
   registry is its one caller, so that rule never applies.

Together: the first stream is dropped on the backend, its location request is
never answered, and every later stream joins a start that waits forever.

## Decision

Both gaps close. Nothing changes on screen; the panel opens.

### The backend ends the stream on the runner

The wire gets one message to the runner:

```rust
/// The backend gave up a user stream: the runner ends it, opened or not.
ResidentClose { stream_id: String },
```

- The backend sends it when it drops a user stream it opened
  (`ResidentStream`'s drop in `backend-remote-host`), whether the runner
  answered `resident_opened` yet or not. A one-shot user call drops its stream
  the same way.
- The runner cancels the stream's own token (`runner-jobs`'
  `ResidentStreams`): an opening stream stops waiting for its resident and
  leaves the start, and an open stream ends as a failing pipe ends it today.
  A stream it no longer holds is nothing to end.
- The wire's `VERSION` goes from 65 to 66. A runner and its backend already
  move together; an older runner's hello is refused, as for every version.

### A resident's start asks any caller still waiting

The registry's start of a resident locates its program through the first
caller still waiting, and hands the question to the next when that one
leaves; with none left, the start stops, as it does today once nothing leases
the resident. Each caller joins with its resolver and its own cancellation
(`ResidentHandle::acquire` passes the caller's token in).

This is the rule the cache's shared download already kept. It moves into
one type both use (`runner-command-packages`' `askers`), so it has one
implementation: the cache's shared download and the registry's start each
keep their waiting callers in it, and it is the resolver either hands on.

## Spike

A spike of both parts ran on the development setup with an iOS simulator's
panel open, with 3 s added to the backend's answer for a program's location,
as a slower store gives in production.

| Scenario | Neither | Close only | Handover only | Both |
|---|---|---|---|---|
| Server upgrade with a panel open | 3 of 10 stuck | 3 of 6 stuck | stuck in the first | 0 of 6 stuck |
| First use, closed after 1 s, reopened | stuck | (not run) | (not run) | opens |

With both, five of the six upgrades met the race, and the start moved to the
next caller each time.

## Prior art

- HTTP/2 lets either peer end a stream in any state, before a response
  included, with `RST_STREAM`
  ([RFC 9113 §6.4](https://www.rfc-editor.org/rfc/rfc9113#section-6.4)), and
  gRPC carries a client's cancellation to the server, which stops the call's
  work ([Cancellation](https://grpc.io/docs/guides/cancellation/)). The backend
  ending a stream on the runner is the same.
- Work that several callers share runs on behalf of none of them in
  particular; a caller that leaves takes only itself out. The cache's shared
  download already follows this, and the registry's start now does too.

## Alternatives

- **The backend answers with a refusal instead of silence.** The start then
  fails for every caller waiting on it, each panel shows an error once, and
  the stream the backend dropped still waits on the runner, holding its
  resident's lease.
- **A deadline on the location request.** A panel stays stuck for the
  deadline, a fixed limit Demi does not need.
- **The runner watches an opening stream's pipes.** Its pipes connect only
  once the stream opened, by design: no bytes move before the answer.

## Open questions

None.

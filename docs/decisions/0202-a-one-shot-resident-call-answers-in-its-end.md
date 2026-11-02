---
status: proposed
---

# A one-shot resident call answers in its end message

Supersedes: 0047 (A runner link sends control before input, and nothing on
it is cut off by a count), the part that says no output crosses the link but
one frame per stream: a one-shot call's whole answer now does.

## Problem

Alex works in a conversation whose Host is a Mac at home, about 180 ms from
the server each way. Alex types `https://example.com/docs` into the browser
panel's address bar and presses Return. The browser on the Host starts
loading at once, but the page's call that asked for it answers only about
1.5 seconds later. A panel tab that reopens after its browser closed waits
the same extra time before the page can show anything, and so does every
other call the page makes to a package on the Host, such as Back, Reload or
the work panel's tab list.

The work on the Host takes no time: the runner logs `browser.open` with
`ms: 0`. The wait is the way a one-shot call (`RemoteHost::call_resident`)
returns its answer:

```
backend                                  runner
  | resident_open ----------------------->|  starts the invocation
  |<--------------------- resident_opened |  invocation ends at once
  |<===== output pipe: WebSocket upgrade ==|  1 round trip to connect
  |<===== answer bytes, then close ========|  1/2
  |====== close answered ================>|  1/2
  |<----------------------- resident_done |  1/2: sent only now
```

From the moment the invocation ends, the answer needs two and a half round
trips instead of a half: the runner opens a WebSocket for the output pipe,
sends the answer and its close on it, waits for the backend to answer the
close, and only then sends `resident_done`, which the backend waits for. On a
link with 180 ms each way that is about 0.9 seconds; through Cloudflare,
where the pipe's connection also needs a TLS handshake, more.

## Decision

A one-shot resident call carries its answer in `resident_done`. It opens no
output pipe, and the runner sends `resident_done` as soon as the invocation
completes:

```
backend                                  runner
  | resident_open (no output pipe) ------>|  starts the invocation
  |<--------------------- resident_opened |  invocation ends at once
  |<--------- resident_done, with stdout -|  1/2
```

A user stream, which the page reads as it comes, such as the live view or a
recording, keeps its output pipe. Input keeps its pipe too: an invocation
reads it only when it asks for it, and a browser call has none.

### Design

- `runner-protocol`, `messages.rs`:
  - `ResidentOpen::output` becomes `Option<PipeRef>`. None names a one-shot
    call.
  - `ResidentDone` gains `stdout: WireBytes`, empty for a stream. It holds at
    most `RESIDENT_ANSWER_BYTES`, 1 MiB, the largest answer either caller
    takes today (`CALL_ANSWER_MAX_BYTES` for package calls,
    `MAX_ANSWER_BYTES` for the Claude Code package).
  - `ResidentError`, which said only why a stream did not open, also ends a
    one-shot call that opened and cannot complete, with the new code
    `answer_too_large` or with `resident_failed`. A stream with an output
    pipe learns these from its pipe, as before.
  - The module's rule that command output never travels in a message gains
    its exception: a one-shot answer of at most 1 MiB, a quarter of the
    largest message, which holds up the link for at most that many bytes.
- `runner-jobs`, `streams.rs`: a stream without an output pipe collects the
  invocation's standard output in memory up to `RESIDENT_ANSWER_BYTES`. More
  stops the collection, which fails the invocation's next write, and ends the
  call with `resident_error` code `answer_too_large`. An invocation that ends
  without completing, as when its resident fails, ends it with
  `resident_failed`. `resident_done` goes out once the invocation completes.
- `runner-client`: `call_resident` opens no output pipe and returns
  `resident_done`'s `stdout`, checked against its `max_bytes`; a
  `resident_error` after the call opened ends its wait, and
  `answer_too_large` is its `TooLarge`.
- A runner of an older release cannot read a `resident_open` without an
  output pipe. The protocol version rises, as every protocol change does,
  and the backend upgrades the runner before it calls it.

## Spike

- Where the time goes. A user `navigate` call from the page, on the bench
  with 180 ms each way on both the page's and the runner's link, answered in
  1,450 to 1,462 ms in four runs. The page's round trip to the backend and the
  backend's to the runner account for 720 ms of it; the runner logged 0 ms of
  work. The 730 ms left match the output pipe's connection, its close and the
  late `resident_done`.
- What a smaller fix would save. Sending `resident_done` without waiting for
  the close's answer saves one round trip, about 360 ms on the bench; the
  backend still waits for the pipe's connection and its bytes, about 540 ms.
  Answering in `resident_done` leaves 180 ms.
- What the decision measured, built, on the same bench: the same `navigate`
  call answered in 727 to 733 ms in five runs, the two round trips alone. A
  panel tab's `bind` after its browser closed, which opens a tab with one
  call, answered in 1.36 to 1.76 seconds, against 1.70 to 1.92 before.

## Prior art

- The [Language Server Protocol](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#responseMessage)
  and the [Chrome DevTools Protocol](https://chromedevtools.github.io/devtools-protocol/)
  answer a request in its response message, over the connection that
  carried the request. CDP hands out a stream to read in chunks (`IO.read`)
  only where a caller asks for one, as `Page.printToPDF` does with
  `transferMode: ReturnAsStream`.
- [VS Code's remote extension host](https://code.visualstudio.com/api/advanced-topics/remote-extensions)
  answers the window's RPC calls over the connection it keeps to the window,
  not over a connection of each call's own.

## Alternatives

- **Send `resident_done` without waiting for the pipe's close.** Smaller and
  keeps the rule that output never travels in a message, but saves only a
  third of the wait.
- **Keep one output connection open per device and reuse it for every
  call.** Saves the connection's round trip for every pipe, a stream's
  included, but changes how every pipe is routed and closed, and still costs
  the close's answer and the late `resident_done`.

## Open questions

None.

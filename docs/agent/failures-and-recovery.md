# Failures and recovery

A provider request can fail in many ways: a vendor's rate limit, an overloaded
server, a dropped connection, an expired credential, a history too long for
the model, or an answer Demi cannot read. Demi keeps what the vendor said,
retries what waiting can fix, and gives every unfinished turn one way to go
on.

A provider reports a failure inside its run, as the run's last event, never as
an error outside the stream. Every failure carries a code, which recovery
decisions read, and diagnostics, which the user and support read:

| Code | Meaning | Retried automatically |
| --- | --- | --- |
| `rate_limit` | A quota or throttling failure | Yes |
| `overloaded` | A transient failure: HTTP 5xx, a timeout, a network or socket failure | Yes |
| An authentication code | A missing, invalid or expired credential, or one that could not be refreshed | No |
| `context_length_exceeded` | The request is larger than the model accepts | No |
| A vendor's own code | Any other failure the vendor named | No |
| None | A failure Demi found in the vendor's answer: a frame it cannot decode, or a stream that breaks the vendor's protocol | No |

Invalid requests are terminal as well. The complete set of codes belongs to the
provider contract ([Providers](../providers/providers.md)).

An HTTP failure gets its code from its status: 401 and 403 are
`auth_expired`, 429 is `rate_limit`, 408, 409, 425 and every 5xx are
`overloaded`, and a 400 whose text mentions `context`, `too long` or `token` is
`context_length_exceeded`. A failure the vendor reports inside a response, such
as an error event in a stream, gets its code from the vendor's own code and
message, read as lowercase words with every other character, `_` and `-`
included, separating them. The first row that matches decides, and without a
match the vendor's own code stands:

| Words | Code |
| --- | --- |
| `context`, `too long`, or a word starting with `max` followed later by `token` or `tokens` | `context_length_exceeded` |
| `rate`, a word starting with `ratelimit`, `quota`, `usage`, `billing`, `balance` | `rate_limit` |
| `auth`, `authentication`, `authorization`, or `invalid` or `expired` before or after an API, access or auth key or token | `auth_expired` |
| a word starting with `overload`, `unavailable`, `server error`, `internal error`, `api error`, `timeout`, `timed out`, `fetch failed`, `network`, `socket`, a word starting with `econn` | `overloaded` |

Words count only whole, so `generate` is not `rate` and `limit` alone decides
nothing: an invalid request whose message mentions a limit keeps the vendor's
code and is not retried.

A failure Demi finds itself gets its code by an explicit rule, never by
matching words in its message. A timeout or a network failure is
`overloaded`. A decode or protocol failure has no code, its source is
`stream`, and its record is the text Demi could not read; it is never retried
automatically, and `resume` can continue the turn.

The diagnostics hold the failure's source, the client request id Demi gave the
attempt, the vendor's request and response ids, the vendor's raw code, the
HTTP status when there is one, and the failure record. A provider that does not
say where a failure came from reports the source `unknown`.

## The failure record

What the vendor said is kept exactly as it arrived. A Codex usage limit arrives
as a WebSocket frame with the plan, the moment the limit lifts, and the quota
headers. Keeping only its `message` would throw away the one thing the user
wants to know: when it works again. The diagnostics therefore carry
`upstream`, the vendor's failure as text:

| `source` | Where the failure came from | `upstream` |
| --- | --- | --- |
| `stream` | An error event inside a streamed response, or a frame Demi could not decode | The frame's text as received: the SSE `data` payload, the WebSocket message, or a CLI's output line |
| `http` | A request the vendor answered with a failure status | JSON `{ "status", "headers", "body" }`: the status code; every response header as a `[name, value]` pair, with the name in lowercase, sorted by name, and a repeated header kept as separate pairs in the order they arrived; and the body text as received |
| `transport`, `unknown` | No answer from the vendor | Absent |

Headers are sorted so that two records of the same response are equal, whatever
order the HTTP stack reports them in. Apart from that order and the lowercase
names, nothing in the record is parsed and rewritten, filtered, redacted or
cut. A field the vendor sends in a form Demi does not expect is still there
for a later reader. An HTTP failure arrives as three parts, so the record keeps
them together in one container; that container is the only structure Demi
adds. A body that cannot be read counts as empty, and the status and headers
still speak. The record is saved with the `error` block and travels wherever
the diagnostics do.

The record holds only what the vendor sent back, never the request Demi sent,
so a credential Demi used is never recorded. Error messages are not redacted
either: they are shown as the vendor sent them. Mainstream vendors do not echo
a full key in an error, so a filter against it would aim at nothing. Demi's
own messages never contain a credential.

## Reading a failure

Whatever a reader needs is read out of the record when it is needed, by the
provider that produced it: only that provider knows its vendor's format. A
provider offers one reader, which takes the diagnostics and the moment the
failure was received and returns the facts Demi shows. There is one fact,
`retryAt`: the moment the vendor says the request can succeed again, as an
RFC 3339 time, with a relative wait counted from the moment the failure was
received.

The standard reading, which every provider can use, is the HTTP `Retry-After`
header, read as RFC 9110 defines it:

- A number of seconds, such as `120`, counts from the moment the response was
  received. Surrounding whitespace is allowed, and a decimal fraction counts
  to the millisecond.
- An HTTP-date, such as `Wed, 21 Oct 2015 07:28:00 GMT`, names the moment
  itself.
- Any other value names no time. For example, `1e3`, `0x10`, an ISO 8601 date
  and `-5` name none.

A vendor's own fields are read by that vendor's provider and nowhere else, in
the unit the vendor declares for each field. For example, a Codex usage limit
carries `resets_at`, a Unix time in seconds, and `resets_in_seconds`, a number
of seconds after the failure was received. The Codex reader looks for them in
the failure's `error`, in a WebSocket envelope's `event.error`, in a failed
response's `response.error`, or in an HTTP failure's body; a value that is not
a number reads as absent. Without them, it uses the standard reading.

The same reader serves both uses, so there is one implementation:

- At failure time, the provider reads its own record to set the wait the retry
  policy needs. The wait is never stored. A vendor wait longer than the
  policy's backoff ceiling is not retried: the retry would fail the same way,
  so the failure is terminal at once.
- When blocks are shown, the backend reads each `error` block's record through
  the provider named in the block's model selection, and sends the facts
  beside the transcript ([Failure facts](../backend/backend.md#failure-facts)).
  The browser renders them and never interprets a vendor payload. A record
  whose provider configuration has been removed shows without facts.

A new fact worth showing is a new field of the reader over the same stored
record. Nothing stored changes, and records written earlier gain the new fact
too.

## Retries

The same diagnostics travel through `retry_scheduled`, terminal `error` blocks,
`error` frames and the failed action. Products can therefore explain a retry in
progress and keep the identifiers needed to investigate a terminal failure
without parsing vendor message text.

The agent owns transient retry. A provider makes one attempt per run and
reports a classified failure; it does not retry the request. Credential
resolution is the one exception: it may repeat a request once after an HTTP 401,
with a refreshed credential ([Providers](../providers/providers.md)). Keeping
retries in the agent keeps retry counts, backoff, cancellation, transcript
safety and retry reports the same for HTTP and streaming failures.

The agent retries a failed attempt when all of these hold:

1. The code is `rate_limit` or `overloaded`.
2. The attempt was not the fourth: a request gets four attempts in all.
3. The vendor named no wait longer than 30 seconds, the backoff ceiling.
4. Everything the attempt put in the transcript can be unwound: its resume
   point reaches back past where it started
   ([Recovery is one mechanism](#recovery-is-one-mechanism)).

Reasoning is unwound and the request reissued. Text already streamed out is
not, because a product may have posted it, and a second attempt would post a
second answer beside it. An empty lifecycle event, such as the start of a
reasoning item, is not written to the transcript unless material output
follows it. Completed tool calls from earlier requests of the turn remain and
are not run again. Unwinding also returns command state to the version
recorded where the attempt began ([Command state history](command-state-history.md)).

The wait before a retry is the vendor's wait when it named one. Otherwise it is
random, from zero up to a cap that starts at 1 second and doubles with each
retry, never above 30 seconds: up to 1 second before the second attempt, 2
before the third, 4 before the fourth. Before each wait, the client receives `retry_scheduled` with the
attempt number, the delay, the code and the diagnostics. The turn stays in its
streaming stage while it waits, so steers keep being accepted, and Stop ends
the wait. The product shows the whole wait as one Requesting row
([Recovering an unfinished turn](../product/product.md#recovering-an-unfinished-turn)).

When the agent does not retry, the failure is written as an `error` block and
the action fails with it. The turn is unfinished, and `resume` can continue
it. Compaction's summary requests follow the same policy
([Compaction](compaction.md#one-pass)).

## Recovery is one mechanism

A transient provider failure and a human asking to continue a dead turn ask
the same question, how this turn gets finished, so they run the same code. The
answer depends only on what has already left the process, never on which of
the two asked or on how the turn died.

`resume` is that mechanism. It finds the turn's resume point, unwinds to it,
and infers again from there. Callers do not choose a granularity, and products
must not branch on the failure kind: a terminal error can arrive after ten
minutes of tool calls just as easily as before the first token, so mapping an
error to one action and a stop to another picks the destructive path exactly
when it is most damaging.

The resume point is found by scanning back from the end over the leftovers of
the attempt that did not finish: thinking, redacted thinking, `error` blocks,
and text that is empty or only whitespace. The scan stops at the first block
someone may already have acted on. Transcript blocks stream outward as they are
produced, and products turn them into effects that cannot be recalled:
rendering them, posting them to a chat, running the tool they describe.

- A tool call stops the scan, whatever its status: one still marked executing
  outlived the process running it, and whether its effect landed is unknown.
- A `response` stops it: it records a request that did complete, and its usage
  anchors the context estimate.
- An `abort` block stops it: it is history the user created, not a leftover.
- Steers and agent messages stop it: they are input, not leftovers.

For example, a turn holds a `user` block, some `text` and a completed
`tool_call`, then `thinking` and an `error` from the next request. The scan
drops the error and the thinking and stops at the tool call; `resume` appends a
`resume` block, and the model continues after the tool's result.

If the scan reaches the block that opened the turn without stopping, the whole
turn was discardable, and `resume` reruns it the way `retry` does; a turn is
opened by a `user`, `context` or new-turn `wakeup` block
([Block types](runtime.md#block-types)). A rerun spares the model a
continuation attached to a stub of its own aborted output.

Otherwise, the leftovers are dropped, command state returns to its version at
the cut, the latest stopped marker is marked as resumed, a `resume` block is
appended, and inference continues after the preserved progress. The model
receives the `resume` block as "Continue from where you left off."

The product offers `resume` in one control above the composer, labeled Resume
after an error and Continue after the user's Stop
([Recovering an unfinished turn](../product/product.md#recovering-an-unfinished-turn)).
A turn the backend shut down, or that a crash interrupted, ends with an `error`
block with the code `interrupted`
([Dispose and restore](runtime.md#dispose-and-restore)), and `resume` drops it
like any other leftover.

`retry` is not part of this. It discards the whole turn and runs it again from
its input: it rewinds to the block that opened the turn, keeps the turn's
steers and agent messages, and drops the rest. That is "regenerate", for a
caller who wants a different answer to the same question and knows the turn's
effects can be repeated. It is never the way to recover a failure.

## Acceptance

Tests use scripted providers, recorded vendor frames and responses, and an
injected clock; no test calls a real model.

| Situation | Required observation |
| --- | --- |
| An HTTP failure with repeated and mixed-case headers | The record lists lowercase names sorted by name, repeats as separate pairs in arrival order, and the body verbatim |
| A stream failure | The record is the frame's text as received |
| A frame Demi cannot decode | The failure has no code and source `stream`, its record is the frame's text, and it is not retried |
| `Retry-After` values | `120` reads as 120 seconds after receipt; an HTTP-date as that moment; `1e3`, `0x10`, an ISO 8601 date and `-5` as no time |
| A Codex usage limit | `retryAt` comes from `resets_at` or `resets_in_seconds`, else from `Retry-After` |
| Transcript frames and history with `error` blocks | Each carries the facts of its blocks; nothing of them is stored |
| An overloaded failure before any output | `retry_scheduled`, then a successful attempt, with no trace of the failure in the transcript |
| Thinking streamed before a transient failure | It is unwound and the request is sent again |
| Text streamed before a transient failure | The failure is terminal and becomes an `error` block |
| Four transient failures in a row | Three `retry_scheduled` frames, then a terminal `error` block; each delay is within its cap |
| A vendor wait longer than 30 seconds | The failure is terminal at once |
| A failure after a completed tool call, then `resume` | The tool does not run again; the model continues after its result |
| `resume` after a failure before any output | The turn reruns from its input, with no `resume` block |

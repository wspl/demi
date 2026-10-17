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
| `usage_limit` | The account cannot be used until a reset or a change of its plan: a used-up subscription window, credits or a spend cap, or use its administrator turned off | No; with [account rotation](../providers/providers.md#account-rotation) the agent moves to another account |
| `rate_limit` | Throttling: too many requests or tokens for a short while | Yes |
| `rate_limited` | The backend's own request rate limit refused the attempt before it reached the vendor ([Rate limit](../providers/usage-and-quota.md#rate-limit)) | No |
| `overloaded` | A transient failure: HTTP 5xx, a timeout, a network or socket failure | Yes |
| An authentication code | A missing, invalid or expired credential, or one that could not be refreshed | No |
| `context_length_exceeded` | The request holds more tokens than the model accepts | Once, after compaction ([Retries](#retries)) |
| `request_too_large` | The request's body or its number of images is more than the vendor accepts | No ([Retries](#retries)) |
| A vendor's own code | Any other failure the vendor named | No |
| None | A failure Demi found in the vendor's answer: a frame it cannot decode, or a stream that breaks the vendor's protocol | No |

Invalid requests are terminal as well. The complete set of codes belongs to the
provider contract ([Providers](../providers/providers.md)).

A vendor's own sign of a used-up account comes first, read by that vendor's
provider, as its own client reads it: Codex's 429 whose `error.type` is
`usage_limit_reached`, and any Codex failure, with a status or without one
as in a `response.failed` frame, whose `error.type` or `error.code` is
`insufficient_quota`, `credit_balance_exhausted` or a `…_spend_limit_exceeded`
or `…_usage_limit_exceeded`, over HTTP or in a WebSocket frame; Grok
Build's 402, and its 403 whose text says `run out of credits`; the Claude
Code CLI's `result` line whose `api_error` is `usage_limit_reached`, which
the CLI sets for a 429 whose unified rate-limit headers name a used-up
window or spend cap, or a seat or member whose use is turned off, and never
for a 429 that only throttles, whose text says *not your usage limit* (CLI 2.1.296, against a
scripted vendor, three runs each). Each is `usage_limit`, and no wording
stands in for these signs. Otherwise an HTTP failure gets its code from its status:
401 and 403 are `auth_expired`, 429 is `rate_limit`, 408, 409, 425 and every 5xx are
`overloaded`, and 413 is `request_too_large`, as is a 400 whose text
mentions `too many images`, `many-image`, `request too large` or `payload too
large`: the first two are the Anthropic API's refusals of a request with more
images than it takes, or with more than 20 images when one exceeds its size
for many images. Any other 400 whose text mentions `context`, `too long`,
`token` or `too large` is `context_length_exceeded`. A failure the vendor reports inside a response, such
as an error event in a stream, gets its code from the vendor's own code and
message, read as lowercase words with every other character, `_` and `-`
included, separating them. The first row that matches decides, and without a
match the vendor's own code stands:

| Words | Code |
| --- | --- |
| `too many images`, `many image`, `request too large`, `payload too large` | `request_too_large` |
| `context`, `too long`, `too large`, or a word starting with `max` followed later by `token` or `tokens` | `context_length_exceeded` |
| `billing`, `balance`, `out of credits` | `usage_limit` |
| `quota`, `rate`, a word starting with `ratelimit` | `rate_limit` |
| `auth`, `authentication`, `authorization`, or `invalid` or `expired` before or after an API, access or auth key or token | `auth_expired` |
| a word starting with `overload`, `unavailable`, `server error`, `internal error`, `api error`, `timeout`, `timed out`, `fetch failed`, `network`, `socket`, a word starting with `econn` | `overloaded` |

Words count only whole, so `generate` is not `rate`, and `limit` or `usage`
alone decides nothing: an invalid request whose message mentions a limit, or
an invalid usage of a parameter, keeps the vendor's code and is not retried.

A failure Demi finds itself gets its code by an explicit rule, never by
matching words in its message. A timeout or a network failure is
`overloaded`, and so is a stream that ends before the vendor said the reply
is complete ([Reading vendor input](../providers/providers.md#reading-vendor-input)):
retried while nothing it streamed is final, as when it broke during the
reasoning, and otherwise ending the turn with Resume. A reply the vendor
ended before it was complete, for its output limit, a filter or another
reason it names, is `incomplete`, which is never retried, since sending the
same request again ends it the same way. A decode or protocol failure has no code, its source is
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
| `stream` | An error event inside a streamed response, or a frame Demi could not decode | The frame's text as received: the SSE `data` payload, the WebSocket message, or a CLI's output line; for Claude Code, its failing assistant line and its `result` line as received, joined by a newline, since the reset time is only on the first and `api_error` on both |
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
a number reads as absent. Without them, it uses the standard reading. The
Claude Code CLI's usage limit carries the limited window and its reset in
the failing assistant line's `api_error_params.rate_limit_info`, the
request's own: its `resetsAt`, the window's reset, and its
`overageResetsAt`, the reset of the usage credits that run past the
subscription. Either one lifts the limit, so its reader takes the earlier
of those present as the moment, before the account's latest quota, which
may be older than the failure.

The same reader serves both uses, so there is one implementation:

- At failure time, the provider reads its own record to set the wait the retry
  policy needs. The wait is never stored. A vendor wait longer than the
  policy's backoff ceiling is not retried: the retry would fail the same way,
  so the failure is terminal at once.
- When blocks are shown, the backend reads each `error` block's record through
  the provider named in the block's model selection, and sends the facts
  beside the transcript ([Failure facts](../backend/backend.md#failure-facts)).
  The web app renders them and never interprets a vendor payload. A record
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
are not run again.

The wait before a retry is the vendor's wait when it named one. Otherwise it is
random, from zero up to a cap that starts at 1 second and doubles with each
retry, never above 30 seconds: up to 1 second before the second attempt, 2
before the third, 4 before the fourth. Before each wait, the client receives `retry_scheduled` with the
attempt number, the delay, the code and the diagnostics. The turn stays in its
streaming stage while it waits, so steers keep being accepted, and Stop ends
the wait. The product shows the whole wait as one Requesting row
([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)).

A request refused as too large is never sent again as it was: the same
request would be refused again. The attempt left nothing in the transcript,
since a refusal comes before any output, and the client sees no
`retry_scheduled`.

- Refused for its tokens (`context_length_exceeded`), its turn runs one
  compaction pass and, when the pass compacted, sends the request again, built
  from the compacted history
  ([When compaction runs](compaction.md#when-compaction-runs)); the client sees
  the phase `compacting`. A pass that compacts nothing, or a second refusal of
  the turn's request, is terminal.
- Refused for its size (`request_too_large`), the failure is terminal: nothing
  is summarized and no medium is dropped, since either would change what
  earlier requests carried and lose the vendor's cache. The turn is
  unfinished: the user's Compact, which this failure allows below half the
  window ([When compaction runs](compaction.md#when-compaction-runs)),
  summarizes the media away, and `resume` then continues it. For example, an
  Anthropic-compatible endpoint that takes less than the Anthropic API refuses
  a request of 20 MB with HTTP 413, and the turn fails with
  `request_too_large`.

When the agent does not retry, the failure is written as an `error` block and
the action fails with it. The turn is unfinished, and `resume` can continue
it, unless the action had begun no turn
([The unfinished turn](#the-unfinished-turn)). Compaction's summary requests
follow the same policy, except that one refused for its tokens
(`context_length_exceeded`) is asked again for half its window; one refused for
its size (`request_too_large`) fails like any other request
([One pass](compaction.md#one-pass)): a summary request that fails for good
is written as the session's `error` block, whether the user's `compact` or a
turn ran the pass, and a turn it ended is unfinished like any other.

## The unfinished turn

The last turn is unfinished when the record that ended it is an `error`
block (something broke) or an `abort` block (the user stopped it); any other
end means it finished. The product offers recovery from that record
([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)).
Only an action that began a turn, that is, wrote into the transcript, can
leave such a record, and a compaction leaves none:

- **A failure of an action that wrote nothing** ended no turn. Its `error`
  block stays in the transcript, and says so with `outsideTurn`. For
  example, the user presses Compact after a finished answer and the summary
  request fails: no turn is unfinished, and `resume` would make the model
  continue an answer it had finished.
- **A Stop of an action that wrote nothing** records no `abort` block. For
  example, the user presses Compact after a finished answer and stops it
  while the summary is written: the transcript is as it was, and nothing is
  left to continue. Input the stop itself writes counts: a pending steer or
  agent message is written, and the `abort` block after it lets Continue
  answer it. Nothing waits for the user afterwards
  ([Stop](runtime.md#stop)).
- **A Host that was away** leaves the root's turn unfinished: when the
  turn ends normally, but a call of it failed because its Host was offline
  and no later call of the turn ran on that Host, the session appends an
  `error` block of Demi's own, cause `host_offline` with the device, after
  the agent's last words, so the turn offers Resume
  ([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)).
  Resume continues it with a `resume` block whose reason says where the
  turn now runs: "MacBook Pro is back online. Continue from where you left
  off.", or, after the user moved the conversation meanwhile, "This
  conversation now runs on Cloud. Continue from where you left off." The
  backend composes the reason as it passes the user's Resume to the
  session, since it alone knows the conversation's primary Host. A
  child's turn gets none: its result, which says what it could not do,
  reaches its parent, whose own turn decides.
- **A compaction** is not a turn's end. Its boundary and marker are kept
  history, but they say nothing about the turn before them. For example, a
  turn fails, and the user then compacts: the divider is the last block,
  and the failed turn still offers Resume, which continues it after the
  summary ([Recovery is one mechanism](#recovery-is-one-mechanism)).

Behind a record that ended no turn, or behind a compaction, the turn before
is as it was: when that turn had failed or been stopped, it still offers its
Resume or Continue.

A root whose last turn ended with an `error` block holds its waiting input,
agent messages and command reports: they are written into the failed turn as
they arrive, where a steer would be, and shown there, but none starts a turn
until the user acts, with Resume or a message, which then answers them. The
record that ended the turn is still the `error` block, read past the input
written behind it, so the dock keeps offering Resume. A Stop holds nothing
([Stop](runtime.md#stop)): the user stopped the work, not the agent's
ability to answer. Otherwise each wakeup would
start a turn that fails the same way: a Claude subscription at its usage
limit failed the main agent once for each of its children that reported,
four red rows in a row, each one a whole history sent for nothing. A child's
turn is not held, since its parent decides for it.

A turn that a graceful shutdown interrupted, for a deployment, an upgrade
or a restart, goes on by itself once the backend is back, unless the user
turned that off (`resumeAfterRestart`, on unless turned off,
[User preferences](../product/web-api-settings.md#user-preferences)). The restart
was Demi's doing, not the turn's, so nothing about the turn suggests it
would fail again. As it starts, the backend opens each conversation whose
root's last turn ended with the `interrupted` record its own shutdown
wrote and resumes it, as the user's Resume would, with the reason "Demi
resumed this turn after the backend restarted."; the subagents that turn
was waiting on are resumed with their tree, as a restored tree resumes
them. A turn is resumed so once for each shutdown that interrupted it: if
it fails again, that failure is held as any other. A crash's interruption,
whose record the conversation writes only when next opened, never resumes
by itself, so a turn that brought the backend down cannot bring it down
again and again; it waits for the user's Resume. With the preference off,
every interrupted root turn waits for the user's Resume; its subagents
resume when their tree is next restored, such as when the user opens the
conversation, as they always do.

The shutdown tells its interruptions from a crash's by a mark in the control
records, one per root, written before it aborts the root's running turn, so
a shutdown cut short, such as by a deployment that stops waiting, still
leaves a mark for each root it began to interrupt. At start a marked root
resumes when its last turn ended interrupted, whichever record says so: the
one the shutdown wrote, or the one written at open when the shutdown was cut
short before saving it
([Startup and shutdown](../backend/backend.md#startup-and-shutdown)). At
start, after the shards are up, the backend resumes the marked roots in the
background, removing each mark before it acts on it; the start waits for
none of them.

A turn its offline Host left (`host_offline`) goes on by itself when that
Host comes back, unless the user turned that off (`resumeWhenHostBack`, on
unless turned off, [User preferences](../product/web-api-settings.md#user-preferences)):
the backend resumes it, as the user's Resume would, with the reason "MacBook
Pro is back online. Continue from where you left off.", when the runner of
the device the `error` block names connects again and the conversation still
runs there. A device that came back before the turn ended, while the turn still ran,
counts as back: the turn resumes as the record is written. It does so once
for each `host_offline` block: a turn that the
same device leaves again waits for the user's Resume, so a device that keeps
dropping costs one resumed turn, not one per reconnection. A conversation
moved elsewhere meanwhile is not resumed; the user's Resume says where it now
runs. With the preference off, the turn waits for the user's Resume, as
before.

Continuing by itself is the user's choice, off unless they turn it on
(`resumeAtRetry`, [User preferences](../product/web-api-settings.md#user-preferences)):
with it on, a root's turn whose failure names its `retryAt` is resumed at that
moment, as the user's Resume would, so a usage limit that resets at 09:00
continues then; with it off, the hold lasts until the user acts. The
preference is read when the moment comes, so turning it off before then
keeps the turn waiting. Only a moment still to come is waited for: a
conversation opened after its failure's moment has passed never resumes by
itself. The backend keeps the wait while it runs, whether the
conversation is open or was put away to save memory, and opens it when the
moment comes; after a backend restart it starts again when the conversation
is next opened. A
usage limit's moment is the one the failure names, such as Codex's
`resets_at`, else the latest reset among the windows that the quota of
the account the failing request used shows used up, which the `error` block
names under [Account rotation](../providers/providers.md#account-rotation); a limit with neither, such as a spend cap, names no
moment and waits for the user.

A message is never left outside a turn. A send writes its `user` block as
soon as its action starts, before its model switch lands
([Model switch](runtime.md#model-switch)), so the message's turn has begun
before anything of it can fail or be stopped. For example, the user picks a
model with a smaller window and sends a message, and the compaction the
switch needs fails: the message stays in the transcript, the turn ends with
the `error` block, and Resume runs the switch and the turn again. A
continuation writes its wakeup or agent messages first in the same way.

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

A compaction's `compaction_boundary` and `compaction_marker` neither stop the
scan nor go: the scan passes over them, and the unwind keeps them where they
are. For example, a turn's request fails before any output, and the user
then compacts: the transcript ends with the turn's `user` block, its `error`
block and the marker. `resume` drops the `error` block, keeps the marker,
and reruns the turn from its `user` block, after the summary.

For example, a turn holds a `user` block, some `text` and a completed
`tool_call`, then `thinking` and an `error` from the next request. The scan
drops the error and the thinking and stops at the tool call; `resume` appends a
`resume` block, and the model continues after the tool's result.

If the scan reaches the block that opened the turn without stopping, the whole
turn was discardable, and `resume` reruns it from that block the way `retry`
does; a turn is opened by a `user`, `context` or new-turn `wakeup` block
([Block types](transcript.md#block-types)). A rerun spares the model a
continuation attached to a stub of its own aborted output.

Otherwise, the leftovers are dropped, the latest stopped marker is marked as resumed, a `resume` block is
appended, and inference continues after the preserved progress. The model
receives the `resume` block as "Continue from where you left off.", after
the sentence its reason holds when it has one: "Demi resumed this turn
after the backend restarted." when the system resumed a subagent's turn
after a restart, or "MacBook Pro is back online." or "This conversation
now runs on Cloud." when the user resumed a turn its offline Host left
unfinished.

The product offers `resume` in one control above the composer, labeled Resume
after an error and Continue after the user's Stop
([Recovering an unfinished turn](../product/turns.md#recovering-an-unfinished-turn)).
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
| A Claude Code usage limit | `retryAt` is the reset its `rate_limit_info` names; the failure is terminal on its first attempt |
| A root's turn fails, then two children report and a command ends | Two receipts and a notice are written into the failed turn; no turn starts, Resume stays offered and answers them |
| The same, with a `retryAt` | With `resumeAtRetry` off, nothing starts at that moment; with it on, the turn resumes then, once |
| A graceful shutdown interrupts a running root turn and its running child; the backend starts again | With `resumeAfterRestart` on or absent, the root resumes without a page open, its child with it, each once, the model told the backend restarted; with it off, the root waits for Resume and the child resumes when the conversation is opened |
| A crash interrupts a running turn; the backend starts again | Nothing resumes by itself; opening the conversation shows the interrupted record and Resume |
| A failure after a completed tool call, then `resume` | The tool does not run again; the model continues after its result |
| `resume` after a failure before any output | The turn reruns from its input, with no `resume` block |
| The user's Compact after a finished answer, stopped during its summary | The transcript is as it was, with no `abort` block; the dock offers no Continue |
| A send whose model switch fails to compact, or is stopped while it compacts | The message's `user` block is in the transcript, followed by the `error` or `abort` block; the dock offers Resume or Continue |
| A failed turn, then the user's Compact, then `resume` | The dock still offers Resume behind the divider; `resume` drops the `error` block, keeps the boundary and marker, and the turn goes on after the summary |
| An HTTP 413, or a 400 that says a request has too many images or too large ones | The code is `request_too_large`; the failure is terminal, with no compaction |
| A 400 that says the prompt is too long | The code is `context_length_exceeded`; the turn compacts once and sends the request again, and a second refusal is terminal |

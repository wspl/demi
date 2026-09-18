# Provider Errors and Retries

Provider runtimes classify vendor and transport failures into stable error codes. `rate_limit` represents quota or throttling failures, while `overloaded` represents transient service, HTTP 5xx, timeout, network, and socket failures. Authentication, invalid requests, and context-length failures remain terminal categories.

Error events retain bounded diagnostics separately from the normalized policy code: failure source, client request id, provider request/response ids, raw provider code, and HTTP status when available. Products use the normalized code for recovery decisions and the diagnostics for logging, inspection, and support escalation.

What the vendor said is kept, not reduced to a sentence. A Codex usage limit arrives as an error object with the plan, the time the limit lifts and the quota headers; keeping only its `message` throws away the one thing the user wants to know, when it works again. The diagnostics therefore carry:

`upstream`: the vendor's failure as it arrived, as JSON text. For a stream it is the error event; for an HTTP failure it is `{ status, headers, body }`, with the headers that say when to retry, what quota is left and which request it was, and the body parsed when it is JSON. It is redacted of the request's secret and cut at 16 KB. It is persisted with the error block and travels wherever the diagnostics do.

The record stores what the vendor sent and nothing worked out from it. Whatever a reader needs is read out of `upstream` when it is needed, by the layer that knows vendor formats: `@demicodes/provider` owns `retryAtFromUpstream(upstream, receivedAt)`, which finds the moment the vendor says the request can succeed again (`resets_at`, `resets_in_seconds`, a `Retry-After` header), counting a relative wait from the time the failure was recorded. The browser reaches it through the agent's client entry and shows it on the error record. A new fact worth showing is a new reader over the same stored payload, with no change to what is stored and no old records left behind.

The one thing taken from the payload at failure time is the wait the retry policy needs, `retryAfterMs` on the error event, which is never stored. A vendor wait longer than the policy's backoff ceiling is not retried: the retry would fail the same way, so the failure is terminal at once.

The same diagnostics travel through `retry_scheduled`, terminal transcript error blocks, server frames, and `ProviderStreamError`. Products can therefore explain an in-progress retry and retain the identifiers needed to investigate a terminal failure without parsing vendor message text.

`@demicodes/agent` owns transient inference retry. Providers perform one inference attempt and emit a classified error event; authentication refresh is part of credential resolution and may repeat a request once after an HTTP 401. This keeps retry counts, backoff, cancellation, transcript safety, and retry telemetry consistent across HTTP and streaming failures.

The agent retries a failed attempt when everything that attempt put in the transcript can be unwound — that is, when its resume point reaches back past where it started. Reasoning is unwound and the request reissued; text already streamed out is not, because a product may have posted it and a second attempt would post a replacement beside it. Empty lifecycle events such as a reasoning-item start are not committed unless material output follows. Completed tool calls from earlier provider requests remain in the transcript and are not executed again. The default policy makes four total attempts with capped full-jitter exponential backoff for `rate_limit` and `overloaded`.

## Recovery is one mechanism

A transient provider failure and a human asking to continue a dead round ask the same question — how does this turn get finished — so they run the same code. The answer depends only on what has already left the process, never on which of the two asked or on how the turn died.

`resume` is that mechanism. It finds the turn's resume point, unwinds to it, and re-infers from there. Callers do not choose a granularity, and products must not branch on the failure kind: a terminal error can arrive after ten minutes of tool calls just as easily as before the first token, so mapping error to one action and abort to another picks the destructive path exactly when it is most damaging.

`findResumePoint` scans back from the end over the leftovers of the attempt that did not finish — thinking, redacted thinking, the error marker, empty text — and stops at the first block that someone may already have acted on. Transcript blocks stream outward as they are produced and products turn them into effects that cannot be recalled: rendering them, posting them to a chat, executing the tool they describe. A tool call stops the scan whatever its status, because one still marked executing outlived the process running it and whether its effect landed is unknown. A `response` stops it too — it records a request that did complete, and its usage anchors the context estimate. An abort block stops it, being history the user created rather than a leftover.

When the scan reaches the user turn without stopping, the whole turn was discardable and `resume` simply reruns it, sparing the model a continuation boundary attached to a stub of its own aborted output. Otherwise the leftovers are dropped, a continuation boundary is appended, and inference continues after the preserved progress.

`retry` is not part of this. It discards the whole turn and runs it again from the user's input — "regenerate", for a caller who wants a different answer to the same question and knows the turn's effects can be repeated. It is never the way to recover a failure.

Coverage lives in:

- `packages/provider/src/__tests__/http.test.ts` for common HTTP and vendor-code classification.
- `packages/provider-codex/src/__tests__/responses.test.ts` for Codex streaming error mapping.
- `packages/provider-codex/src/__tests__/provider.test.ts` for Codex-to-Agent transient retry integration.
- `packages/agent/src/__tests__/turn-retry.test.ts` for retry safety, exhaustion, tool continuation, and progress-preserving resume.
- `packages/agent/src/__tests__/recovery.test.ts` for resume-point derivation and the unwind `resume` performs.

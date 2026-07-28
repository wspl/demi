# Provider Errors and Retries

Provider runtimes classify vendor and transport failures into stable error codes. `rate_limit` represents quota or throttling failures, while `overloaded` represents transient service, HTTP 5xx, timeout, network, and socket failures. Authentication, invalid requests, and context-length failures remain terminal categories.

Error events retain bounded diagnostics separately from the normalized policy code: failure source, client request id, provider request/response ids, raw provider code, and HTTP status when available. Products use the normalized code for recovery decisions and the diagnostics for logging, inspection, and support escalation. Arbitrary raw response bodies are not persisted.

The same diagnostics travel through `retry_scheduled`, terminal transcript error blocks, server frames, and `ProviderStreamError`. Products can therefore explain an in-progress retry and retain the identifiers needed to investigate a terminal failure without parsing vendor message text.

`@demicodes/agent` owns transient inference retry. Providers perform one inference attempt and emit a classified error event; authentication refresh is part of credential resolution and may repeat a request once after an HTTP 401. This keeps retry counts, backoff, cancellation, transcript safety, and retry telemetry consistent across HTTP and streaming failures.

The agent retries only when the failed provider attempt has emitted no transcript content. Empty lifecycle events such as a reasoning-item start do not count as content and are not committed unless material output follows. Completed tool calls from earlier provider requests remain in the transcript and are not executed again. The default policy makes four total attempts with capped full-jitter exponential backoff for `rate_limit` and `overloaded`.

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

# Provider Errors and Retries

Provider runtimes classify vendor and transport failures into stable error codes. `rate_limit` represents quota or throttling failures, while `overloaded` represents transient service, HTTP 5xx, timeout, network, and socket failures. Authentication, invalid requests, and context-length failures remain terminal categories.

Error events retain bounded diagnostics separately from the normalized policy code: failure source, client request id, provider request/response ids, raw provider code, and HTTP status when available. Products use the normalized code for recovery decisions and the diagnostics for logging, inspection, and support escalation. Arbitrary raw response bodies are not persisted.

The same diagnostics travel through `retry_scheduled`, terminal transcript error blocks, server frames, and `ProviderStreamError`. Products can therefore explain an in-progress retry and retain the identifiers needed to investigate a terminal failure without parsing vendor message text.

`@demicodes/agent` owns transient inference retry. Providers perform one inference attempt and emit a classified error event; authentication refresh is part of credential resolution and may repeat a request once after an HTTP 401. This keeps retry counts, backoff, cancellation, transcript safety, and retry telemetry consistent across HTTP and streaming failures.

The agent retries only when the failed provider attempt has emitted no transcript content. Completed tool calls from earlier provider requests remain in the transcript and are not executed again. The default policy makes four total attempts with capped full-jitter exponential backoff for `rate_limit` and `overloaded`.

`retry` and `resume` have distinct recovery semantics:

- `retry` rewinds the transcript to the latest user turn and reruns that turn.
- `resume` preserves completed model and tool progress, appends a continuation boundary, and continues after an abort or terminal provider error.

Coverage lives in:

- `packages/provider/src/__tests__/http.test.ts` for common HTTP and vendor-code classification.
- `packages/provider-codex/src/__tests__/responses.test.ts` for Codex streaming error mapping.
- `packages/provider-codex/src/__tests__/provider.test.ts` for Codex-to-Agent transient retry integration.
- `packages/agent/src/__tests__/turn-retry.test.ts` for retry safety, exhaustion, tool continuation, and progress-preserving resume.

---
'@demicodes/provider-codex': patch
'@demicodes/provider': patch
---

A Codex response stream that ends before `response.completed`, `response.failed`, `response.incomplete` or `error` — a WebSocket closed or an SSE body cut off mid-response — is now a transport error instead of a silent end of the turn, so the turn retry policy retries it. TLS and certificate verification failures now classify as transient `overloaded` errors too.

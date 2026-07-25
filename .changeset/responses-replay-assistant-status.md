---
'@demicodes/provider-openai-api': patch
---

Add `request.replayAssistantStatus` so the Responses transport can emit `status: 'completed'` on replayed assistant messages. Gateways that validate input against the full Responses item schema reject the message without it — Volcengine Ark fails the request with `missing input.status` — while relay bridges reject the field as an unknown parameter, so it stays off by default.

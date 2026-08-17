---
'@demicodes/provider-grok-build': patch
---

Treat every Grok Build catalog model as image-capable. The cli-chat-proxy `/v1/models` payload has no modality field, and the official Grok Build client keeps native images on the stock harness instead of maintaining a per-id allowlist.

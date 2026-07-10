---
"@demicodes/provider-grok-build": patch
---

Harden Grok Build against concurrency and wire edge cases: auth-lock
contenders now wait out a slow refresh (30s) and adopt a token another
process refreshed instead of refreshing again; video content blocks degrade
to text placeholders instead of shipping as `image_url`; the 401 retry path
cancels the stale response body; and multi-line SSE `data:` fields are
joined per spec before JSON parsing.

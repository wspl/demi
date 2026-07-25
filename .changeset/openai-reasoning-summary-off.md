---
'@demicodes/provider-openai-api': patch
---

Honor `thinking.summary: 'off'` on the Responses API by omitting `reasoning.summary` instead of downgrading it to `'auto'`. Strict OpenAI-compatible gateways such as Volcengine Ark reject `reasoning.summary` as an unknown field for any value, null included, so callers that opt out of summaries had no way to reach those endpoints.

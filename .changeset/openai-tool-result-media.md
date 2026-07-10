---
"@demicodes/provider-openai-api": patch
---

Tool results carrying images on the Responses wire now reach the model:
`function_call_output` stays text (gateways drop or reject media inside it),
and the images ride a follow-up user message labeled with the call id. The
agent layer only attaches media the model's catalog accepts, so text-only
models see no behavior change. Verified end to end against a live
vision-capable Responses endpoint.

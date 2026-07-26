---
"@demicodes/provider-google": patch
---

Reduce tool schemas to the keywords Gemini's function declarations accept.

`functionDeclarations.parameters` is an OpenAPI 3.0 subset rather than JSON Schema, and it rejects the entire request on the first keyword it does not recognise instead of ignoring it:

```
Invalid JSON payload received. Unknown name "additionalProperties"
  at 'tools[0].function_declarations[3].parameters': Cannot find field.
```

`additionalProperties: false` is exactly what a careful tool author writes — demi's own standard shell tools all do — so passing `inputSchema` through verbatim broke every agent that used them. Unsupported keywords are now dropped, recursing through `properties`, `items` and `anyOf`. Dropping is the right failure mode: those keywords only constrain what the model may send, and the command parses its own input anyway, so a slightly looser wire schema costs nothing while an error would break callers over a constraint the transport merely cannot express.

Also stop replaying thought signatures that this provider did not issue. A transcript outlives a provider choice — a conversation can start on one and continue on another — but a signature's format is private to the provider that made it, and Gemini rejects both a foreign signature (`Base64 decoding failed`) and a `functionCall` part without one. Signatures are now tagged on the way out and only accepted back if they carry that tag; a call whose signature is not ours degrades, along with its result, to plain text, so the model still sees what ran instead of the request failing outright.

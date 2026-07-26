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

# @demicodes/provider-google

## 0.17.3

### Patch Changes

- @demicodes/core@0.17.3
- @demicodes/provider@0.17.3
- @demicodes/utils@0.17.3

## 0.17.2

### Patch Changes

- @demicodes/core@0.17.2
- @demicodes/provider@0.17.2
- @demicodes/utils@0.17.2

## 0.17.1

### Patch Changes

- @demicodes/core@0.17.1
- @demicodes/provider@0.17.1
- @demicodes/utils@0.17.1

## 0.17.0

### Patch Changes

- @demicodes/core@0.17.0
- @demicodes/provider@0.17.0
- @demicodes/utils@0.17.0

## 0.16.0

### Patch Changes

- @demicodes/core@0.16.0
- @demicodes/provider@0.16.0
- @demicodes/utils@0.16.0

## 0.15.0

### Patch Changes

- @demicodes/core@0.15.0
- @demicodes/provider@0.15.0
- @demicodes/utils@0.15.0

## 0.14.2

### Patch Changes

- @demicodes/core@0.14.2
- @demicodes/provider@0.14.2
- @demicodes/utils@0.14.2

## 0.14.1

### Patch Changes

- Updated dependencies [2da4bf6]
  - @demicodes/utils@0.14.1
  - @demicodes/core@0.14.1
  - @demicodes/provider@0.14.1

## 0.14.0

### Patch Changes

- @demicodes/core@0.14.0
- @demicodes/provider@0.14.0
- @demicodes/utils@0.14.0

## 0.13.0

### Patch Changes

- @demicodes/core@0.13.0
- @demicodes/provider@0.13.0
- @demicodes/utils@0.13.0

## 0.12.0

### Patch Changes

- @demicodes/core@0.12.0
- @demicodes/provider@0.12.0
- @demicodes/utils@0.12.0

## 0.11.0

### Major Changes

- 5843565: Require `AgentProvider.clone()` and add `AgentSession.clone()` for isolated session forks.

  Every provider runtime must return an independently disposable clone with the same configuration but without shared live-process / continuation state. Sessions expose `.clone()` for point-in-time copies (optional provider/runtime/state/transcript overrides); parent persistence is never inherited. See `docs/provider-session-clone.md`.

### Patch Changes

- Updated dependencies [5843565]
  - @demicodes/provider@0.11.0
  - @demicodes/core@0.11.0
  - @demicodes/utils@0.11.0

## 0.10.2

### Patch Changes

- @demicodes/core@0.10.2
- @demicodes/provider@0.10.2
- @demicodes/utils@0.10.2

## 0.10.1

### Patch Changes

- @demicodes/core@0.10.1
- @demicodes/provider@0.10.1
- @demicodes/utils@0.10.1

## 0.10.0

### Patch Changes

- @demicodes/core@0.10.0
- @demicodes/provider@0.10.0
- @demicodes/utils@0.10.0

## 0.9.1

### Patch Changes

- fd4b9ce: Reduce tool schemas to the keywords Gemini's function declarations accept.

  `functionDeclarations.parameters` is an OpenAPI 3.0 subset rather than JSON Schema, and it rejects the entire request on the first keyword it does not recognise instead of ignoring it:

  ```
  Invalid JSON payload received. Unknown name "additionalProperties"
    at 'tools[0].function_declarations[3].parameters': Cannot find field.
  ```

  `additionalProperties: false` is exactly what a careful tool author writes — demi's own standard shell tools all do — so passing `inputSchema` through verbatim broke every agent that used them. Unsupported keywords are now dropped, recursing through `properties`, `items` and `anyOf`. Dropping is the right failure mode: those keywords only constrain what the model may send, and the command parses its own input anyway, so a slightly looser wire schema costs nothing while an error would break callers over a constraint the transport merely cannot express.

  Also stop replaying thought signatures that this provider did not issue. A transcript outlives a provider choice — a conversation can start on one and continue on another — but a signature's format is private to the provider that made it, and Gemini rejects both a foreign signature (`Base64 decoding failed`) and a `functionCall` part without one. Signatures are now tagged on the way out and only accepted back if they carry that tag; a call whose signature is not ours degrades, along with its result, to plain text, so the model still sees what ran instead of the request failing outright.

  - @demicodes/core@0.9.1
  - @demicodes/provider@0.9.1
  - @demicodes/utils@0.9.1

## 0.9.0

### Patch Changes

- @demicodes/core@0.9.0
- @demicodes/provider@0.9.0
- @demicodes/utils@0.9.0

## 0.8.0

### Minor Changes

- df261c2: Add a native Google Gemini provider, and stop dropping tool-returned media on the Chat Completions wire.

  `@demicodes/provider-google` talks the Gemini `generateContent` API directly instead of routing through an OpenAI-compatible shim. That matters for three things the shim cannot express:

  - **Video is a real inline part**, so the model reads the frames _and_ the audio track. Adapters without a video block have to degrade it to a placeholder.
  - **Thought summaries, thought signatures and thinking token counts arrive as first-class fields** rather than `<thought>` tags glued into the answer text and thinking tokens missing from the usage report.
  - **Thought signatures survive replay.** Gemini rejects any request whose replayed `functionCall` lost its `thoughtSignature`, so an agent loop cannot reach its second turn without carrying it. The provider parks each signature on the thinking item immediately in front of the call, which keeps it in the persisted transcript — a runtime-local map would evaporate on session resume and 400 from then on.

  Separately, `@demicodes/provider-openai-api` now forwards tool-returned media on the Chat Completions wire the way it already did on the Responses wire. A `tool` message is text-only, so images and video a command produced were silently dropped there: commands whose whole purpose is to _show_ the model something did nothing. Both wires now also pass video through, not just images.

### Patch Changes

- @demicodes/core@0.8.0
- @demicodes/provider@0.8.0
- @demicodes/utils@0.8.0

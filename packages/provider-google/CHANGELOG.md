# @demicodes/provider-google

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

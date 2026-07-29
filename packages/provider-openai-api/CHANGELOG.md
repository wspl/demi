# @demicodes/provider-openai-api

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

## 0.7.2

### Patch Changes

- f5bf224: Add `request.replayAssistantStatus` so the Responses transport can emit `status: 'completed'` on replayed assistant messages. Gateways that validate input against the full Responses item schema reject the message without it — Volcengine Ark fails the request with `missing input.status` — while relay bridges reject the field as an unknown parameter, so it stays off by default.
  - @demicodes/core@0.7.2
  - @demicodes/provider@0.7.2
  - @demicodes/utils@0.7.2

## 0.7.1

### Patch Changes

- acb56b0: Honor `thinking.summary: 'off'` on the Responses API by omitting `reasoning.summary` instead of downgrading it to `'auto'`. Strict OpenAI-compatible gateways such as Volcengine Ark reject `reasoning.summary` as an unknown field for any value, null included, so callers that opt out of summaries had no way to reach those endpoints.
  - @demicodes/core@0.7.1
  - @demicodes/provider@0.7.1
  - @demicodes/utils@0.7.1

## 0.7.0

### Patch Changes

- Updated dependencies [2bb314c]
  - @demicodes/core@0.7.0
  - @demicodes/provider@0.7.0
  - @demicodes/utils@0.7.0

## 0.6.1

### Patch Changes

- @demicodes/core@0.6.1
- @demicodes/provider@0.6.1
- @demicodes/utils@0.6.1

## 0.6.0

### Patch Changes

- @demicodes/core@0.6.0
- @demicodes/provider@0.6.0
- @demicodes/utils@0.6.0

## 0.5.0

### Minor Changes

- ca617e1: Align all first-party packages on one shared version so consumers and repository workspaces use one coherent Demi release without reconciling independent package versions.

### Patch Changes

- Updated dependencies [ca617e1]
  - @demicodes/core@0.5.0
  - @demicodes/provider@0.5.0
  - @demicodes/utils@0.5.0

## 0.3.4

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/utils@0.4.0
  - @demicodes/provider@0.4.3

## 0.3.3

### Patch Changes

- Updated dependencies [0a3936f]
  - @demicodes/provider@0.4.0

## 0.3.2

### Patch Changes

- ca71716: Publish tarballs without the `development` export condition. The condition
  resolves to ./src for in-repo workspace resolution, but dist-only tarballs do
  not ship src — and dev-mode bundlers (Vite) enable the development condition
  by default, so consumers resolved exports to files that do not exist. The
  release pipeline now strips the condition at pack time and validates that
  every packed export target actually exists in the tarball.
- Updated dependencies [ca71716]
  - @demicodes/core@0.3.2
  - @demicodes/provider@0.3.2
  - @demicodes/utils@0.3.2

## 0.3.1

### Patch Changes

- Republish with resolved internal dependency ranges. The 0.3.0 tarballs shipped
  literal `workspace:^` ranges because the release went through `changeset
publish` (npm does not rewrite the workspace protocol); 0.3.0 is deprecated.
  The release pipeline now packs and publishes with bun and validates every
  tarball's manifest before anything is pushed to the registry.
- Updated dependencies
  - @demicodes/core@0.3.1
  - @demicodes/utils@0.3.1
  - @demicodes/provider@0.3.1

## 0.3.0

### Minor Changes

- Align all public packages on 0.3.0. Highlights of this release: correct
  Claude Code context-usage reporting (no more spurious compaction on long
  tool-heavy sessions) and session storage phase 1 — role-based
  Status/View/Checkpoint/Artifact naming and bounded tool views that shrink
  session checkpoints from tens of MB to content-proportional size.

### Patch Changes

- Updated dependencies [dd69eb0]
- Updated dependencies
- Updated dependencies [c352335]
  - @demicodes/provider@0.3.0
  - @demicodes/core@0.3.0
  - @demicodes/utils@0.3.0

## 0.2.1

### Patch Changes

- Republish with a runnable dependency closure: `@demicodes/just-bash`
  3.0.1-demi.5 ships the full dist its deep-path exports point at (0.2.0
  installed but could not run), and intra-workspace dependencies publish as
  caret ranges instead of exact pins so future patch releases do not require
  republishing every dependent.
- Updated dependencies
  - @demicodes/utils@0.2.1
  - @demicodes/core@0.2.1
  - @demicodes/provider@0.2.1

## 0.2.0

### Minor Changes

- Align the workspace on 0.2.0: byte-clean binary pipelines with a model-media
  boundary, the --help flag replacing the prompt pseudo-subcommand, hardened
  command bridge execution (ephemeral shells, byte-identical stdin), unified
  provider quota surfaces, the multi-credential pool with a global active
  switch, and tool-result media delivery for OpenAI-compatible and Claude Code
  transports.

### Patch Changes

- 0bcb313: Merge helper duplicates into their owning packages: `errorCode` (errno-style
  code guard) joins `@demicodes/utils`; `numberHeader`, `redactCredentialText`,
  and `toolResultContentToText` join the `@demicodes/provider` kit and replace
  the per-provider copies in codex, claude-code, grok-build, and openai-api.
  The shared tool-result flattener renders `[<type>:<mediaType>]`, so the
  openai-api provider now labels video blocks `[video:…]` instead of `[image:…]`.
- 09bcc0d: Tool results carrying images on the Responses wire now reach the model:
  `function_call_output` stays text (gateways drop or reject media inside it),
  and the images ride a follow-up user message labeled with the call id. The
  agent layer only attaches media the model's catalog accepts, so text-only
  models see no behavior change. Verified end to end against a live
  vision-capable Responses endpoint.
- Updated dependencies [8b7b981]
- Updated dependencies [966c530]
- Updated dependencies [0bcb313]
- Updated dependencies [10dbc6b]
- Updated dependencies [18a72d1]
- Updated dependencies
- Updated dependencies [2af7114]
  - @demicodes/utils@0.2.0
  - @demicodes/core@0.2.0
  - @demicodes/provider@0.2.0

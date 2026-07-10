# @demicodes/provider-claude-code

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

- 32f0e41: Forward images returned by tools to the Claude Code CLI instead of dropping them.

  A `tool_result` carrying an `image` block was being flattened to a `[image:…]`
  text placeholder on both paths: the live SDK-MCP tool-call response
  double-encoded the base64 `data`, and the replayed-history serialization
  replaced the image with placeholder text. The Claude Code CLI does accept images
  inside `tool_result` content, so both now pass the image through unchanged — the
  model can actually see images a tool returns.

- 966c530: Consolidate the credential pool: the three byte-identical per-provider
  `credentials-pool.ts` copies merge into one implementation behind the
  node-only `@demicodes/provider/credentials-pool` subpath (the provider main
  entry stays platform-neutral), which also becomes the canonical home of
  `resolveDemiHome` (host-local re-exports it). Pool mutations are now
  serialized by a create-exclusive lock with unique temp names, closing the
  torn-write race between concurrent imports, and grok's `importDefault`
  activates the vendor-preferred entry deterministically by identity key
  instead of label/detail guessing.
- 0bcb313: Merge helper duplicates into their owning packages: `errorCode` (errno-style
  code guard) joins `@demicodes/utils`; `numberHeader`, `redactCredentialText`,
  and `toolResultContentToText` join the `@demicodes/provider` kit and replace
  the per-provider copies in codex, claude-code, grok-build, and openai-api.
  The shared tool-result flattener renders `[<type>:<mediaType>]`, so the
  openai-api provider now labels video blocks `[video:…]` instead of `[image:…]`.
- 10dbc6b: Native video input support (no frame extraction) plus a per-model modality marker.

  - `core` gains `video` content blocks (`UserContentBlock` / `ToolResultContentBlock`,
    with `VideoSource` / `Base64VideoSource`), video file extensions on
    `FileExtension`, and the shared video capability helpers.
  - `provider` gains `ProviderModel.supportsVideo` — the marker for whether a model
    accepts native video. A model's `acceptedExtensions` now includes the shared
    core video extensions only when it marks video support.
  - `shell` `CommandAsset` and `agent`'s tool-result mapping carry video assets end to end,
    so a command can emit a video the same way it emits an image.
  - Providers whose API has no video content type (Claude Code, Anthropic) degrade video
    blocks defensively; the marker keeps video from being attached to them in the first place.

- 2af7114: Residue cleanup: `supportedAssetTypesFor(model)` in core replaces two inline
  ternaries; the codex/grok text redactors get unambiguous vendor names
  (`redactCodexSecretText`, private grok equivalent) instead of shadowing the
  provider kit's differently-typed `redactSecretText`; claude-code quota parses
  the unified-utilization header via the shared `numberHeader`; leftover
  `editor` naming from the demi rename is gone from comments, docs, and test
  fixtures; and the real-spawn exclusions (`bash`/`sh`/`sleep`) are documented
  as a routing decision rather than a wrapper workaround.
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

# @demicodes/provider

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

## 0.3.0

### Minor Changes

- Align all public packages on 0.3.0. Highlights of this release: correct
  Claude Code context-usage reporting (no more spurious compaction on long
  tool-heavy sessions) and session storage phase 1 — role-based
  Status/View/Checkpoint/Artifact naming and bounded tool views that shrink
  session checkpoints from tens of MB to content-proportional size.

### Patch Changes

- dd69eb0: Report single-request usage instead of turn-cumulative totals from Claude Code.

  The CLI's `result.usage` sums every API call inside a turn, which inflated the
  agent's context estimation 2–3× and triggered spurious compaction on long
  tool-heavy sessions. The provider now maps the last `usage.iterations[]` entry
  (the final request's real usage) as the response usage, the provider `response`
  event documents the single-request contract, and `estimateContextTokens`
  discards anchors larger than the context window as physically impossible.

- Updated dependencies
- Updated dependencies [c352335]
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

## 0.2.0

### Minor Changes

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

- Align the workspace on 0.2.0: byte-clean binary pipelines with a model-media
  boundary, the --help flag replacing the prompt pseudo-subcommand, hardened
  command bridge execution (ephemeral shells, byte-identical stdin), unified
  provider quota surfaces, the multi-credential pool with a global active
  switch, and tool-result media delivery for OpenAI-compatible and Claude Code
  transports.

### Patch Changes

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
- 18a72d1: Restrict the OS command bridge to registered, path-safe command names; preserve
  probed quota windows when passive observations arrive; isolate Codex inference
  from quota observer failures; and document native video in `demi read` help.
- Updated dependencies [8b7b981]
- Updated dependencies [0bcb313]
- Updated dependencies [10dbc6b]
- Updated dependencies
- Updated dependencies [2af7114]
  - @demicodes/utils@0.2.0
  - @demicodes/core@0.2.0

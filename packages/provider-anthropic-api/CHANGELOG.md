# @demicodes/provider-anthropic-api

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

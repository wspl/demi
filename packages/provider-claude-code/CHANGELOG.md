# @demicodes/provider-claude-code

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

## 0.4.1

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/utils@0.4.0
  - @demicodes/provider@0.4.3

## 0.4.0

### Minor Changes

- 0a3936f: Native, remote-friendly credential logins for all pool providers.

  `beginLogin` no longer spawns a vendor CLI. Each provider drives its public
  login protocol directly and imports the result straight into the credential
  pool, returning the `credentialId`:

  - **codex** and **grok-build** run their public device-code grants; the
    verification URL and one-time code stream out via `onPending` so the user
    completes login from any browser on any device.
  - **claude-code** runs the copy-back PKCE OAuth flow: `onPending` carries the
    authorize URL (`requiresCodeInput: true`), `promptForCode` collects the
    pasted `code#state` string, and pool secrets carry refresh tokens that renew
    on expiry.

  New `ProviderCredentialLoginOptions` fields (`onPending`, `promptForCode`),
  a `ProviderCredentialLoginPending` shape, and a `credentialId` on the completed
  login result. `runVendorLoginCommand` is removed.

### Patch Changes

- Updated dependencies [0a3936f]
  - @demicodes/provider@0.4.0

## 0.3.3

### Patch Changes

- 0fd000d: Stop injecting keychain-sourced tokens into spawned CLIs. The macOS keychain
  fallback reads the Claude CLI's own short-lived access token; passing it as
  CLAUDE_CODE_OAUTH_TOKEN disables the CLI's refresh flow, so runs started
  401ing as soon as the token expired (typically after hours of idling).
  ClaudeCodeOAuthAccess now carries its resolution source, and the CLI
  injection path skips `keychain` — the CLI authenticates and refreshes
  itself. Owned sources (static/file/env, i.e. pool entries and explicit
  tokens) inject as before.

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

- dd69eb0: Report single-request usage instead of turn-cumulative totals from Claude Code.

  The CLI's `result.usage` sums every API call inside a turn, which inflated the
  agent's context estimation 2–3× and triggered spurious compaction on long
  tool-heavy sessions. The provider now maps the last `usage.iterations[]` entry
  (the final request's real usage) as the response usage, the provider `response`
  event documents the single-request contract, and `estimateContextTokens`
  discards anchors larger than the context window as physically impossible.

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

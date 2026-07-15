# @demicodes/provider-codex

## 0.4.2

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/utils@0.4.0
  - @demicodes/provider@0.4.3

## 0.4.1

### Patch Changes

- Classify transport-level transient failures (header/connect/idle timeouts, dropped sockets, DNS/connect errors) as retryable `overloaded` errors so the turn retry policy handles them instead of failing the round. The Codex provider now routes unknown thrown errors through the shared `normalizeErrorCode` instead of hardcoding `code: null`.
- Updated dependencies
  - @demicodes/provider@0.4.2

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

# @demicodes/host-local

## 0.7.0

### Patch Changes

- Updated dependencies [2bb314c]
  - @demicodes/agent@0.7.0
  - @demicodes/provider@0.7.0
  - @demicodes/shell@0.7.0
  - @demicodes/utils@0.7.0

## 0.6.1

### Patch Changes

- Updated dependencies
  - @demicodes/shell@0.6.1
  - @demicodes/agent@0.6.1
  - @demicodes/provider@0.6.1
  - @demicodes/utils@0.6.1

## 0.6.0

### Minor Changes

- 5b4f84f: Route shell operations to action-selected Hosts, bind command bridge calls to their originating shell, and expose the active Host to registered commands.

### Patch Changes

- Updated dependencies [5b4f84f]
  - @demicodes/agent@0.6.0
  - @demicodes/shell@0.6.0
  - @demicodes/provider@0.6.0
  - @demicodes/utils@0.6.0

## 0.5.0

### Minor Changes

- ca617e1: Align all first-party packages on one shared version so consumers and repository workspaces use one coherent Demi release without reconciling independent package versions.

### Patch Changes

- Updated dependencies [ca617e1]
  - @demicodes/agent@0.5.0
  - @demicodes/provider@0.5.0
  - @demicodes/shell@0.5.0
  - @demicodes/utils@0.5.0

## 0.3.4

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/agent@0.4.0
  - @demicodes/utils@0.4.0
  - @demicodes/provider@0.4.3
  - @demicodes/shell@0.3.3

## 0.3.3

### Patch Changes

- Updated dependencies [0a3936f]
  - @demicodes/provider@0.4.0
  - @demicodes/agent@0.3.3

## 0.3.2

### Patch Changes

- ca71716: Publish tarballs without the `development` export condition. The condition
  resolves to ./src for in-repo workspace resolution, but dist-only tarballs do
  not ship src — and dev-mode bundlers (Vite) enable the development condition
  by default, so consumers resolved exports to files that do not exist. The
  release pipeline now strips the condition at pack time and validates that
  every packed export target actually exists in the tarball.
- Updated dependencies [ca71716]
  - @demicodes/agent@0.3.2
  - @demicodes/provider@0.3.2
  - @demicodes/shell@0.3.2
  - @demicodes/utils@0.3.2

## 0.3.1

### Patch Changes

- Republish with resolved internal dependency ranges. The 0.3.0 tarballs shipped
  literal `workspace:^` ranges because the release went through `changeset
publish` (npm does not rewrite the workspace protocol); 0.3.0 is deprecated.
  The release pipeline now packs and publishes with bun and validates every
  tarball's manifest before anything is pushed to the registry.
- Updated dependencies
  - @demicodes/utils@0.3.1
  - @demicodes/shell@0.3.1
  - @demicodes/provider@0.3.1
  - @demicodes/agent@0.3.1

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
  - @demicodes/agent@0.3.0
  - @demicodes/utils@0.3.0
  - @demicodes/shell@0.3.0

## 0.2.1

### Patch Changes

- Republish with a runnable dependency closure: `@demicodes/just-bash`
  3.0.1-demi.5 ships the full dist its deep-path exports point at (0.2.0
  installed but could not run), and intra-workspace dependencies publish as
  caret ranges instead of exact pins so future patch releases do not require
  republishing every dependent.
- Updated dependencies
  - @demicodes/utils@0.2.1
  - @demicodes/shell@0.2.1
  - @demicodes/provider@0.2.1
  - @demicodes/agent@0.2.1

## 0.2.0

### Minor Changes

- Align the workspace on 0.2.0: byte-clean binary pipelines with a model-media
  boundary, the --help flag replacing the prompt pseudo-subcommand, hardened
  command bridge execution (ephemeral shells, byte-identical stdin), unified
  provider quota surfaces, the multi-credential pool with a global active
  switch, and tool-result media delivery for OpenAI-compatible and Claude Code
  transports.

### Patch Changes

- 8b7b981: Binary streams end to end, attachment channel removed. Pipes are byte-clean
  through real OS processes in both directions (`hostSpawn` stdin/stdout were
  UTF-8-lossy); the exec boundary classifies the final stream — valid UTF-8 is
  text, anything else surfaces as `binaryStdout` (raw bytes, truncation-aware)
  with a placeholder text render. The agent layer sniffs the closed model-media
  set by magic bytes and attaches image/video blocks when the model accepts the
  type, explaining why otherwise. `CommandAsset` / `io.asset()` and every
  `supportedAssetTypes` thread are gone; `demi read` emits raw file bytes
  (media presentation happens at the boundary); the command bridge carries
  binary stdout as base64 and the shim writes raw bytes to its OS stdout.
- 966c530: Consolidate the credential pool: the three byte-identical per-provider
  `credentials-pool.ts` copies merge into one implementation behind the
  node-only `@demicodes/provider/credentials-pool` subpath (the provider main
  entry stays platform-neutral), which also becomes the canonical home of
  `resolveDemiHome` (host-local re-exports it). Pool mutations are now
  serialized by a create-exclusive lock with unique temp names, closing the
  torn-write race between concurrent imports, and grok's `importDefault`
  activates the vendor-preferred entry deterministically by identity key
  instead of label/detail guessing.
- 18a72d1: Restrict the OS command bridge to registered, path-safe command names; preserve
  probed quota windows when passive observations arrive; isolate Codex inference
  from quota observer failures; and document native video in `demi read` help.
- 80d5c6d: Final-state cleanup of bridge exec plumbing: the command scope id is exposed
  under a single env var (`DEMI_SESSION_ID`; the exec-time `DEMI_AGENT_SESSION_ID`
  alias and the shim's fallback chain are gone), and ephemeral execs take an
  explicit `cwd` (validated as a directory) instead of a rendered `cd … &&`
  prefix in the script.
- Updated dependencies [8b7b981]
- Updated dependencies [9179edc]
- Updated dependencies [3360e35]
- Updated dependencies [bf2ffa2]
- Updated dependencies [966c530]
- Updated dependencies [0bcb313]
- Updated dependencies [10dbc6b]
- Updated dependencies [18a72d1]
- Updated dependencies
- Updated dependencies [80d5c6d]
- Updated dependencies [2af7114]
  - @demicodes/utils@0.2.0
  - @demicodes/shell@0.2.0
  - @demicodes/agent@0.2.0
  - @demicodes/provider@0.2.0

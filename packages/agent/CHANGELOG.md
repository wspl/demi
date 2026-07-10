# @demicodes/agent

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
  - @demicodes/shell@0.2.1
  - @demicodes/provider@0.2.1

## 0.2.0

### Minor Changes

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

- 9179edc: Harden command-bridge execution semantics: `AgentServer.runCommandLine` now
  runs every bridge invocation in an ephemeral shell (disposed after the call),
  so the caller's `cd`/env can never leak into the model's persistent session
  shell; and newline-terminated bridge stdin arrives byte-identical instead of
  gaining a duplicated trailing newline from heredoc rendering.
  `ShellExecInput` gains an `ephemeral` option backing this.
- 3360e35: Replace the `prompt` pseudo-subcommand with a standard `--help` flag.
  `--help` renders a node's documentation at every level — groups, dual-mode
  parents, leaves, and bare run-only roots — and wins wherever it appears among
  a command's arguments. Because help is a flag, it can never collide with
  subcommand names or positional values: the reserved-`prompt` child validation
  and the routing-precedence rule are gone, and `prompt` is an ordinary name
  again. Help-rendering APIs follow the concept: `renderCommandHelp`,
  `CommandRegistry.renderHelp()`, and `COMMAND_HELP_DEFAULTS` (which now
  advertises `--help`) replace the `*Prompt` names.
- 18a72d1: Restrict the OS command bridge to registered, path-safe command names; preserve
  probed quota windows when passive observations arrive; isolate Codex inference
  from quota observer failures; and document native video in `demi read` help.
- 80d5c6d: Final-state cleanup of bridge exec plumbing: the command scope id is exposed
  under a single env var (`DEMI_SESSION_ID`; the exec-time `DEMI_AGENT_SESSION_ID`
  alias and the shim's fallback chain are gone), and ephemeral execs take an
  explicit `cwd` (validated as a directory) instead of a rendered `cd … &&`
  prefix in the script.
- 2af7114: Residue cleanup: `supportedAssetTypesFor(model)` in core replaces two inline
  ternaries; the codex/grok text redactors get unambiguous vendor names
  (`redactCodexSecretText`, private grok equivalent) instead of shadowing the
  provider kit's differently-typed `redactSecretText`; claude-code quota parses
  the unified-utilization header via the shared `numberHeader`; leftover
  `editor` naming from the demi rename is gone from comments, docs, and test
  fixtures; and the real-spawn exclusions (`bash`/`sh`/`sleep`) are documented
  as a routing decision rather than a wrapper workaround.
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
  - @demicodes/core@0.2.0
  - @demicodes/shell@0.2.0
  - @demicodes/provider@0.2.0

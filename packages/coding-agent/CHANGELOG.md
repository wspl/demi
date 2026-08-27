# @demicodes/coding-agent

## 0.20.0

### Patch Changes

- Updated dependencies
  - @demicodes/agent@0.20.0
  - @demicodes/core@0.20.0
  - @demicodes/shell@0.20.0
  - @demicodes/utils@0.20.0

## 0.19.3

### Patch Changes

- Updated dependencies [a675924]
  - @demicodes/agent@0.19.3
  - @demicodes/core@0.19.3
  - @demicodes/shell@0.19.3
  - @demicodes/utils@0.19.3

## 0.19.2

### Patch Changes

- Updated dependencies [316969e]
  - @demicodes/agent@0.19.2
  - @demicodes/core@0.19.2
  - @demicodes/shell@0.19.2
  - @demicodes/utils@0.19.2

## 0.19.1

### Patch Changes

- Updated dependencies [baa14d3]
  - @demicodes/agent@0.19.1
  - @demicodes/core@0.19.1
  - @demicodes/shell@0.19.1
  - @demicodes/utils@0.19.1

## 0.19.0

### Minor Changes

- Subagents: parents spawn isolated child agent sessions through the injected `demi agent` command (blocking spawn with steer/abort/list/show, send-parent back-channel, subagent profiles including a read-only explore profile in the coding harness, and `subagent*` protocol frames on the parent client). Registered commands now run as virtual foreground jobs with an abort signal, live output, and a post-start stdin stream, so shell_write/shell_abort control them uniformly.

### Patch Changes

- Updated dependencies
  - @demicodes/agent@0.19.0
  - @demicodes/shell@0.19.0
  - @demicodes/core@0.19.0
  - @demicodes/utils@0.19.0

## 0.18.0

### Patch Changes

- Updated dependencies [79379f3]
  - @demicodes/shell@0.18.0
  - @demicodes/agent@0.18.0
  - @demicodes/core@0.18.0
  - @demicodes/utils@0.18.0

## 0.17.4

### Patch Changes

- @demicodes/agent@0.17.4
- @demicodes/core@0.17.4
- @demicodes/shell@0.17.4
- @demicodes/utils@0.17.4

## 0.17.3

### Patch Changes

- Updated dependencies [1b8e18e]
  - @demicodes/agent@0.17.3
  - @demicodes/core@0.17.3
  - @demicodes/shell@0.17.3
  - @demicodes/utils@0.17.3

## 0.17.2

### Patch Changes

- @demicodes/agent@0.17.2
- @demicodes/core@0.17.2
- @demicodes/shell@0.17.2
- @demicodes/utils@0.17.2

## 0.17.1

### Patch Changes

- @demicodes/agent@0.17.1
- @demicodes/core@0.17.1
- @demicodes/shell@0.17.1
- @demicodes/utils@0.17.1

## 0.17.0

### Patch Changes

- Updated dependencies
  - @demicodes/agent@0.17.0
  - @demicodes/core@0.17.0
  - @demicodes/shell@0.17.0
  - @demicodes/utils@0.17.0

## 0.16.0

### Patch Changes

- Updated dependencies [e3ec7fc]
  - @demicodes/agent@0.16.0
  - @demicodes/core@0.16.0
  - @demicodes/shell@0.16.0
  - @demicodes/utils@0.16.0

## 0.15.0

### Patch Changes

- Updated dependencies [6e26ef6]
- Updated dependencies [f57938e]
  - @demicodes/shell@0.15.0
  - @demicodes/agent@0.15.0
  - @demicodes/core@0.15.0
  - @demicodes/utils@0.15.0

## 0.14.2

### Patch Changes

- Updated dependencies [8a70829]
  - @demicodes/shell@0.14.2
  - @demicodes/agent@0.14.2
  - @demicodes/core@0.14.2
  - @demicodes/utils@0.14.2

## 0.14.1

### Patch Changes

- Updated dependencies [2da4bf6]
  - @demicodes/utils@0.14.1
  - @demicodes/agent@0.14.1
  - @demicodes/core@0.14.1
  - @demicodes/shell@0.14.1

## 0.14.0

### Minor Changes

- Remove `Command.effects` from the command contract and help renderer. Help
  carries summary, usage, outputs, parameters, stdin/heredoc fields, `--json`,
  and subcommands. See `docs/command-help.md`.

### Patch Changes

- Updated dependencies
  - @demicodes/shell@0.14.0
  - @demicodes/agent@0.14.0
  - @demicodes/core@0.14.0
  - @demicodes/utils@0.14.0

## 0.13.0

### Patch Changes

- Updated dependencies [c0ea408]
  - @demicodes/agent@0.13.0
  - @demicodes/core@0.13.0
  - @demicodes/shell@0.13.0
  - @demicodes/utils@0.13.0

## 0.12.0

### Patch Changes

- Updated dependencies [bd9c359]
  - @demicodes/shell@0.12.0
  - @demicodes/agent@0.12.0
  - @demicodes/core@0.12.0
  - @demicodes/utils@0.12.0

## 0.11.0

### Patch Changes

- Updated dependencies [5843565]
- Updated dependencies [72725c8]
  - @demicodes/agent@0.11.0
  - @demicodes/core@0.11.0
  - @demicodes/shell@0.11.0
  - @demicodes/utils@0.11.0

## 0.10.2

### Patch Changes

- Updated dependencies
  - @demicodes/agent@0.10.2
  - @demicodes/core@0.10.2
  - @demicodes/shell@0.10.2
  - @demicodes/utils@0.10.2

## 0.10.1

### Patch Changes

- Updated dependencies [eb1dcaf]
  - @demicodes/agent@0.10.1
  - @demicodes/core@0.10.1
  - @demicodes/shell@0.10.1
  - @demicodes/utils@0.10.1

## 0.10.0

### Patch Changes

- Updated dependencies [cb2a522]
  - @demicodes/agent@0.10.0
  - @demicodes/core@0.10.0
  - @demicodes/shell@0.10.0
  - @demicodes/utils@0.10.0

## 0.9.1

### Patch Changes

- @demicodes/agent@0.9.1
- @demicodes/core@0.9.1
- @demicodes/shell@0.9.1
- @demicodes/utils@0.9.1

## 0.9.0

### Patch Changes

- Updated dependencies [92e330e]
  - @demicodes/shell@0.9.0
  - @demicodes/agent@0.9.0
  - @demicodes/core@0.9.0
  - @demicodes/utils@0.9.0

## 0.8.0

### Patch Changes

- @demicodes/agent@0.8.0
- @demicodes/core@0.8.0
- @demicodes/shell@0.8.0
- @demicodes/utils@0.8.0

## 0.7.2

### Patch Changes

- @demicodes/agent@0.7.2
- @demicodes/core@0.7.2
- @demicodes/shell@0.7.2
- @demicodes/utils@0.7.2

## 0.7.1

### Patch Changes

- @demicodes/agent@0.7.1
- @demicodes/core@0.7.1
- @demicodes/shell@0.7.1
- @demicodes/utils@0.7.1

## 0.7.0

### Patch Changes

- Updated dependencies [2bb314c]
  - @demicodes/agent@0.7.0
  - @demicodes/core@0.7.0
  - @demicodes/shell@0.7.0
  - @demicodes/utils@0.7.0

## 0.6.1

### Patch Changes

- Updated dependencies
  - @demicodes/shell@0.6.1
  - @demicodes/agent@0.6.1
  - @demicodes/core@0.6.1
  - @demicodes/utils@0.6.1

## 0.6.0

### Minor Changes

- 5b4f84f: Route shell operations to action-selected Hosts, bind command bridge calls to their originating shell, and expose the active Host to registered commands.

### Patch Changes

- Updated dependencies [5b4f84f]
  - @demicodes/agent@0.6.0
  - @demicodes/shell@0.6.0
  - @demicodes/core@0.6.0
  - @demicodes/utils@0.6.0

## 0.5.0

### Minor Changes

- ca617e1: Align all first-party packages on one shared version so consumers and repository workspaces use one coherent Demi release without reconciling independent package versions.

### Patch Changes

- Updated dependencies [ca617e1]
  - @demicodes/agent@0.5.0
  - @demicodes/core@0.5.0
  - @demicodes/shell@0.5.0
  - @demicodes/utils@0.5.0

## 0.3.3

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/agent@0.4.0
  - @demicodes/utils@0.4.0
  - @demicodes/shell@0.3.3

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
  - @demicodes/core@0.3.2
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
  - @demicodes/core@0.3.1
  - @demicodes/utils@0.3.1
  - @demicodes/shell@0.3.1
  - @demicodes/agent@0.3.1

## 0.3.0

### Minor Changes

- Align all public packages on 0.3.0. Highlights of this release: correct
  Claude Code context-usage reporting (no more spurious compaction on long
  tool-heavy sessions) and session storage phase 1 — role-based
  Status/View/Checkpoint/Artifact naming and bounded tool views that shrink
  session checkpoints from tens of MB to content-proportional size.
- c352335: Session storage phase 1: role-based naming and bounded tool views (see
  docs/session-storage-and-naming.md).

  Renames — one word per role, "snapshot" retired: `ShellCommandSnapshot` →
  `ShellCommandStatus`, `StreamArtifact`/`ShellOutputArtifact` →
  `ShellStreamView`/`ShellOutputView`, `PersistedShellCommandArtifact` →
  `CommandArtifact`, `AgentSessionSnapshot` → `AgentSessionCheckpoint`
  (`checkpoint.json`, `saveCheckpoint`/`loadCheckpoint`,
  `AgentSession.fromCheckpoint`), agent class `Transcript` → `TranscriptLog`
  (with `toJSON()`), frames `transcript_snapshot` → `transcript_reset` and
  `shell_output.snapshot` → `.status`, tool_call block `metadata` → `view`.

  Bounded views — `toShellToolResult` no longer dumps the whole command status
  into the block: it stores a `ShellToolView` (commandId reference plus a
  32 KiB tail render window) instead of 3–4 duplicate stdout encodings, raw
  binary bytes, and triple diff encodings. `demi` file diffs keep `unifiedDiff`
  only. The vestigial `ToolContinuation` channel is removed. Command storage
  moves under the unified `agent-sessions/<id>/` prefix. Fixes multi-MB session
  checkpoints (measured 47.8 MB for a session whose content was ~hundreds of KB).

### Patch Changes

- Updated dependencies [dd69eb0]
- Updated dependencies
- Updated dependencies [c352335]
  - @demicodes/agent@0.3.0
  - @demicodes/core@0.3.0
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
  - @demicodes/core@0.2.1
  - @demicodes/shell@0.2.1
  - @demicodes/agent@0.2.1

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
- d203fc1: Rename the `editor` command to `demi` and give it a content-aware `read`.

  `createEditorCommand` is now `createDemiCommand`, and the registered command is
  `demi` (`demi create` / `demi edit` / `demi patch`) — a single namespace for the
  framework's built-in workspace tools rather than an edit-only "editor". The new
  `demi read <path>` reads a file: text is returned as text, images
  (png/jpeg/webp/gif) are returned as a viewable image block, and videos
  (mp4/mov/webm/m4v) are returned as a native video block — all via
  `CommandIO.asset` — so the model can actually see the media a read surfaces
  (video reaches only models whose catalog marks video support; unsupported
  models return an error before the file is read). The
  `coding-harness` option `editorHost` is now `demiHost`, and file-diff metadata
  is `file_diffs`.

- Align the workspace on 0.2.0: byte-clean binary pipelines with a model-media
  boundary, the --help flag replacing the prompt pseudo-subcommand, hardened
  command bridge execution (ephemeral shells, byte-identical stdin), unified
  provider quota surfaces, the multi-credential pool with a global active
  switch, and tool-result media delivery for OpenAI-compatible and Claude Code
  transports.

### Patch Changes

- 3360e35: Replace the `prompt` pseudo-subcommand with a standard `--help` flag.
  `--help` renders a node's documentation at every level — groups, dual-mode
  parents, leaves, and bare run-only roots — and wins wherever it appears among
  a command's arguments. Because help is a flag, it can never collide with
  subcommand names or positional values: the reserved-`prompt` child validation
  and the routing-precedence rule are gone, and `prompt` is an ordinary name
  again. Help-rendering APIs follow the concept: `renderCommandHelp`,
  `CommandRegistry.renderHelp()`, and `COMMAND_HELP_DEFAULTS` (which now
  advertises `--help`) replace the `*Prompt` names.
- bf2ffa2: `prompt` is the help pseudo-subcommand only at nodes that route to
  subcommands. At a pure run node it is an ordinary argument again, so a
  positional literally named "prompt" (e.g. `demi read prompt` for a file
  named `prompt`) executes the command instead of printing help. Leaf docs
  remain fully reachable through the parent/root help render.
- 18a72d1: Restrict the OS command bridge to registered, path-safe command names; preserve
  probed quota windows when passive observations arrive; isolate Codex inference
  from quota observer failures; and document native video in `demi read` help.
- Updated dependencies [8b7b981]
- Updated dependencies [9179edc]
- Updated dependencies [3360e35]
- Updated dependencies [bf2ffa2]
- Updated dependencies [0bcb313]
- Updated dependencies [10dbc6b]
- Updated dependencies [18a72d1]
- Updated dependencies
- Updated dependencies [80d5c6d]
- Updated dependencies [2af7114]
  - @demicodes/utils@0.2.0
  - @demicodes/core@0.2.0
  - @demicodes/shell@0.2.0
  - @demicodes/agent@0.2.0

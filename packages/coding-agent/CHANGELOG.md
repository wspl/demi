# @demicodes/coding-agent

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

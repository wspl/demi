# @demicodes/shell

## 0.19.1

### Patch Changes

- @demicodes/utils@0.19.1

## 0.19.0

### Minor Changes

- Subagents: parents spawn isolated child agent sessions through the injected `demi agent` command (blocking spawn with steer/abort/list/show, send-parent back-channel, subagent profiles including a read-only explore profile in the coding harness, and `subagent*` protocol frames on the parent client). Registered commands now run as virtual foreground jobs with an abort signal, live output, and a post-start stdin stream, so shell_write/shell_abort control them uniformly.

### Patch Changes

- @demicodes/utils@0.19.0

## 0.18.0

### Minor Changes

- 79379f3: Host-backed shells now match GNU bash on the observation surfaces models use.

  Unix names (`ls`, `grep`, `whoami`, …) spawn the PATH binary first; the portable implementation runs only when spawn reports `executable_not_found`. `Host` gains `identity` and `process.openCwd` (a directory fd on Linux) so a deleted cwd is not classified as a missing binary, children receive the exported env only, and `type` / `$UID` / `test -O` describe the Host principal. Custom Hosts must implement the new fields.

### Patch Changes

- @demicodes/utils@0.18.0

## 0.17.4

### Patch Changes

- @demicodes/utils@0.17.4

## 0.17.3

### Patch Changes

- @demicodes/utils@0.17.3

## 0.17.2

### Patch Changes

- @demicodes/utils@0.17.2

## 0.17.1

### Patch Changes

- @demicodes/utils@0.17.1

## 0.17.0

### Patch Changes

- @demicodes/utils@0.17.0

## 0.16.0

### Patch Changes

- @demicodes/utils@0.16.0

## 0.15.0

### Minor Changes

- 6e26ef6: Command artifacts are now plain files on the host filesystem; the `/@` virtual filesystem is gone.

  `Host` gains `commandArtifactsDir` — a directory reachable through `Host.fs` and visible to processes from `Host.process.spawn` (one shared filesystem namespace). Each command writes `meta.json` / `stdout.txt` / `stderr.txt` (and full `stdout.bin` for binary streams, no longer capped or base64-wrapped) under `<dir>/<storageId>/<commandId>/`, and every status/view path (`artifactDir`, `stdout.path`, placeholders) names those real files. Any tool — a portable in-shell command, a real spawned process, or the embedder's own tooling — reads and searches artifacts with ordinary file operations; embedders on virtual hosts point `commandArtifactsDir` at their virtual disk and portable commands keep working through the same paths.

- f57938e: Tree scanners (`rg` / `grep` / `find`) route to the host's real binary first. Their in-process portable implementations read files whole and burn the embedding host's main thread for minutes on large trees; a real process runs off-thread with output bounded by the capture limit. Hosts without a binary fall back to the portable implementation on first use, per shell — no configuration, no platform loss.

### Patch Changes

- @demicodes/utils@0.15.0

## 0.14.2

### Patch Changes

- 8a70829: Bound every in-memory ingestion path of the shell environment. The execution model buffers whole command outputs (in several copies) and portable commands load files whole, so one stray `rg`/`cat` over a large tree could balloon the embedding process by tens of gigabytes until the kernel OOM-killed it.

  - Foreground host processes now answer to `maxCaptureBytes` (default 64 MiB): output beyond the ceiling kills the process and fails the command with an explicit error naming the limit.
  - Background jobs retain only the most recent output within the view budget and report how much they dropped.
  - In-shell file reads (`HostBackedFileSystem`) refuse files over `maxFileReadBytes` (default 64 MiB) with an explicit error pointing at real-process routes for large files.
  - The interpreter output ceiling now aligns with the capture limit instead of a 1 GiB blanket raise.
  - @demicodes/utils@0.14.2

## 0.14.1

### Patch Changes

- Updated dependencies [2da4bf6]
  - @demicodes/utils@0.14.1

## 0.14.0

### Minor Changes

- Remove `Command.effects` from the command contract and help renderer. Help
  carries summary, usage, outputs, parameters, stdin/heredoc fields, `--json`,
  and subcommands. See `docs/command-help.md`.

### Patch Changes

- @demicodes/utils@0.14.0

## 0.13.0

### Patch Changes

- @demicodes/utils@0.13.0

## 0.12.0

### Minor Changes

- bd9c359: Remove `Command.examples` from the command contract and help renderer. Help stays declarative (summary, usage, parameters, outputs) so models compose from the contract instead of overfitting to canned invocation strings. See `docs/command-help.md`.

### Patch Changes

- @demicodes/utils@0.12.0

## 0.11.0

### Patch Changes

- @demicodes/utils@0.11.0

## 0.10.2

### Patch Changes

- @demicodes/utils@0.10.2

## 0.10.1

### Patch Changes

- @demicodes/utils@0.10.1

## 0.10.0

### Patch Changes

- @demicodes/utils@0.10.0

## 0.9.1

### Patch Changes

- @demicodes/utils@0.9.1

## 0.9.0

### Minor Changes

- 92e330e: Stop measuring media a tool produced against the limit that exists to stop log floods.

  A single `maxOutputBytes` was deciding the fate of two unrelated things. Text costs context roughly in proportion to its bytes, so its cap has to stay tight. Raw bytes are carried to be _looked at_, and there a megabyte buys far more than a megabyte of text does — against a frontier model a KiB of video costs ~2 tokens where a KiB of text costs ~280. Under one number, a cap loose enough to show a short clip also let a stray `cat` of a log file flood the window, so in practice the cap stayed tight and commands whose entire purpose was to show the model something quietly produced nothing.

  The decision now sits in two places, each owning what it knows:

  - `@demicodes/shell` gains `maxBinaryBytes` (default 16 MiB) for a raw-byte final stream, separate from `maxOutputBytes`. It stays deliberately modality-blind: which modality the bytes are, and what a given model should be shown, is not something this layer can know.
  - `@demicodes/agent` applies per-modality caps where the modality has already been sniffed and the model is already in hand — `ShellToolResultOptions.maxMediaBytes`, defaulting to 4 MiB of image and 16 MiB of video. One number cannot serve both: a KiB of image costs ~50 tokens against ~2 for video, so a cap generous enough for a five-minute clip would let a single still eat a six-figure token budget. Over-cap media is withheld with a note that names the cap and points at producing a smaller version.

  `BinaryStdout` carries the ceiling that applied as `limitBytes`, so callers can name it instead of guessing which knob to point at.

### Patch Changes

- @demicodes/utils@0.9.0

## 0.8.0

### Patch Changes

- @demicodes/utils@0.8.0

## 0.7.2

### Patch Changes

- @demicodes/utils@0.7.2

## 0.7.1

### Patch Changes

- @demicodes/utils@0.7.1

## 0.7.0

### Patch Changes

- @demicodes/utils@0.7.0

## 0.6.1

### Patch Changes

- Sync the bundled just-bash fork with upstream: builtin `rg` now searches piped stdin instead of recursively crawling the cwd (fixes shell commands like `X | rg PATTERN` hanging on hosts with large working directories), fd close redirections (`>&-`) follow the fd-sink delivery model, plus upstream interpreter/jq/tar/cat fixes.
  - @demicodes/utils@0.6.1

## 0.6.0

### Minor Changes

- 5b4f84f: Route shell operations to action-selected Hosts, bind command bridge calls to their originating shell, and expose the active Host to registered commands.

### Patch Changes

- @demicodes/utils@0.6.0

## 0.5.0

### Minor Changes

- ca617e1: Align all first-party packages on one shared version so consumers and repository workspaces use one coherent Demi release without reconciling independent package versions.

### Patch Changes

- Updated dependencies [ca617e1]
  - @demicodes/utils@0.5.0

## 0.3.3

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/utils@0.4.0

## 0.3.2

### Patch Changes

- ca71716: Publish tarballs without the `development` export condition. The condition
  resolves to ./src for in-repo workspace resolution, but dist-only tarballs do
  not ship src — and dev-mode bundlers (Vite) enable the development condition
  by default, so consumers resolved exports to files that do not exist. The
  release pipeline now strips the condition at pack time and validates that
  every packed export target actually exists in the tarball.
- Updated dependencies [ca71716]
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

- Updated dependencies
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
- 3360e35: Replace the `prompt` pseudo-subcommand with a standard `--help` flag.
  `--help` renders a node's documentation at every level — groups, dual-mode
  parents, leaves, and bare run-only roots — and wins wherever it appears among
  a command's arguments. Because help is a flag, it can never collide with
  subcommand names or positional values: the reserved-`prompt` child validation
  and the routing-precedence rule are gone, and `prompt` is an ordinary name
  again. Help-rendering APIs follow the concept: `renderCommandHelp`,
  `CommandRegistry.renderHelp()`, and `COMMAND_HELP_DEFAULTS` (which now
  advertises `--help`) replace the `*Prompt` names.
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
- bf2ffa2: `prompt` is the help pseudo-subcommand only at nodes that route to
  subcommands. At a pure run node it is an ordinary argument again, so a
  positional literally named "prompt" (e.g. `demi read prompt` for a file
  named `prompt`) executes the command instead of printing help. Leaf docs
  remain fully reachable through the parent/root help render.
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
- Updated dependencies [0bcb313]
- Updated dependencies
  - @demicodes/utils@0.2.0

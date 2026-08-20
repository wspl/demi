# @demicodes/agent

## 0.17.4

### Patch Changes

- @demicodes/core@0.17.4
- @demicodes/provider@0.17.4
- @demicodes/shell@0.17.4
- @demicodes/utils@0.17.4

## 0.17.3

### Patch Changes

- 1b8e18e: Make checkpoint persistence atomic and serialized: `LocalHostStore.writeJson` now writes through a same-directory temp file + rename so concurrent writers (other processes included) and mid-write process death can no longer tear the stored JSON, and `AgentSession` serializes checkpoint writes so a boundary flush never overlaps a scheduled write and the last completed write always carries the newest snapshot.
  - @demicodes/core@0.17.3
  - @demicodes/provider@0.17.3
  - @demicodes/shell@0.17.3
  - @demicodes/utils@0.17.3

## 0.17.2

### Patch Changes

- @demicodes/core@0.17.2
- @demicodes/provider@0.17.2
- @demicodes/shell@0.17.2
- @demicodes/utils@0.17.2

## 0.17.1

### Patch Changes

- @demicodes/core@0.17.1
- @demicodes/provider@0.17.1
- @demicodes/shell@0.17.1
- @demicodes/utils@0.17.1

## 0.17.0

### Minor Changes

- `setProvider` / `updateModel` take an explicit apply timing: `'next_turn'` (default, the original behavior — a running turn finishes on the old model) or `'immediate'`, which applies the switch at the next mid-turn sampling/tool continuation so the very next inference request already runs on the new model. When the target model needs compaction first, the old model summarizes and a resume marker is spliced in, mirroring auto-compaction.

### Patch Changes

- @demicodes/core@0.17.0
- @demicodes/provider@0.17.0
- @demicodes/shell@0.17.0
- @demicodes/utils@0.17.0

## 0.16.0

### Minor Changes

- e3ec7fc: Allow configuring auto-compaction with an absolute `preflightThresholdTokens` value. When set, it replaces the ratio-derived threshold (still clamped to the model context window); a non-finite `preflightThresholdRatio` continues to disable auto-compaction for summary clones.

### Patch Changes

- @demicodes/core@0.16.0
- @demicodes/provider@0.16.0
- @demicodes/shell@0.16.0
- @demicodes/utils@0.16.0

## 0.15.0

### Minor Changes

- 6e26ef6: Command artifacts are now plain files on the host filesystem; the `/@` virtual filesystem is gone.

  `Host` gains `commandArtifactsDir` — a directory reachable through `Host.fs` and visible to processes from `Host.process.spawn` (one shared filesystem namespace). Each command writes `meta.json` / `stdout.txt` / `stderr.txt` (and full `stdout.bin` for binary streams, no longer capped or base64-wrapped) under `<dir>/<storageId>/<commandId>/`, and every status/view path (`artifactDir`, `stdout.path`, placeholders) names those real files. Any tool — a portable in-shell command, a real spawned process, or the embedder's own tooling — reads and searches artifacts with ordinary file operations; embedders on virtual hosts point `commandArtifactsDir` at their virtual disk and portable commands keep working through the same paths.

### Patch Changes

- Updated dependencies [6e26ef6]
- Updated dependencies [f57938e]
  - @demicodes/shell@0.15.0
  - @demicodes/core@0.15.0
  - @demicodes/provider@0.15.0
  - @demicodes/utils@0.15.0

## 0.14.2

### Patch Changes

- Updated dependencies [8a70829]
  - @demicodes/shell@0.14.2
  - @demicodes/core@0.14.2
  - @demicodes/provider@0.14.2
  - @demicodes/utils@0.14.2

## 0.14.1

### Patch Changes

- 2da4bf6: Make text truncation surrogate-safe and scrub replayed text to well-formed Unicode. Shell preview and transcript replay bounding no longer split emoji into lone UTF-16 surrogates that poisoned checkpoints and made every subsequent Codex request fail with `invalid_request_error`; transcripts polluted by earlier builds are healed at replay time.
- Updated dependencies [2da4bf6]
  - @demicodes/utils@0.14.1
  - @demicodes/core@0.14.1
  - @demicodes/provider@0.14.1
  - @demicodes/shell@0.14.1

## 0.14.0

### Patch Changes

- Updated dependencies
  - @demicodes/shell@0.14.0
  - @demicodes/core@0.14.0
  - @demicodes/provider@0.14.0
  - @demicodes/utils@0.14.0

## 0.13.0

### Minor Changes

- c0ea408: Let `AgentHarness.systemPrompt`, `preamble`, and `commands` return a promise. Harness authors can now build prompts and command sets from I/O (reading workspace files, querying a store) instead of pre-computing them and closing over the result. Existing synchronous implementations keep working — the return types are widened, not replaced. `initialState` stays synchronous because `AgentSession` calls it from its constructor; async setup belongs in `host()` or a lifecycle hook.

### Patch Changes

- @demicodes/core@0.13.0
- @demicodes/provider@0.13.0
- @demicodes/shell@0.13.0
- @demicodes/utils@0.13.0

## 0.12.0

### Patch Changes

- Updated dependencies [bd9c359]
  - @demicodes/shell@0.12.0
  - @demicodes/core@0.12.0
  - @demicodes/provider@0.12.0
  - @demicodes/utils@0.12.0

## 0.11.0

### Major Changes

- 5843565: Require `AgentProvider.clone()` and add `AgentSession.clone()` for isolated session forks.

  Every provider runtime must return an independently disposable clone with the same configuration but without shared live-process / continuation state. Sessions expose `.clone()` for point-in-time copies (optional provider/runtime/state/transcript overrides); parent persistence is never inherited. See `docs/provider-session-clone.md`.

- 72725c8: Run compaction summaries through `AgentSession.clone()` on the normal turn path so prefix caches can reuse the conversation prefix. Removes the inert-reference summary request path.

### Patch Changes

- Updated dependencies [5843565]
  - @demicodes/provider@0.11.0
  - @demicodes/core@0.11.0
  - @demicodes/shell@0.11.0
  - @demicodes/utils@0.11.0

## 0.10.2

### Patch Changes

- Resume no longer chops a fresh compaction marker: `executeResume` truncates before applying a pending model switch (whose compaction splices a boundary and appends a marker), so the stale usage anchor cannot resurrect and trigger a duplicate compaction. Compaction also skips degenerate windows that contain nothing beyond the previous boundary/marker.
  - @demicodes/core@0.10.2
  - @demicodes/provider@0.10.2
  - @demicodes/shell@0.10.2
  - @demicodes/utils@0.10.2

## 0.10.1

### Patch Changes

- @demicodes/core@0.10.1
- @demicodes/provider@0.10.1
- @demicodes/shell@0.10.1
- @demicodes/utils@0.10.1

## 0.10.0

### Minor Changes

- cb2a522: Make `resume` find how far a failed turn can be unwound instead of making the caller guess.

  `retry` and `resume` were both offered as ways to recover a failed turn, so every product had to pick one — with no information to pick correctly. The only signal available was the failure kind, and it says nothing about safety: a terminal error can arrive after ten minutes of tool calls just as easily as before the first token. A product that maps "error" to retry and "abort" to resume therefore takes the destructive path exactly when it is most damaging.

  Destructive because `retry` rewinds to the user turn and reruns it. Transcript blocks stream outward as they are produced and products turn them into effects that cannot be recalled — rendering them, posting them to a chat, executing the tool they describe. Rerunning a turn that already emitted text or ran a tool duplicates work that has left the process while destroying the record of it: the model reruns believing none of it happened, then edits the same file or posts the same message a second time.

  `resume` now answers the only question that matters — what has already left the process. `findResumePoint` scans back over the leftovers of the attempt that did not finish (thinking, redacted thinking, the error marker, empty text) and stops at the first block someone may have acted on. A tool call stops it whatever its status: one still marked executing outlived the process running it, so whether its effect landed is unknown, and unknown has to be treated as landed. A `response` stops it too, since it records a request that did complete and its usage anchors the context estimate; an abort block stops it as history the user created.

  Reaching the user turn without stopping means the whole turn was discardable, and `resume` reruns it plainly — no continuation boundary attached to a stub of the model's own aborted output. Otherwise the leftovers are dropped and inference continues after the preserved progress; this also drops the stale thinking that would otherwise be replayed to the model, and the failed attempt's error marker, matching what a full rerun already did.

  `retry` keeps working and is now scoped to what it is actually good for: "regenerate", discarding the whole turn to answer the same question differently. It is documented as not being a recovery path.

  The automatic path now asks the same question. A transient failure used to be retried only when the attempt had emitted nothing at all, so a stream that reasoned for thirty seconds and then hit `overloaded` was terminal — the most common shape of a provider hiccup on a reasoning model. It is now retried, with that reasoning unwound first. Text already streamed out is still not retried: a product may have posted it, and a second attempt would post a replacement beside it rather than in place of it.

### Patch Changes

- @demicodes/core@0.10.0
- @demicodes/provider@0.10.0
- @demicodes/shell@0.10.0
- @demicodes/utils@0.10.0

## 0.9.1

### Patch Changes

- @demicodes/core@0.9.1
- @demicodes/provider@0.9.1
- @demicodes/shell@0.9.1
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

- Updated dependencies [92e330e]
  - @demicodes/shell@0.9.0
  - @demicodes/core@0.9.0
  - @demicodes/provider@0.9.0
  - @demicodes/utils@0.9.0

## 0.8.0

### Patch Changes

- @demicodes/core@0.8.0
- @demicodes/provider@0.8.0
- @demicodes/shell@0.8.0
- @demicodes/utils@0.8.0

## 0.7.2

### Patch Changes

- @demicodes/core@0.7.2
- @demicodes/provider@0.7.2
- @demicodes/shell@0.7.2
- @demicodes/utils@0.7.2

## 0.7.1

### Patch Changes

- @demicodes/core@0.7.1
- @demicodes/provider@0.7.1
- @demicodes/shell@0.7.1
- @demicodes/utils@0.7.1

## 0.7.0

### Minor Changes

- 2bb314c: Move transient inference retry into the agent runtime with four safe attempts and capped jitter backoff. Classify server, HTTP 5xx, timeout, network, and socket failures as overloaded; carry bounded request diagnostics through retry and terminal error frames; preserve completed tool progress on resume; and keep empty reasoning lifecycle events from suppressing retry.

### Patch Changes

- Updated dependencies [2bb314c]
  - @demicodes/core@0.7.0
  - @demicodes/provider@0.7.0
  - @demicodes/shell@0.7.0
  - @demicodes/utils@0.7.0

## 0.6.1

### Patch Changes

- Updated dependencies
  - @demicodes/shell@0.6.1
  - @demicodes/core@0.6.1
  - @demicodes/provider@0.6.1
  - @demicodes/utils@0.6.1

## 0.6.0

### Minor Changes

- 5b4f84f: Route shell operations to action-selected Hosts, bind command bridge calls to their originating shell, and expose the active Host to registered commands.

### Patch Changes

- Updated dependencies [5b4f84f]
  - @demicodes/shell@0.6.0
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
  - @demicodes/shell@0.5.0
  - @demicodes/utils@0.5.0

## 0.4.0

### Minor Changes

- ec89b33: Add caller-defined action metadata that crosses agent transports and remains attached to queued actions, prompt construction, reference resolution, tools, lifecycle hooks, and yield wakeups without becoming transcript or provider content.

  Export the portable JSON value type used by action metadata, including `bigint` and `Uint8Array` values preserved by Demi transports.

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/utils@0.4.0
  - @demicodes/provider@0.4.3
  - @demicodes/shell@0.3.3

## 0.3.5

### Patch Changes

- 365dd51: Complete tool calls left `executing` in a checkpoint when restoring via `AgentSession.fromCheckpoint`. A checkpoint can only hold an executing call if the process died mid-tool; replaying it without a result made providers reject every subsequent request (`No tool output found for function call ...`), deadlocking the session. Restore now completes each dangling call with an interrupted error result.

## 0.3.4

### Patch Changes

- Steers arriving while a turn is compacting now queue (like during streaming/tool phases) and materialize on the post-compaction continuation instead of being rejected. A standalone compact action materializes queued steers into the transcript so the next turn carries them. Only the finalizing phase still rejects steering.

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
  - @demicodes/provider@0.3.1

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

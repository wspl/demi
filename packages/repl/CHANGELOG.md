# @demicodes/repl

## 0.1.2

### Patch Changes

- Updated dependencies [ec89b33]
  - @demicodes/agent@0.4.0
  - @demicodes/utils@0.4.0
  - @demicodes/coding-agent@0.3.3
  - @demicodes/host-local@0.3.4
  - @demicodes/provider-claude-code@0.4.1
  - @demicodes/provider-codex@0.4.2
  - @demicodes/provider@0.4.3
  - @demicodes/provider-anthropic-api@0.3.4
  - @demicodes/provider-grok-build@0.4.1
  - @demicodes/provider-openai-api@0.3.4
  - @demicodes/shell@0.3.3

## 0.1.1

### Patch Changes

- Updated dependencies [0a3936f]
  - @demicodes/provider@0.4.0
  - @demicodes/provider-codex@0.4.0
  - @demicodes/provider-grok-build@0.4.0
  - @demicodes/provider-claude-code@0.4.0
  - @demicodes/agent@0.3.3
  - @demicodes/host-local@0.3.3
  - @demicodes/provider-anthropic-api@0.3.3
  - @demicodes/provider-openai-api@0.3.3

## 0.1.0

### Minor Changes

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
  - @demicodes/provider-claude-code@0.3.0
  - @demicodes/provider@0.3.0
  - @demicodes/agent@0.3.0
  - @demicodes/core@0.3.0
  - @demicodes/utils@0.3.0
  - @demicodes/shell@0.3.0
  - @demicodes/provider-codex@0.3.0
  - @demicodes/provider-openai-api@0.3.0
  - @demicodes/provider-anthropic-api@0.3.0
  - @demicodes/provider-grok-build@0.3.0
  - @demicodes/coding-agent@0.3.0
  - @demicodes/host-local@0.3.0

## 0.0.1

### Patch Changes

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
- Updated dependencies [32f0e41]
- Updated dependencies [3360e35]
- Updated dependencies [bf2ffa2]
- Updated dependencies [966c530]
- Updated dependencies [0bcb313]
- Updated dependencies [d203fc1]
- Updated dependencies [579e231]
- Updated dependencies [084831e]
- Updated dependencies [10dbc6b]
- Updated dependencies [09bcc0d]
- Updated dependencies [18a72d1]
- Updated dependencies
- Updated dependencies [80d5c6d]
- Updated dependencies [2af7114]
  - @demicodes/utils@0.2.0
  - @demicodes/core@0.2.0
  - @demicodes/shell@0.2.0
  - @demicodes/agent@0.2.0
  - @demicodes/coding-agent@0.2.0
  - @demicodes/host-local@0.2.0
  - @demicodes/provider-claude-code@0.2.0
  - @demicodes/provider@0.2.0
  - @demicodes/provider-codex@0.2.0
  - @demicodes/provider-grok-build@0.2.0
  - @demicodes/provider-openai-api@0.2.0
  - @demicodes/provider-anthropic-api@0.2.0

# @demicodes/web-ui

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
  - @demicodes/agent@0.3.0
  - @demicodes/core@0.3.0
  - @demicodes/utils@0.3.0

## 0.0.1

### Patch Changes

- Updated dependencies [8b7b981]
- Updated dependencies [9179edc]
- Updated dependencies [3360e35]
- Updated dependencies [0bcb313]
- Updated dependencies [10dbc6b]
- Updated dependencies [18a72d1]
- Updated dependencies
- Updated dependencies [80d5c6d]
- Updated dependencies [2af7114]
  - @demicodes/utils@0.2.0
  - @demicodes/core@0.2.0
  - @demicodes/agent@0.2.0

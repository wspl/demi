---
'@demicodes/core': minor
'@demicodes/shell': minor
'@demicodes/agent': minor
'@demicodes/coding-agent': minor
'@demicodes/web-ui': minor
'@demicodes/repl': minor
---

Session storage phase 1: role-based naming and bounded tool views (see
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

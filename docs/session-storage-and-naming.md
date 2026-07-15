# Session Storage & Naming

Final-state design for (1) the persistence-domain vocabulary and (2) the on-disk
layout of agent session data. Supersedes the current `snapshot.json` monolith.
No migration paths: old persisted sessions are historical artifacts and stay
outside runtime code.

## Motivation

Two problems, one root:

- **Vocabulary collision.** "snapshot" currently names three unrelated things:
  the durable session file (`AgentSessionSnapshot` / `snapshot.json`), the
  ephemeral per-command status view returned by `BashEnvironment.exec/status`
  (`ShellCommandSnapshot`), and the clone method `Transcript.snapshot()`.
  Meanwhile "artifact" names both the durable per-command record
  (`PersistedShellCommandArtifact`) and the *ephemeral* stream views inside the
  status (`StreamArtifact`, `ShellOutputArtifact`). Two types are both named
  `Transcript` (the core data shape and the agent's live journaled container).
- **Checkpoint bloat.** `toShellToolResult` dumps the entire
  `ShellCommandSnapshot` into `tool_call.metadata`, so every command's output is
  persisted 3–4× (stdout `delta`, `output.text`, `output.chunks[].text`, tails),
  `demi edit` persists three diff encodings (`oldText`/`newText`/`unifiedDiff`),
  and binary stdout bytes land in metadata *in addition to* the media block in
  `output`. A measured real session: 47.8 MB `snapshot.json` for ~135K tokens of
  actual model-visible content. None of those bytes buy losslessness — the shell
  already persists complete command output separately (see below); the bloat is
  pure duplication.

## Vocabulary (final state)

One word per role, applied across the session/transcript/shell domains:

| Word           | Role                                                        |
| -------------- | ----------------------------------------------------------- |
| **transcript** | conversation history (blocks)                               |
| **status**     | ephemeral point-in-time view returned by an API             |
| **checkpoint** | durable, restorable session state                           |
| **artifact**   | durable per-command record (complete output)                |
| **view**       | bounded UI-facing enhancement data on a block, never replayed to the model |
| **blob**       | content-addressed bytes (media, binary streams)             |

"snapshot" is retired from these domains entirely.

## Rename inventory

### `@demicodes/core`

| Current | Final | Notes |
| --- | --- | --- |
| `Transcript` (`{ blocks }`) | `Transcript` (keep) | The pure data shape; it is what a checkpoint embeds. |
| `Block` (tool_call) field `metadata: unknown \| null` | `view: unknown \| null` | Contract below. |
| `AgentSessionCoreState` | **delete** | Unused. |
| media `source: { mediaType, data }` | add `{ mediaType, ref }` variant | Phase 2; persisted blocks hold refs, bytes live in the blob store. |

### `@demicodes/shell`

| Current | Final | Notes |
| --- | --- | --- |
| `ShellCommandSnapshot` | `ShellCommandStatus` | It literally has a `status` discriminant and offset-relative views; it is not persistence. |
| `StreamArtifact` | `ShellStreamView` | Ephemeral incremental view of one stream. |
| `ShellOutputArtifact` | `ShellOutputView` | Ephemeral merged view (chunks). |
| `PersistedShellCommandArtifact` | `CommandArtifact` | The one true durable record; name matches `CommandArtifactStore`. |
| `BashEnvironment.snapshotCommand()` (private) | `commandStatus()` | |
| `AgentSessionCommandStorage` prefix `<sessionId>/` | `agent-sessions/<sessionId>/` | Unifies the session's storage under one prefix (layout below). |
| — | `BlobStore` (new, phase 2) | Sibling of `CommandArtifactStore`; content-addressed, session-scoped. |

### `@demicodes/agent`

| Current | Final | Notes |
| --- | --- | --- |
| class `Transcript` | `TranscriptLog` | The live, journaled container (`session.ts` already names its field `transcriptLog`). Not exported from the package root today, so low blast radius. |
| `Transcript.snapshot()` | `TranscriptLog.toJSON(): Transcript` | Says what it is: the serializable data. |
| `AgentSessionSnapshot` | `AgentSessionCheckpoint` | |
| `AgentSessionStore.saveSnapshot/loadSnapshot` | `saveCheckpoint/loadCheckpoint` | |
| `AgentSessionRestoreParams.snapshot` | `.checkpoint` | |
| `AgentSession.fromSnapshot()` | `fromCheckpoint()` | |
| `AgentSession.persistSnapshot()` (private) | `persistCheckpoint()` | |
| store key `agent-sessions/<id>/snapshot.json` | `agent-sessions/<id>/checkpoint.json` | |
| frames: `ShellCommandSnapshotLike`, field `snapshot:` | `ShellCommandStatusLike`, field `status:` | Protocol frame rename; touches `client-entry.ts`, `@demicodes/web` agent hub, REPL. |
| frame `transcript_snapshot` | `transcript_reset` | The full-sync frame replacing client transcript state; "reset" says what it does to the receiver. |
| `AgentToolInvokeResult.metadata` | `.view` | |
| `AgentToolInvokeResult.continuation` / `ToolContinuation` | **delete** | Vestigial: written into block metadata as a fallback, never read back. `ShellToolView.status/shellId/commandId` carries the same information. |
| `toShellToolResult` `metadata: result` (full status dump) | builds a `ShellToolView` | Shape below. |

Other `view` payloads produced by the agent all carry a `kind` discriminant:
`{ kind: 'yield_wakeup', wakeupId, durationMs }`,
`{ kind: 'tool_error', error }`, `{ kind: 'repeated_shell_exec', script, count }`.

### `@demicodes/coding-agent`

| Current | Final | Notes |
| --- | --- | --- |
| `FileDiffMetadata` `{ oldText, newText, unifiedDiff }` | `{ unifiedDiff }` only | One encoding is enough to render a diff; old/new full texts are dropped. |

### `@demicodes/web-ui`

`block-helpers.ts` reads `view.chunks` only; the `artifactDelta` /
`namedSection` text-parsing fallbacks are deleted (final-state, no compat
reads).

### Follow-up (optional, separate change)

`@demicodes/provider` `ProviderQuotaSnapshot` → `ProviderQuotaStatus` for full
vocabulary consistency ("status" = point-in-time view). Touches
`docs/provider-quota.md`. Low priority; not part of the phases below.

## `tool_call.view` contract

`view` is bounded, UI/host-facing enhancement data attached to a tool_call
block. Rules:

- Never replayed to the model (`collectInferenceItems` ignores it — already true
  for `metadata` today).
- Bounded size: it must not embed unbounded payloads (full stdout, file bodies,
  raw/base64 bytes). Anything unbounded lives in a command artifact or blob and
  is referenced by id.
- Typed per tool at the owning layer. Core keeps the field `unknown | null`
  (core must not know shell details); `@demicodes/agent` defines the shapes it
  produces.

The shell tool view:

```ts
interface ShellToolView {
  kind: 'shell'
  status: 'running' | 'exited' | 'aborted'
  shellId: string
  commandId: string          // key to commands/<id>/artifact.json for full output
  exitCode?: number
  runningMs: number
  idleMs: number
  /** Bounded render window: tail of the merged chunks, capped at SHELL_VIEW_MAX_CHARS. */
  chunks: ShellOutputChunk[]
  /** True when chunks were capped; the artifact has the full output. */
  viewTruncated: boolean
  audit?: BashAuditEvent[]
  commandMeta?: CommandMetadataRecord[]  // e.g. file_diffs (unifiedDiff only)
}
```

`SHELL_VIEW_MAX_CHARS = 32_768` (tail-biased: keep the newest chunks). The repeated-
exec guard result becomes `{ kind: 'repeated_shell_exec', script, count }`.
`provider-turn-loop` stops using `metadata` as a continuation fallback
(`result.metadata ?? result.continuation` today); continuation stays its own
channel.

What this deletes from persistence per command: `stdout`/`stderr` stream views
(duplicate `delta`+`tail` of the chunks), `output.text`/`output.tail`
(duplicates of `output.chunks`), `binaryStdout` (bytes already appear in
`output` as a media block when model-viewable, and are always readable at
`/@/commands/<id>/stdout.bin`), and two of the three diff encodings.

## Storage layout (final state)

Everything a session persists lives under one `HostStore` prefix:

```
<hostKey>/
  agent-sessions/<sessionId>/
    checkpoint.json                    # small: transcript + state/phase/queue/model/cwd
    journal.jsonl                      # phase 3: append-only TranscriptPatch log
    commands/<commandId>/artifact.json # complete command output (existing CommandArtifactStore)
    blobs/<sha256>                     # phase 2: content-addressed media/binary bytes
```

Agent-owned shells store command artifacts under their actual agent session id.
Anonymous shells have no `DEMI_SESSION_ID` and use their shell id only as the
internal command-artifact storage key.

### Roles

- **checkpoint.json** — the restorable session state. Transcript blocks carry
  text (already head/tail-bounded on replay), tool views (bounded), and refs.
  No unbounded payload is stored inline.
- **commands/\<id\>/artifact.json** — the lossless record of one command:
  full stdout/stderr text and binary stdout base64. Already exists
  (`CommandArtifactStore`); already exposed read-only at
  `/@/commands/<id>/{meta.json,stdout.txt,stderr.txt,stdout.bin}`. UIs that
  need more than the view window fetch it by `commandId`.
- **blobs/\<sha256\>** — media/binary bytes referenced from content blocks
  (`source.ref`). Content addressing dedupes repeated reads of the same file
  within a session structurally. Session-scoped (no cross-session sharing):
  deleting the session directory reclaims everything, no refcounting.
- **journal.jsonl** — the same `TranscriptPatch` stream that already feeds live
  UIs (`TranscriptLog.takePatches()`), appended during streaming. Restore =
  checkpoint + replay journal.

### Write path

- During streaming: append patches to `journal.jsonl` (O(delta) per write).
  Until phase 3 lands, the current throttled full-checkpoint write
  (`persistIntervalMs`, default 1 s) remains, but it shrinks from tens of MB to
  ~100s of KB after phase 1, which is what makes it acceptable in the interim.
- At action boundaries (turn end, abort, dispose): rewrite `checkpoint.json`,
  truncate `journal.jsonl`.
- Command artifacts and blobs are written by their owners
  (`CommandArtifactStore` on status transitions; `BlobStore.put` when bytes
  first appear) — the checkpoint never embeds them.

### Restore / replay path

- `AgentSession.restore` loads the checkpoint (+ replays the journal, phase 3).
- Model replay (`collectInferenceItems`) rehydrates `source.ref` media blocks
  from the blob store when building the inference request; providers still
  receive inline bytes. A missing blob (corrupted store) degrades to a text
  placeholder — it must not fail the turn.
- UI rendering uses `view.chunks` directly; "open full output" resolves
  `commandId` against the artifact.

### Lifecycle invariants

- A command artifact or blob referenced by the transcript must outlive the
  checkpoint that references it: `release()` (tombstoning) is only legal for
  commands whose tool_call blocks have been compacted away or whose session is
  being deleted. Deletion is per-session-directory, which makes this trivially
  safe.
- Checkpoint writes are atomic (temp file + rename in the host-local store);
  journal appends are naturally crash-safe (a torn tail line is discarded on
  restore).

## Phasing

Independent branches off `main`, in order of value:

1. **`fix/claude-code-turn-usage`** (implemented, pushed) — unrelated to storage but highest urgency:
   claude-code provider reports turn-cumulative usage as the `response` event
   usage, inflating context estimation 2–3× and triggering spurious compaction.
   Fix: map the last `result.usage.iterations[]` entry (real per-request usage)
   with fallback to top-level usage; document the single-request contract on
   the `response` provider event; sanity-guard `estimateContextTokens` (anchor
   > context window ⇒ fall back to char estimation).
2. **Phase 1: view slimming + renames** (implemented on `refactor/session-storage-phase1`) — everything in the rename inventory
   except blobs/journal; `ShellToolView`; storage prefix unification;
   `checkpoint.json`. Kills the 47.8 MB class of bloat except inline media in
   `output` blocks.
3. **Phase 2: blob store** — `BlobStore`, `source.ref` content blocks, replay
   rehydration. Kills inline media duplication and size.
4. **Phase 3 (optional): journal** — O(delta) streaming persistence. Do it if
   checkpoint write frequency still shows up in profiles after phases 1–2.

## Test coverage

| Module | Coverage |
| --- | --- |
| `provider-claude-code/__tests__/jsonl-output.test.ts` | `result.usage.iterations` → last entry mapped as response usage; missing `iterations` → top-level usage fallback. |
| `agent/__tests__/transcript.test.ts` | anchor > context window falls back to char estimation; normal anchor path unchanged. |
| `agent/__tests__/tools.test.ts` | `ShellToolView` shape: chunks capped at `SHELL_VIEW_MAX_CHARS` (tail kept), no `stdout`/`stderr`/`output.text`/`binaryStdout` in the view; repeated-exec view shape. |
| `agent/__tests__/session-persistence.test.ts` (new) | checkpoint round-trip via `AgentSessionStore`; restored session replays identically; checkpoint contains no unbounded payloads (guard: serialized size of a media-heavy fixture). |
| `shell/__tests__/command.test.ts` | `ShellCommandStatus` rename; artifact completeness unchanged (full stdout/binary base64 round-trip). |
| `shell/__tests__/blob-store.test.ts` (new, phase 2) | put/get round-trip, content-address dedupe, session-scoped listing/deletion. |
| `agent/__tests__/replay-rehydration.test.ts` (new, phase 2) | `source.ref` blocks rehydrate to inline bytes in inference requests; missing blob degrades to placeholder without failing the turn. |
| `coding-agent/__tests__/demi-command.test.ts` | file_diffs metadata carries `unifiedDiff` only. |
| `web-ui` block-helpers tests | rendering reads `view.chunks` exclusively. |

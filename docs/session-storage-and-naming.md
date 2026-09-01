# Session Storage & Naming

| | |
|---|---|
| Date | 2026-07-31 |
| Status | Vocabulary, views, and checkpoint naming implemented; blob store and journal remaining |
| Scope | Persistence vocabulary, `tool_call.view`, on-disk session layout |

Final-state design for (1) the persistence-domain vocabulary and (2) the on-disk
layout of agent session data. No migration paths: old persisted sessions are
historical artifacts and stay outside runtime code.

## Motivation (why the rename happened)

"snapshot" previously named three unrelated things: the durable session file,
the ephemeral per-command status view, and `Transcript.snapshot()`. Checkpoint
bloat came from dumping full command status into every `tool_call` — measured at
47.8 MB for ~135K tokens of model-visible content — while the shell already
persisted complete command output separately.

## Vocabulary (current)

One word per role, applied across the session/transcript/shell domains:

| Word           | Role                                                        |
| -------------- | ----------------------------------------------------------- |
| **transcript** | conversation history (blocks)                               |
| **status**     | ephemeral point-in-time view returned by an API             |
| **checkpoint** | durable, restorable session state                           |
| **artifact**   | durable per-command record (complete output)                |
| **view**       | bounded UI-facing enhancement data on a block, never replayed to the model |
| **blob**       | content-addressed bytes (media, binary streams) — not yet persisted |

"snapshot" is retired from these domains.

## Implemented naming

| Area | Current name |
| --- | --- |
| Ephemeral command API result | `ShellCommandStatus` |
| Ephemeral stream / merged views | `ShellStreamView`, `ShellOutputView` |
| Durable per-command record | `CommandArtifact` / `CommandArtifactStore` |
| Live transcript container | `TranscriptLog` with `toJSON(): Transcript` |
| Durable session state | `AgentSessionCheckpoint`, `saveCheckpoint` / `loadCheckpoint`, `fromCheckpoint` |
| Store key | `agent-sessions/<sessionId>/checkpoint.json` |
| Protocol full-sync frame | `transcript_reset` |
| Protocol status frame field | `status` (`ShellCommandStatusLike`) |
| Tool result enhancement | `AgentToolInvokeResult.view` / `block.view` |
| File diffs in coding-agent | `{ unifiedDiff }` only |

Optional follow-up (separate change): rename `@demicodes/provider`
`ProviderQuotaSnapshot` → `ProviderQuotaStatus` for vocabulary consistency.

## `tool_call.view` contract

`view` is bounded, UI/host-facing enhancement data attached to a tool_call
block. Rules:

- Never replayed to the model (`collectInferenceItems` ignores it).
- Bounded size: it must not embed unbounded payloads (full stdout, file bodies,
  raw/base64 bytes). Anything unbounded lives in a command artifact or blob and
  is referenced by id.
- Typed per tool at the owning layer. Core keeps the field `unknown | null`
  (core must not know shell details); `@demicodes/agent` defines the shapes it
  produces.

The shell tool view (implemented):

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

## Storage layout (final state)

Everything a session persists lives under one `HostStore` prefix:

```
<hostKey>/
  agent-sessions/<sessionId>/
    checkpoint.json                    # transcript + state/phase/queue/model/cwd
    journal.jsonl                      # remaining: append-only TranscriptPatch log
    commands/<commandId>/artifact.json # complete command output (CommandArtifactStore)
    blobs/<sha256>                     # remaining: content-addressed media/binary bytes
```

Agent-owned shells store command artifacts under their actual agent session id.
Anonymous shells have no `DEMI_SESSION_ID` and use their shell id only as the
internal command-artifact storage key.

### Roles

- **checkpoint.json** (implemented) — the restorable session state. Transcript
  blocks carry text, tool views (bounded), and (eventually) refs. No unbounded
  command-status dump is stored inline.
- **command artifacts** (implemented) — the lossless record of one command,
  written as plain files under `Host.commandArtifactsDir/<storageId>/<commandId>/`
  (`meta.json`, `stdout.txt`, `stderr.txt`, `stdout.bin`): one filesystem
  namespace shared with spawned processes, so any tool reads and searches them
  with ordinary file operations.
- **blobs/\<sha256\>** (remaining) — media/binary bytes referenced from content
  blocks (`source.ref`). Session-scoped content addressing; deleting the session
  directory reclaims everything.
- **journal** (remaining, **required design**) —
  incremental transcript persistence: streaming appends finished blocks;
  nothing rewrites the whole transcript per interval. `journal.jsonl` is its
  file-backed realization here; the product backend realizes the same
  contract as one row per block in a per-conversation SQLite database (see
  `docs/demi-next.md` § Database). The persistence contract therefore
  becomes append-block + save-state, not save-whole-checkpoint.

### Write path (today vs final)

- Today: throttled full-checkpoint writes (`persistIntervalMs`, default 1 s) plus
  boundary rewrites at turn end / abort / dispose. After view slimming this is
  ~100s of KB for ordinary sessions, not tens of MB — still a whole-transcript
  rewrite per interval, which is not the final write path.
- Final with journal: append finished blocks during streaming; persist the
  non-transcript state at action boundaries. In the file realization that is
  `journal.jsonl` + a slim `checkpoint.json`; in the product backend it is
  block rows + a state row in `conversations/<id>.sqlite`.
- Command artifacts (and future blobs) are written by their owners — the
  checkpoint never embeds them.

### Restore / replay path

- `AgentSession` restore loads the checkpoint (and will replay the journal once
  that lands).
- Remaining: model replay rehydrates `source.ref` media blocks from the blob
  store when building the inference request; providers still receive inline
  bytes. A missing blob degrades to a text placeholder — it must not fail the turn.
- UI rendering uses `view.chunks` directly; "open full output" resolves
  `commandId` against the artifact.

## Remaining work

Independent branches off `main`:

1. **Blob store** — `BlobStore`, `source.ref` content blocks, replay rehydration.
   Removes inline media duplication from checkpoints.
2. **Journal** (optional) — O(delta) streaming persistence if checkpoint write
   frequency still shows up in profiles after the blob store.

## Test coverage

| Module | Coverage |
| --- | --- |
| `provider-claude-code/__tests__/jsonl-output.test.ts` | `result.usage.iterations` → last entry mapped as response usage; missing `iterations` → top-level usage fallback. |
| `agent/__tests__/transcript.test.ts` | anchor > context window falls back to char estimation; normal anchor path unchanged. |
| `agent/__tests__/tools.test.ts` | `ShellToolView` shape: chunks capped at `SHELL_VIEW_MAX_CHARS` (tail kept), no unbounded status dump in the view; repeated-exec view shape. |
| `agent/__tests__/session-persistence.test.ts` | checkpoint round-trip via `AgentSessionStore`; restored session replays identically. |
| `shell/__tests__/command.test.ts` | `ShellCommandStatus`; artifact completeness (full stdout/binary base64 round-trip). |
| `coding-agent/__tests__/demi-command.test.ts` | file_diffs metadata carries `unifiedDiff` only. |
| `web-ui` block-helpers | rendering reads `view.chunks` exclusively. |
| `shell/__tests__/blob-store.test.ts` (remaining) | put/get round-trip, content-address dedupe, session-scoped listing/deletion. |
| `agent/__tests__/replay-rehydration.test.ts` (remaining) | `source.ref` blocks rehydrate to inline bytes; missing blob degrades without failing the turn. |

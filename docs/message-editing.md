# Message editing and resend

Status: Implemented. This document defines the current behavior and regression
contract.

## Behavior and ownership

Editing a user message replaces that message and the transcript suffix with a
new user turn. `AgentSession.editAndSend` owns the operation. The request names
the target block, supplies the complete replacement content, carries a stable
operation ID, and identifies the transcript snapshot used by the editor.

```text
A → answer A → B → answer B → C → answer C
                    edit B and submit
A → answer A → B′ → answer B′
```

`TranscriptLog` owns suffix replacement and its replication patches.
`AgentSession` prepares the replacement turn and harness state, commits the
checkpoint, publishes the accepted history, and starts inference with an
independent provider runtime. The replacement uses a new turn ID, the selected
model, and freshly prepared preamble and references. Retained blocks preserve
their IDs, timestamps, content and media bytes.

User blocks keep the submitted `content`. When reference resolution changes that
content, `resolvedContent` keeps the materialized input for inference and replay.
The editor reads `content`; submitting an edit resolves its references again.
Media externalization and hydration cover both fields. Equal submitted and
resolved content uses only `content`.

Harnesses opt into editing through `restoreState`, which reconstructs a detached
plain state record from the retained transcript. Commit preserves the live root
record's identity so command closures observe the replacement fields. Consumers
read nested state through that root after a rewrite. Preparation hooks may append
new blocks but cannot modify the retained prefix.

`AgentServer` coordinates admission with the session's child supervisor. Editing
requires no active action, queued message, pending steer, scheduled wakeup, live
child, or child completion awaiting delivery. Admission excludes competing
actions and completion delivery while the edit is prepared and committed.
Editable targets are explicit user submissions; steer blocks, hidden turns,
internal completion messages and assistant blocks are not editable targets.

The injected session store commits transcript rows and harness state atomically.
Preparation and persistence failures preserve the accepted history. A successful
commit accepts the replacement even if inference subsequently fails. Disconnect
does not cancel an accepted edit. Repeating an accepted operation does not apply
the rewrite again; reuse of its ID for a different edit is rejected. Accepted
operations are reconciled before snapshot validation. An unapplied edit with a
stale snapshot is rejected without mutation, including after server restart or
a later edit. A replayed request cannot restore a superseded transcript suffix.

Harness state derived from conversation history is reconstructed from the
retained prefix, or from initial state when no applicable snapshot remains.
A harness that cannot provide that reconstruction rejects editing before commit.
Files, command storage, completed child records and already executed tool effects
remain external state; transcript editing does not undo them.

`web-ui` owns the editor and submission interaction; `web` supplies product state
and handlers, and `web-gallery` supplies examples of the same components.
The UI offers editing only on the last explicit user submission. Assistant
output, hidden inputs, completion messages and steers do not change that target.
This is a browser presentation rule; the agent API retains its general
targeted-edit contract.
`SessionComposer` displays the original content and attachments in the existing
composer. The target message and subsequent transcript blocks remain visible at
reduced opacity and cannot receive pointer or keyboard interaction. The × to the
left of the send button or Escape exits an unsubmitted edit and restores the unrelated
composer draft. Each text part remains independently editable, preserving its
position relative to attachments in the submitted content. Enter submits;
Shift+Enter inserts a newline and active IME composition does not submit.
Entering or canceling editing does not modify the transcript. Save and resend
submits one operation. The draft and attachments remain recoverable until the
client confirms acceptance. An uncertain result is reconciled by operation ID
before resubmission. Sending and uncertain phases lock content and exit controls;
explicit rejection restores editing. Successful acceptance restores the composer
draft. Local attachment reads are aborted on exit or unmount.

## Acceptance invariants

Tests observe four independently obtained results: the session transcript, the
client transcript reconstructed from frames, the checkpoint loaded from storage,
and requests delivered to the provider. Provider assertions use hand-authored
expected inference items; calling the production replay helper to construct the
expected result would conceal replay defects.

1. The retained prefix is unchanged. The replacement appears once. No removed
   answer, steer, tool result or invalid summary reaches subsequent inference.
2. Before durable acceptance, neither clients nor the replacement provider observe
   the candidate history. A failed save leaves both memory and storage unchanged,
   including after pending checkpoint work has finished.
3. After acceptance, reconnect and restart recover the same replacement history.
   A generation error does not restore the removed suffix or duplicate the input.
4. Each accepted operation performs at most one rewrite. This is not a guarantee
   of one provider request: tool continuations, compaction and explicit recovery
   can make additional requests under their own contracts.
5. Rejected, failed and canceled preparation releases reservations and candidate
   resources. Successful replacement releases the discarded provider runtime.
   Completion callbacks, timers and pending writes cannot mutate the replacement
   from a discarded action.

## Fixtures and deterministic scheduling

- Build a three-turn transcript containing distinct sentinel values for each
  user message, assistant answer, steer, tool call/result and state snapshot.
  Give attachments distinct bytes, including two files with the same name.
- Reuse `MemorySessionStore` and scripted providers for session tests. Extend
  the recording-provider fixture with per-runtime identity, detached request
  captures and disposal observations. Share only the script/observation sink;
  each clone owns independent consumed-context and continuation state.
- Use the Claude Code fake transport factory to observe transport starts,
  input writes and termination. No installed CLI or model account is required.
- Use deferred promises as barriers at preparation, save, publication and
  provider start. Exercise both orders of competing operations explicitly.
  Control wakeups with an injected clock or test timers; arbitrary sleeps are
  not synchronization.
- SQLite tests use a temporary on-disk database and blob store. Wrap the
  `SqlDatabase` interface to inject a failure inside a real transaction, after
  some statements have executed. A fake store throwing before any write does
  not establish transactional rollback.
- Every test releases barriers and disposes sessions, providers, transports,
  timers and temporary storage in teardown, including on assertion failure.

## Transcript and compaction cases

Focused cases live in `packages/agent/src/__tests__/editing.test.ts`, alongside
the existing transcript, patch and compaction regression suites.

| Case | Required observation |
| --- | --- |
| Edit first, middle or last user turn | Exact retained prefix and one replacement; suffix IDs disappear. |
| Multiple text blocks and attachments | All replacement blocks arrive in order; preserved bytes and metadata are equal. Caller mutations cannot alter accepted content. |
| Suffix contains steers, tool results, abort/resume blocks and state snapshots | None survives the cut or leaks into replay. Retained tool calls and results remain correctly paired. |
| Edit before a compaction boundary | The summary covering edited content disappears; retained original blocks supply the context. |
| Edit after a compaction boundary | The valid prefix summary remains; the replaced turn and later content disappear. |
| Multiple boundaries or boundary inside a turn | Only summaries describing retained content can reach inference. Boundary/marker references remain valid under replay and context accounting. |
| Cut removes a compaction marker or latest usage response | Context estimation does not use an invalid usage anchor. A preflight summary request contains only retained content and the replacement where applicable. |
| Patch application and reload | Applying emitted patches to the initial client copy produces the accepted transcript exactly; save/load produces the same blocks. |

The fixed-seed cases generate bounded valid histories and edit positions. Their
fixture-level prefix/replacement oracle is separate from semantic assertions on
inference items and patch-array assertions.

## Session, provider and admission cases

Session and admission cases live in `agent/src/__tests__/editing.test.ts` and
`subagent.test.ts`; transport isolation cases live in
`provider-claude-code/src/__tests__/provider.test.ts`, under `packages/`.

| Case | Required observation |
| --- | --- |
| Edit B in the three-turn fixture | The generation request contains A, answer A and B′; sentinel values belonging to B, answer B, C and answer C are absent. |
| Edit the final non-first user message | A fresh runtime receives the replacement even though user-message count and the first user message are unchanged. |
| Change only attachment bytes | A fresh runtime receives the new bytes; equal text, filenames and media types do not permit stale context reuse. |
| Stateful provider has consumed removed content | No continuation is sent to its transport; replacement inference starts independently. Disposal does not terminate the replacement runtime. |
| Preamble, reference resolver or state reconstruction fails | No published rewrite, durable mutation or replacement inference; the next valid operation can acquire admission. |
| Preparation is aborted or provider creation fails | The candidate is discarded, the accepted transcript gains no candidate abort/output blocks, and acquired resources are released. |
| Commit is in progress when abort arrives | The durable outcome determines acceptance: failure preserves the accepted history; success retains the replacement and prevents generation from starting when cancellation has been accepted. |
| Discarded runtime disposal fails | The failure is observable, admission is released, and no request falls back to the consumed runtime. A committed replacement remains accepted. |
| Prefix has a state snapshot, or has none | Harness-observed state is restored from the prefix or initial state; removed snapshots cannot affect the prompt or tools. |
| Model fails before output or after a completed tool | The replacement remains accepted. Explicit recovery follows the resume contract and does not repeat a completed tool solely because submission was retried. |
| Active action, queue, pending steer, wakeup, live child or undelivered completion | Editing is rejected without deleting, consuming or silently canceling that work. |
| Send, model/target change, child resume or completion delivery races with edit | Exactly one admissible ordering takes effect; the loser observes busy/conflict or operates on the committed state. No callback enters the preparation window. |
| Completed external effects | A sentinel file, command-state value and archived child record remain intact; delivered child completions are not replayed because their parent blocks were removed. |

## Durability and failure boundaries

`packages/backend/src/__tests__/editing-storage.test.ts` exercises the real
SQLite store and process crash boundaries. `scenarios/editing.test.ts` in that
test directory exercises an authenticated scripted-model backend and restart.

| Fault or pause | Required observation |
| --- | --- |
| Save is blocked | Connected clients retain the accepted transcript; replacement inference has not started. |
| Blob externalization fails | No checkpoint changes; retained media remains readable. |
| SQL fails after block writes/deletion and before transaction completion | Reopen the database and observe the complete pre-edit checkpoint, never mixed blocks/state. |
| An earlier checkpoint write is in flight | Edit commit is serialized with it; no delayed write restores removed rows or overwrites replacement state. |
| Commit succeeds but acceptance delivery is lost | Reconnect recovers the replacement; repeating the operation produces no second rewrite. |
| Process exits before commit | Reload recovers the complete accepted pre-edit checkpoint. |
| Process exits after commit and before publication or provider start | Reload recovers the replacement and an actionable unfinished turn. Retrying submission confirms acceptance; inference recovery is a separate action. |
| Process exits after generated output has been checkpointed | Reload retains that output and uses ordinary incomplete-turn recovery where needed. |

Crash tests terminate a separate scripted `AgentSession` process using the
backend's real SQLite and blob stores, without session disposal or a final
checkpoint. The parent test reopens that database in a fresh runtime. These
storage-boundary crash tests and the full authenticated backend's graceful
restart test cover different failure boundaries.

## Protocol and UI cases

Protocol coverage lives in `agent/src/__tests__/server.test.ts`,
`client-submit.test.ts`, `transcript-pipeline.test.ts`, the backend's scoped
transport tests, and the shared UI's agent tests under `packages/`.

- Reject malformed content, invalid targets, unauthorized session access and stale
  snapshot tokens without provider calls or transcript changes.
- Exercise repeated operation IDs before acceptance, after acceptance, after
  reconnect/restart, and after a subsequent edit removes the replacement turn.
  Identical retries do not rewrite again; conflicting payloads are rejected.
- Create two editors from the same snapshot. Accept one edit, then submit the
  other through the supported connection/takeover flow and assert conflict.
- Drop a transcript patch and require snapshot resynchronization. Lose an
  acceptance frame and require reconciliation. Neither case guesses success
  from message text or silently generates a fresh operation ID.
- Cancel preserves the transcript and unrelated composer draft. Save preserves
  every text block and attachment, blocks duplicate activation while pending,
  and retains the edited draft after validation or persistence failure.
- After acceptance, generation failure shows turn recovery rather than offering
  to submit the accepted edit as another new message.
- Gallery examples cover editing, saving, conflict and failure using the shared
  `web-ui` behavior. Browser acceptance covers keyboard submission, Chinese input,
  multiline text, attachments, cancel, reconnect and the replaced transcript.
  The shared keyboard predicate is tested with active IME composition events.
  The product acceptance fixture supplies a scripted backend and checks captured
  requests as well as the visible page.

## Completion gate

Implementation is accepted when all invariant groups above have automated
coverage, the focused suites pass, TypeScript and Vue checks pass, and shared
Gallery/product browser acceptance passes. UI screenshots alone do not establish
transcript correctness or durability.

Run explicit test-file paths with `bun test --conditions development`, followed
by `bun run typecheck` and `bun run typecheck:web`. Inspect each selected fixture
before running it. Tests use fake providers and transports; model-backed E2E
suites and live compaction fixture generation are excluded.

Check the strength of the suite with temporary targeted defects: retain one
removed block, reuse the consumed provider runtime, publish before save, or
accept a stale snapshot. Each defect must fail its corresponding assertion.
Remove these defects before the implementation checkpoint and rerun the affected
tests. Report commands and results in the task response, without adding a
validation record to the repository.

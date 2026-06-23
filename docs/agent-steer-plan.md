# Agent Steer Plan

| | |
|---|---|
| 日期 | 2026-06-23 |
| 状态 | 实现前设计完成 |
| 范围 | `@demi/agent` session runtime, transport frames, `AgentClient`, provider contract, REPL/Web input surfaces |

## 1. Problem

Demi 当前只有 queued input：`send` 在 session busy 时进入下一 turn 的队列，等当前 turn 结束后再执行。这能保证顺序，但不能表达用户在 agent 正在工作时给当前工作追加方向修正。

Codex 的工作模型里 `queue` 和 `steer` 是两种不同的 active session input policy：

- `queue`：把输入排到下一 turn，不影响当前 in-flight turn。
- `steer`：把输入追加到当前 in-flight turn，不创建新 turn，让当前工作即时收到方向修正。

这不是 UI 文案差异。它影响 turn id、transcript replay、provider transport、tool continuation、retry/resume、promise 收敛和测试接受标准。

## 2. Final Semantics

### 2.1 Definitions

- **Turn**：一次用户发起的 agent 工作单元，有稳定 `turnId`。一次 turn 内可以包含多次 provider request continuation、tool execution、auto compaction 后的 resume continuation。
- **Base user input**：启动 turn 的用户输入。
- **Queued input**：一个尚未启动的新 turn。它必须等当前 active turn 完成后才进入 transcript 和 provider request。
- **Steer input**：追加到当前 active turn 的用户输入。它属于当前 `turnId`，必须进入 transcript，但不能创建新的 user turn。
- **Active turn**：`AgentSession` 正在执行 `send` / `retry` / `resume` 中的一个 turn，且还未发出该 turn 的最终 idle 收敛。

### 2.2 Invariants

1. `queue` and `steer` are explicit caller choices. The runtime must not silently convert one into the other.
2. A rejected steer is not written to transcript and is not queued automatically.
3. An accepted steer is durable transcript data and belongs to the active `turnId`.
4. Steer must not reorder queued turns. Queued turns drain only after the active turn finishes.
5. Steer must not interrupt or rewrite tool results by itself. If the user wants cancellation, that remains `abort`; if the user wants shell stdin, that remains `shell_input`.
6. Steer accepted during tool execution is delivered before the next provider continuation in the same turn.
7. Steer accepted during provider streaming requires provider-level active-run support. If the provider cannot receive in-flight input, the session rejects the steer instead of pretending it was applied.
8. Compaction and retry must treat a base user input and its accepted steers as one logical turn group.

## 3. Package Boundary Placement

The package boundary remains the highest constraint:

- `@demi/core` owns shared data types only: add portable transcript and content types needed to represent steer.
- `@demi/provider` owns the abstract inference contract: add provider-neutral active-run steering types and capability shape.
- `@demi/agent` owns session state, turn grouping, steer acceptance rules, transport frames, and `AgentClient` APIs.
- `@demi/shell` remains unaware of steer. `shell_input` stays shell-process stdin, not agent steering.
- `@demi/coding-agent` may adjust prompt wording for steered turns, but must not instantiate sessions or replace runtime behavior.
- `@demi/repl`, `@demi/web-ui`, and `@demi/web` expose explicit input affordances and call `AgentClient.steer` or queue/send APIs.
- Concrete provider packages implement native steering only inside their own transport boundaries.

## 4. Current Code Feasibility Audit

This section ties the design to the current implementation so the implementation can start without redoing the architecture analysis.

### 4.1 Core Types

Current facts:

- `packages/core/src/index.ts` defines `Block` without `turnId` and without a `steer` block.
- `SessionPhase` is only `idle | running | compacting`; that is enough for public UI state, but not enough for internal steer acceptance. Internal active-turn phase should stay inside `@demi/agent`.
- `QueuedMessage` already carries `content`, so queued sends can keep their current behavior without schema changes.

Feasibility:

- Adding `turnId` to `user` and `resume`, plus adding a new `steer` block, is a direct type expansion. It will intentionally create compile errors in transcript tests, renderers, visible-block filters, and fixtures; those errors are useful implementation guidance.
- Assistant, tool, response, error, and abort blocks do not need `turnId` for the first implementation because transcript order already groups them. The active turn id is needed on user-facing turn markers: `user`, `resume`, and `steer`.

### 4.2 Transcript Runtime

Current facts:

- `Transcript.pushUserTurn()` creates a `user` block with a generated `id`, but does not receive the session's `activeTurnId`.
- `Transcript.pushResumeTurn()` creates a `resume` block without an explicit turn id.
- `Transcript.collectInferenceItems()` maps `user` and `resume` to `user_message`, and has no `user_steer` item.
- `findLastUserTurnIndex()` in `AgentSession` truncates everything after the latest `user` during retry. If `steer` blocks are appended after that user, current retry would delete them.
- `estimateBlockText()` has no steer case, so compaction token estimates and summaries need a steer branch.
- Compaction currently cuts by block order and prefers `response` boundaries. It can preserve steer ordering if `steer` is a normal transcript block and cut selection avoids splitting logical turns.

Feasibility:

- Make `pushUserTurn(model, turnId, content, preamble)` and `pushResumeTurn(model, turnId)` explicit.
- Add `pushSteer(turnId, model, content)`.
- Add `InferenceItem` variant `{ type: 'user_steer'; turnId: string; content: UserContentBlock[] }`.
- `collectInferenceItems()` should emit `user_steer` at the steer block's actual transcript position. That preserves ordering relative to assistant text and tool calls. For a tool currently executing, the existing `tool_call` block is already before the steer block; when the tool completes, `collectInferenceItems()` emits tool result before the steer because output lives inside that earlier tool block.
- Retry must be changed from "splice after latest user" to "preserve accepted steers for the retried logical turn". The direct implementation is: find the latest user block, capture later steer blocks with the same `turnId`, splice generated assistant/tool/result blocks, then reinsert those steer blocks immediately after the base user for the retry request.

### 4.3 AgentSession

Current facts:

- `AgentSession` already has `activeTurnId`, `currentAbortController`, `currentPhase`, `pendingActions`, and `queued`.
- `send()` calls `enqueue()`. While busy, `enqueue()` pushes the send into `queued` and `pendingActions`.
- `runWorker()` sets `activeTurnId` before every action. For `send`, it uses the send action id; for `retry` and `resume`, it currently generates a fresh id.
- `streamProviderOnce()` creates a provider request, then immediately iterates `this.provider.run(request)` through `providerEvents()`. It does not retain a provider run handle.
- `executePendingTools()` awaits tool invocation. During that await, a concurrent public `steer()` method can run on the same JS event loop if the session exposes one.
- `commitTranscript()` emits transcript patches and saves snapshots. A steer implementation should use the same path; it should not mutate transcript without going through `commitTranscript()`.

Feasibility:

- Keep the existing worker FIFO for `send/retry/resume/compact`. Add `steer()` outside `pendingActions`.
- Add private active-turn details without changing public `SessionPhase`: `activeTurnPhase: 'provider_streaming' | 'tool_executing' | 'compacting' | 'finalizing' | null` and `activeProviderRun: ProviderRun | null`.
- Set `activeTurnPhase` around `streamProviderOnce()`, `executePendingTools()`, compaction, and final cleanup.
- Store the result of `this.provider.run(request)` in `activeProviderRun` while streaming so `steer()` can call `activeProviderRun.steer`.
- For retry, use the retried user block's `turnId` as the request `turnId`; do not invent a new logical turn id.
- For resume, generate a new turn id and pass it to `pushResumeTurn()`.
- Reject `steer()` if `externalMutationReserved` is true, no active turn exists, the active turn is compacting/finalizing, reference resolution fails, or no delivery path exists.

### 4.4 Provider Contract

Current facts:

- `AgentProvider.run(request)` returns `AsyncIterable<ProviderEvent>`.
- Existing concrete providers and tests mostly implement `async *run(...)`.
- TypeScript can structurally treat an async iterable as a `ProviderRun` if `steer` is optional.

Feasibility:

- Add `InferenceSteer` and `ProviderRun extends AsyncIterable<ProviderEvent> { steer?(input: InferenceSteer): Promise<void> | void }`.
- Change `AgentProvider.run()` to return `ProviderRun`.
- Existing `async *run()` implementations can remain valid because `steer` is optional. Tests that explicitly annotate `AsyncIterable<ProviderEvent>` may need type updates only where the compiler demands it.
- Add a provider test helper to wrap an async iterable with a `steer` function for session/server tests.

### 4.5 Agent Transport And Client

Current facts:

- `ClientFrame` has no `steer`.
- `ServerFrame` has no per-action ack except generic `rejected`.
- `AgentClient` resolves `send/retry/resume/compact` using a phase FIFO. This is correct for queued turn actions but not for steer, because steer does not start or finish a phase cycle.
- `AgentServer.handleFrame()` has no steer branch and sends action promises through `observeSessionAction()`.

Feasibility:

- Add `ClientFrame { type: 'steer'; steerId: string; content: UserContentBlock[] }`.
- Add `ServerFrame { type: 'steer_result'; steerId: string; status: 'accepted' | 'rejected'; reason?: string }`.
- Add `AgentClient.steer()` with a waiter map keyed by `steerId`.
- `closed` and `error` must settle pending steer waiters as well as normal action waiters.
- Server should handle steer separately from `observeSessionAction()`: call `session.steer(frame.content)`, then send `steer_result` for the frame's `steerId`.

### 4.6 REPL And Web UI

Current facts:

- REPL non-command input always calls `client.send()`. While busy, that becomes queue.
- REPL command surface has `/abort`, `/retry`, `/resume`, `/compact`, and `/input`; no `/steer`.
- Web UI `handleSubmit()` always calls `workspace.send()`. When the session is running and the editor has content, the send icon still calls send, which currently means queue.
- `AgentWorkspace` and `ConversationRuntime` expose `send/abort/retry/resume/compact`, but no `steer`.

Feasibility:

- Add `steer()` through `ConversationRuntime` and `AgentWorkspace`.
- REPL can use `/steer <text>` as the explicit first affordance. Non-command input should keep its current queued behavior while running until a separate keybinding is intentionally designed.
- Web UI should expose two explicit running-state actions when editor content exists: steer current turn and queue next turn. It must not silently change existing send semantics.
- Accepted steer visibility comes from the transcript patch for the new `steer` block; queue visibility remains the `queue` frame.

### 4.7 Concrete Providers

Current facts:

- `@demi/provider-codex` builds one request body and calls `transport.stream(...)`.
- `FetchCodexResponsesTransport` is SSE-only and has no active control channel.
- `WebSocketCodexResponsesTransport` owns the socket inside `stream()` and currently sends only `response.create`. There is no object retained outside the generator that can send a later steer message.
- `AutoCodexResponsesTransport` falls back from WebSocket to SSE only if WebSocket fails before any event is yielded. After events start, WebSocket failure is fatal.
- `@demi/provider-claude-code` keeps a long-lived CLI process, but its current continuation logic sends only new user/tool-result input at turn boundaries. There is no proven in-flight user steering channel.

Feasibility:

- Native active provider-stream steering is feasible first for Codex WebSocket, but requires refactoring `WebSocketCodexResponsesTransport.stream()` into a run object that owns the socket and exposes a local `steer()` send path.
- Codex SSE must not expose `ProviderRun.steer`.
- Codex auto mode can expose steer only when the active run is actually WebSocket. If auto falls back to SSE, active provider-stream steer must reject.
- Claude Code should initially return runs without `steer`; tool-execution-time steer can still be accepted by `AgentSession` because it is delivered in the next provider continuation, but provider-stream-time steer must reject.

## 5. Transcript Model

Steer needs a first-class transcript block. Reusing `user` would incorrectly create a new turn; hiding it in assistant text would break audit and replay.

Final shape:

```ts
type Block =
  | {
      type: 'user'
      id: string
      turnId: string
      createdAt: string
      model: ModelSelection
      content: UserContentBlock[]
      preamble: string | null
    }
  | {
      type: 'resume'
      id: string
      turnId: string
      createdAt: string
      model: ModelSelection
    }
  | {
      type: 'steer'
      id: string
      turnId: string
      createdAt: string
      model: ModelSelection
      content: UserContentBlock[]
    }
  // existing assistant/tool/response/abort/compaction blocks
```

Rules:

- The base `user` or `resume` block and every `steer` block in that turn share `turnId`.
- Rejected steer attempts are surfaced as protocol rejections, not transcript blocks.
- Accepted steers are appended in receive order.
- Future transcript replay exposes steer as provider input through a provider-neutral `InferenceItem`, not as a fresh queued turn.
- Assistant/tool/result blocks do not need explicit `turnId` in the first implementation because block order remains the authoritative replay order.

Provider-facing replay item:

```ts
type InferenceItem =
  | { type: 'user_message'; content: UserContentBlock[] }
  | { type: 'user_steer'; turnId: string; content: UserContentBlock[] }
  // existing assistant/tool items
```

Provider adapters decide how to map historical `user_steer` to their protocol. A provider without historical steer metadata can map it to a normal user input item while preserving order, but active in-flight steer still requires native active-run support.

## 6. Provider Contract

The current provider contract returns an `AsyncIterable<ProviderEvent>`. Steer requires a control surface on the active run, not a separate provider call that races the stream.

Final shape:

```ts
export interface InferenceSteer {
  id: string
  sessionId: string
  turnId: string
  content: UserContentBlock[]
}

export interface ProviderRun extends AsyncIterable<ProviderEvent> {
  steer?(input: InferenceSteer): Promise<void>
}

export interface AgentProvider {
  run(request: InferenceRequest): ProviderRun
  dispose?(): Promise<void> | void
}
```

Provider semantics:

- `ProviderRun.steer` means native active-run delivery. It is not a queue hook.
- If `steer` is absent for the active provider run, `AgentSession` must reject active provider-stream steering.
- Providers may still receive historical `user_steer` replay items in later requests.
- Provider transport implementations are responsible for preserving provider-specific pairing data, such as Responses item ids and encrypted reasoning signatures.
- Provider packages must not import `@demi/agent` to implement steer.

Codex provider target:

- Native steering should use the same class of provider-side control channel as Codex `turn/steer`, where verified available for Demi's Codex transport.
- If a transport mode only supports one-way SSE with no active control channel, that mode must not advertise native active-run steer.
- WebSocket fallback logic must not replay partial output or duplicate tool calls when a steer delivery fails.

Claude Code provider target:

- It should remain unsupported for active provider-stream steering unless the CLI transport exposes a true in-flight user input channel with matching semantics.
- It must not emulate steer by interrupting and resuming, because that changes the turn boundary and tool replay semantics.

## 7. AgentSession State Machine

`AgentSession` needs an active turn controller separate from the queued action FIFO.

Conceptual state:

```ts
interface ActiveTurn {
  turnId: string
  phase: 'provider_streaming' | 'tool_executing' | 'compacting' | 'finalizing'
  providerRun: ProviderRun | null
  signal: AbortSignal
}
```

Acceptance rules:

```ts
async function steer(content: UserContentBlock[]): Promise<void> {
  if (!activeTurn) reject('No active turn')
  if (activeTurn.phase === 'compacting' || activeTurn.phase === 'finalizing') reject('Active turn cannot accept steering now')

  const resolved = await resolveReferences(content, activeTurn.signal)
  if (!activeTurn) reject('Turn completed before steer could be accepted')

  const delivery =
    activeTurn.phase === 'tool_executing'
      ? 'next_provider_continuation'
      : activeTurn.providerRun?.steer
        ? 'native_provider_run'
        : reject('Provider does not support active steering')

  const steer = makeSteer(activeTurn.turnId, resolved)
  appendSteerBlock(steer)
  await commitTranscript()

  if (delivery === 'native_provider_run') await activeTurn.providerRun.steer(steer)
}
```

Delivery rule:

- Transcript is the source of truth. Once a steer block is committed, later provider continuations see it through `Transcript.collectInferenceItems()`.
- No separate `pendingSteers` list is needed for replay. If steer is accepted while a tool is executing, the steer block is appended after the existing `tool_call` block. When the tool completes, `collectInferenceItems()` emits that tool's result before the steer because the result is stored on the earlier tool block.
- `ProviderRun.steer` should be implemented as a local active-run enqueue/send operation. If it throws after transcript commit, the session should surface an active turn error and keep the steer in transcript for retry/resume; it should not pretend the steer was never submitted.

Interaction with existing actions:

- `send` while idle starts a new turn.
- `send` while busy remains next-turn queue behavior.
- `steer` while busy targets the current turn and never touches `pendingActions`.
- `retry`, `resume`, and `compact` remain rejected while busy unless they are already documented as queued maintenance behavior. Steer does not change their rules.
- `abort` cancels the active turn and leaves accepted steer blocks as part of the aborted turn history.

## 8. Transport Frames And AgentClient

Steer is not an idle-to-running action and cannot use the existing phase-based action FIFO. Multiple steer attempts can be sent while one turn is active, so they need explicit ack ids.

Frame additions:

```ts
type ClientFrame =
  | { type: 'steer'; steerId: string; content: UserContentBlock[] }
  // existing frames

type ServerFrame =
  | { type: 'steer_result'; steerId: string; status: 'accepted' }
  | { type: 'steer_result'; steerId: string; status: 'rejected'; reason: string }
  // existing frames
```

`AgentClient` additions:

```ts
interface AgentClient {
  steer(content: UserContentBlock[]): Promise<void>
}
```

Client rules:

- `AgentClient.steer` resolves on `steer_result accepted`.
- It rejects on `steer_result rejected`, `error`, or `closed`.
- It must maintain a steer waiter map keyed by `steerId`, not reuse the normal action FIFO.
- `transcript_patch` remains the source of truth for accepted steer visibility.
- `queue` events remain only for next-turn queued inputs.

## 9. UI And REPL Surface

Every surface must make the busy-session choice explicit.

Web UI:

- While idle, the primary submit starts a turn.
- While running, the input surface must expose two distinct commands: steer current turn and queue next turn.
- Queue state continues to render pending next-turn messages.
- Accepted steer appears inline in the transcript as a user steering block attached to the active turn, not inside the queue bar.
- If steering is temporarily unavailable, the steer command is disabled or rejects with a clear reason; it must not silently queue.

REPL:

- Existing queued-input behavior can remain as the queue command.
- Add an explicit steer path, such as a slash command or a running-state keybinding, that calls `AgentClient.steer`.
- The renderer should distinguish queued input from accepted steering input.

Programmatic clients:

- Must choose `send`/queue or `steer`.
- Should handle steer rejection by asking the caller what to do, not by automatic fallback.

## 10. Compaction, Retry, Resume, And Replay

Compaction:

- Base `user` and accepted `steer` blocks sharing a `turnId` form a logical turn group.
- Normal compaction cut points should prefer not to split inside a turn group.
- If split-turn compaction is unavoidable, the summary must preserve every accepted steer and its ordering relative to tool results and assistant output.

Retry:

- Retrying a turn that had accepted steers should rerun the latest logical turn, including the base user input and all accepted steers for that `turnId`.
- Assistant output, tool calls, tool results, and response blocks after the base user block are removed or regenerated according to the existing retry semantics.
- Accepted steer blocks can either be preserved immediately after the base user block or reconstructed as part of the retry preparation, but they must not become separate queued turns.

Resume:

- Resume after abort keeps accepted steers in the aborted turn history.
- A resume continuation should see the steer history through transcript replay and continue from the abort point.

Auto compaction:

- Auto compaction during a turn must not drop accepted steer blocks.
- If active steering arrives while auto compaction is running, reject with "active turn cannot accept steering now" unless the implementation has a proven atomic path for compaction-time insertion.

## 11. Error Handling

Reject steer when:

- no session is open;
- no turn is active;
- the active turn is compacting or finalizing;
- reference resolution fails;
- the active provider run cannot accept in-flight steering;
- the session closes or aborts before steer acceptance.

Do not reject steer merely because there are queued messages. Queue and steer operate on different turn targets.

If a provider accepts steer and then later fails the active run, the accepted steer remains in transcript as part of the failed or aborted turn. The user can retry/resume with that history intact.

## 12. Test Coverage Map

Add or update deterministic tests under `packages/agent/src/__tests__`:

- `session.test.ts`: steer rejects while idle; accepts during active provider run; does not emit queue events; appends `steer` transcript block with the active `turnId`.
- `session.test.ts`: multiple steers preserve order and share the same active `turnId`.
- `session.test.ts`: steer during tool execution is included before the next provider continuation, without draining queued sends early.
- `session.test.ts`: provider-stream steer rejects when `ProviderRun.steer` is absent.
- `session.test.ts`: abort after accepted steer records abort and preserves steer history.
- `compaction.test.ts`: compaction does not split base user and steer blocks except through documented split-turn summary behavior.
- `server.test.ts`: `steer` frame produces `steer_result` ack and transcript patch; multiple acks correlate by `steerId`.
- `server.test.ts`: steer rejection does not create queue entries or transcript blocks.
- `session-marathon.test.ts`: queue and steer interleave correctly across long-running tool continuations.

Add or update surface tests:

- `packages/repl/src/__tests__/renderer.test.ts`: accepted steer renders distinctly from queued input.
- `packages/web-ui/src/agent/__tests__`: running-state input exposes distinct steer and queue commands.

Provider tests:

- `@demi/provider-codex`: fake native control-channel test proves steer delivery does not duplicate output or tool calls.
- `@demi/provider-claude-code`: unsupported active steering is explicit and does not fallback to abort/resume.

Document real-provider acceptance under `docs/repl-acceptance/` once a concrete provider supports native active steering.

## 13. Executable Implementation Checklist

Each item is intended to be implementable as a small checkpoint. Do not enable UI default steering for a provider until the provider-specific native path is verified.

### 13.1 Type And Transcript Groundwork

- [x] Update `packages/provider/src/types.ts`: add `InferenceSteer`, `ProviderRun`, and `InferenceItem` variant `user_steer`.
- [x] Update `AgentProvider.run()` to return `ProviderRun`. Keep `steer` optional so existing async generators remain structurally valid.
- [x] Update `packages/core/src/index.ts`: add `turnId` to `user` and `resume`; add `steer` block with `turnId`, `model`, and `content`.
- [x] Update `packages/agent/src/transcript.ts`: change `pushUserTurn()` and `pushResumeTurn()` to accept `turnId`.
- [x] Add `Transcript.pushSteer(turnId, model, content)`.
- [x] Update `collectInferenceItems()` to emit `user_steer` in transcript order.
- [x] Update `estimateBlockText()` and `renderItemsForSummary()` to include steer content.
- [x] Update transcript tests for user/resume `turnId`, steer block append, replay order, summary rendering, and token estimation.
- [x] Verification: `bun test packages/agent/src/__tests__/transcript.test.ts packages/agent/src/__tests__/context-cache.test.ts`.

### 13.2 ProviderRun Compatibility

- [x] Add a small helper in `@demi/provider` testing utilities for steerable runs, for example `createProviderRun(events, { steer })`.
- [x] Update provider and test annotations that explicitly return `AsyncIterable<ProviderEvent>` only where TypeScript requires it.
- [x] Update `packages/provider/src/__tests__/stub.test.ts` so existing non-steer providers still work unchanged.
- [x] Verification: `bun test packages/provider/src/__tests__/stub.test.ts packages/provider-codex/src/__tests__/provider.test.ts packages/provider-claude-code/src/__tests__/provider.test.ts packages/provider-claude-code/src/__tests__/jsonl-output.test.ts`.

### 13.3 AgentSession Runtime

- [x] Add `AgentSession.steer(content)` outside `pendingActions`.
- [x] Add private active-turn fields: active phase and active provider run. Reuse existing `activeTurnId` and `currentAbortController` instead of introducing a parallel turn identity.
- [x] In `runWorker()`, set send `activeTurnId` from the send action id and pass it to `pushUserTurn()`.
- [x] In `executeRetry()`, set the active turn id to the retried user block's `turnId`, preserve accepted steer blocks for that turn, and rerun with those steers included.
- [x] In `executeResume()`, use the current active turn id for `pushResumeTurn()`.
- [x] In `streamProviderOnce()`, store the returned `ProviderRun` in `activeProviderRun` while streaming and clear it in `finally`.
- [x] In `executePendingTools()`, set internal active phase to `tool_executing` while tool invocations are awaited.
- [x] Reject steer during idle, compaction, finalizing, external mutation reservation, reference-resolution failure, unsupported provider-stream delivery, closed session, or abort race.
- [x] Commit accepted steer through `commitTranscript()` only; never mutate transcript silently.
- [x] Verification: targeted `session.test.ts` cases for idle rejection, provider-stream native steer, unsupported provider-stream rejection, tool-execution steer, queue interleaving, abort preservation, retry preservation, and resume replay.

### 13.4 Transport, Server, And Client

- [x] Update `packages/agent/src/frames.ts`: add `steer` client frame and `steer_result` server/client event.
- [x] Update JSON codec tests if frame shape coverage is explicit.
- [x] Add `AgentClient.steer(content)` with generated `steerId` and a waiter map keyed by id.
- [x] Ensure `closed` and `error` settle all pending steer waiters.
- [x] Update `AgentServer.handleFrame()` with a steer branch that sends `steer_result` instead of using phase FIFO.
- [x] Add server tests for accepted ack, rejected ack, id correlation, no queue event on accepted steer, and no transcript patch on rejected steer.
- [x] Verification: `bun test packages/agent/src/__tests__/server.test.ts packages/agent/src/__tests__/json-codec.test.ts packages/agent/src/__tests__/websocket-transport.test.ts packages/agent/src/__tests__/stdio-transport.test.ts`.

### 13.5 REPL Surface

- [x] Extend `ReplCommandClient` and `ReplLoopClient` with `steer(content)`.
- [x] Add `/steer <text>` to REPL help and `handleCommand()`.
- [x] Keep plain non-command input as `send()` so busy-session plain input continues to queue.
- [x] Render accepted `steer` transcript blocks distinctly from queued input.
- [x] Add renderer and command tests for `/steer`, steer failure, and visible steer block rendering.
- [x] Verification: `bun test packages/repl/src/__tests__/renderer.test.ts packages/repl/src/__tests__/process.test.ts`.

### 13.6 Web UI Surface

- [x] Add `steer()` to `ConversationRuntime` and `AgentWorkspace`.
- [x] Update `ConversationState`/block rendering to display `steer` blocks.
- [x] In `AgentMessageInput`, when running and editor has content, expose distinct actions for "steer current turn" and "queue next turn".
- [x] Keep generic submit behavior as `send()` so idle submit starts a turn and running submit queues the next turn.
- [x] Report steer rejection as a visible error; do not fallback to queue.
- [x] Add web-ui tests for running-state controls and action dispatch.
- [x] Verification: `bun test packages/web-ui/src/agent/__tests__/input-actions.test.ts packages/web-ui/src/agent/__tests__/reasoning.test.ts`.

Current checkpoint: Web running input now exposes explicit steer and queue commands. Generic submit remains `send()` and therefore preserves existing running-state queue behavior.

### 13.7 Provider Implementations

- [ ] `@demi/provider-claude-code`: leave provider-stream steer unsupported unless a true in-flight CLI input channel is proven. Add tests that unsupported provider-stream steer rejects without abort/resume fallback.
- [ ] `@demi/provider-codex`: refactor WebSocket transport into a run object that keeps socket ownership and exposes local `steer()`.
- [ ] `@demi/provider-codex`: do not expose `steer` for SSE runs.
- [ ] `@demi/provider-codex`: in auto mode, expose `steer` only after WebSocket is the active transport; if auto falls back to SSE, active provider-stream steer rejects.
- [ ] `@demi/provider-codex`: verify steer delivery does not duplicate output, replay partial streams, or break tool call/result pairing.
- [ ] Verification: provider unit tests with fake WebSocket, plus a real-provider acceptance document under `docs/repl-acceptance/` before enabling the UI as supported for Codex.

Current checkpoint: `user_steer` replay conversion is covered for Codex Responses and Claude Code JSONL. Native Codex in-flight delivery remains pending and must stay gated until fake WebSocket and real-provider acceptance pass.

### 13.8 Full Gate

- [x] Run `bun run typecheck`.
- [x] Run `bun run test`.
- [ ] Run targeted long-session or real-provider acceptance for queue + steer interleaving once a native provider supports steer.
- [x] Confirm `docs/package-boundaries.md` still matches any new public types or package edges.

## 14. Acceptance Criteria

The feature is complete when:

- callers can choose queue or steer explicitly while a session is running;
- accepted steer affects the current turn without creating a new queued turn;
- rejected steer is explicit and leaves no hidden queue or transcript side effect;
- transcript replay, retry, resume, and compaction preserve accepted steers;
- provider packages implement or reject active steering within their own boundaries;
- deterministic tests cover the runtime, transport, client, and rendering behavior;
- at least one real-provider acceptance path proves native active steering before UI enables it by default for that provider.

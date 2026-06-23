# Agent Steer Plan

| | |
|---|---|
| 日期 | 2026-06-23 |
| 状态 | 方案设计 |
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

## 4. Transcript Model

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

- The base `user` block and every `steer` block in that turn share `turnId`.
- Rejected steer attempts are surfaced as protocol rejections, not transcript blocks.
- Accepted steers are appended in receive order.
- Future transcript replay exposes steer as provider input through a provider-neutral `InferenceItem`, not as a fresh queued turn.

Provider-facing replay item:

```ts
type InferenceItem =
  | { type: 'user_message'; content: UserContentBlock[] }
  | { type: 'user_steer'; turnId: string; content: UserContentBlock[] }
  // existing assistant/tool items
```

Provider adapters decide how to map historical `user_steer` to their protocol. A provider without historical steer metadata can map it to a normal user input item while preserving order, but active in-flight steer still requires native active-run support.

## 5. Provider Contract

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

## 6. AgentSession State Machine

`AgentSession` needs an active turn controller separate from the queued action FIFO.

Conceptual state:

```ts
interface ActiveTurn<State> {
  turnId: string
  phase: 'provider_streaming' | 'tool_executing' | 'compacting' | 'finalizing'
  providerRun: ProviderRun | null
  pendingSteers: InferenceSteer[]
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

  if (delivery === 'next_provider_continuation') activeTurn.pendingSteers.push(steer)
  else await activeTurn.providerRun.steer(steer)
}
```

Provider continuation rule:

- Before building the next provider request inside the same active turn, append all accepted `pendingSteers` to the replayable inference item stream after the tool result that preceded them.
- Clear `pendingSteers` only after the provider continuation request has been built from committed transcript.

Interaction with existing actions:

- `send` while idle starts a new turn.
- `send` while busy remains next-turn queue behavior.
- `steer` while busy targets the current turn and never touches `pendingActions`.
- `retry`, `resume`, and `compact` remain rejected while busy unless they are already documented as queued maintenance behavior. Steer does not change their rules.
- `abort` cancels the active turn and leaves accepted steer blocks as part of the aborted turn history.

## 7. Transport Frames And AgentClient

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

## 8. UI And REPL Surface

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

## 9. Compaction, Retry, Resume, And Replay

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

- Auto compaction during a turn must not drop pending steers.
- If active steering arrives while auto compaction is running, reject with "active turn cannot accept steering now" unless the implementation has a proven atomic path for compaction-time insertion.

## 10. Error Handling

Reject steer when:

- no session is open;
- no turn is active;
- the active turn is compacting or finalizing;
- reference resolution fails;
- the active provider run cannot accept in-flight steering;
- the session closes or aborts before steer acceptance.

Do not reject steer merely because there are queued messages. Queue and steer operate on different turn targets.

If a provider accepts steer and then later fails the active run, the accepted steer remains in transcript as part of the failed or aborted turn. The user can retry/resume with that history intact.

## 11. Test Coverage Map

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

## 12. Implementation Order

This order is for reducing integration risk; each step still targets the final design.

1. Add core transcript and provider inference types for `steer` / `user_steer`.
2. Add `ProviderRun` optional `steer` hook and update existing providers to return a run object or an async iterable compatible with that contract.
3. Add `AgentSession.steer`, active turn bookkeeping, transcript commit semantics, and retry/compaction turn-group handling.
4. Add transport frames and `AgentClient.steer` waiter map.
5. Add server handling and deterministic tests.
6. Add REPL/Web explicit input affordances.
7. Enable native provider implementation only after provider-specific transport behavior is verified.

## 13. Acceptance Criteria

The feature is complete when:

- callers can choose queue or steer explicitly while a session is running;
- accepted steer affects the current turn without creating a new queued turn;
- rejected steer is explicit and leaves no hidden queue or transcript side effect;
- transcript replay, retry, resume, and compaction preserve accepted steers;
- provider packages implement or reject active steering within their own boundaries;
- deterministic tests cover the runtime, transport, client, and rendering behavior;
- at least one real-provider acceptance path proves native active steering before UI enables it by default for that provider.

import { onBeforeUnmount, reactive, shallowRef } from 'vue'
import type { Block, ContextUsage, UserContentBlock } from '@demicodes/protocol'
import { ACTIVITY_HANDOFF_MS } from '@demicodes/web-ui/agent/activity-slot'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { ChatSessionState, ConversationState, PendingSubmissionState } from '@demicodes/web-ui/agent/types'
import { segmentStreamUnits } from '@demicodes/web-ui/ui/stream-reveal'
import { demoModel, editedFile, shellView, type ShellView } from './fixtures/blocks'
import { usageAt } from './fixtures/catalog'
import { printLive } from './live-command'

/**
 * `turn` is a full turn from a sent message, whose delivery the server
 * confirms; `undelivered` is a sent message whose delivery fails, until Retry
 * sends it again; `offline` is a message sent while the backend cannot be
 * reached, which waits and goes once it can; `resume` and `retry` recover an aborted or failed tail;
 * `connect` opens over a dropped socket; `stream` is thinking then reply with
 * nothing waited for; `work` is a turn of many steps, thinking without text
 * and commands in a row, one failing, two changing files. Every fixture only changes conversation state,
 * the way the product's runtime does: the transcript's tail row, its faces and
 * the handoff into a block are `web-ui`'s.
 */
export type TurnFlowKind = 'turn' | 'undelivered' | 'offline' | 'resume' | 'retry' | 'connect' | 'stream' | 'work'

/** A sent message as the composer showed it: what the page holds until the server confirms it. */
export type SentMessage = Pick<PendingSubmissionState, 'text' | 'attachments'>

/** The state `ChatSession` reads, over the full live conversation state the runtime keeps. */
export type TurnFlowState = ConversationState & ChatSessionState

const THINK_1 = 'The cookie name changed from sid to session. The helper already writes the new header. The test is the one still looking for sid.'
const THINK_2 = 'The helper is fine. Update the assertion in auth.test.ts and leave cookie.ts alone.'
const REPLY = 'The cookie helper is fine. The test still expects `sid`.\n\nI updated the assertion in `auth.test.ts` and left `cookie.ts` alone.'
const TOOL_SCRIPT = 'rg -n "sid" packages/web/src/auth.test.ts'
const TOOL_DESCRIPTION = 'Find the old cookie name in the login test'
/** What the turn's command prints, a line at a time while its call runs. */
const TOOL_LINES = [
  'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")',
  'packages/web/src/auth.test.ts:42:    // legacy sid header',
  'packages/web/src/auth.test.ts:57:    expect(readHeader()).not.toContain("sid=")',
]
const USER_TEXT = 'The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.'
const RETRY_ERROR = 'Anthropic API request failed with HTTP 529: Overloaded. The upstream service is temporarily unavailable.'
/** The simulated server's acknowledgement of a recovery or a reconnect. */
const ACK_MS = 800
/** The simulated server admits a sent message: the turn starts before the message is written, as the backend's does. */
const ADMIT_MS = 400
/** The simulated server writes the sent message to the transcript, which confirms its delivery; past a second, so the wait shows its clock first. */
const CONFIRM_MS = 1600
const DELIVERY_ERROR = 'Connection closed before confirmation'
/** How long the simulated backend stays out of reach after an offline send. */
const OFFLINE_MS = 2500
const WAIT_MS = 80
/** Time the model takes before its first output in a turn. */
const FIRST_OUTPUT_MS = 1000
const TOOL_RUN_MS = 1400
/** Time a compaction the user asked for takes. */
const COMPACT_MS = 1200
const FEED_CHARS = 4
const FEED_MS = 90

/** A step of the `work` turn: thinking, without text or with it, or a command. */
type WorkStep =
  | { kind: 'think', text: string }
  | { kind: 'call', description: string, script: string, lines: string[], exitCode?: number, files?: ShellView['files'] }

const WORK_TEXT = 'The wasm mjsunit tests fail on d8. Fix them.'
const WORK_STEPS: WorkStep[] = [
  { kind: 'think', text: '' },
  { kind: 'call', description: 'Check the tests directory contents', script: 'ls test/mjsunit | head -3', lines: ['array-sort.js', 'array-splice.js', 'regress'] },
  { kind: 'think', text: '' },
  { kind: 'call', description: 'Trial-run three mjsunit tests on d8.wasm', script: 'tools/run-tests.sh mjsunit/array-sort', lines: ['tools/run-tests.sh: Permission denied'], exitCode: 126 },
  { kind: 'think', text: 'The runner script has no execute bit. Set it, then point the status file at the new suite paths.' },
  {
    kind: 'call',
    description: 'Make the runner script executable',
    script: 'chmod +x tools/run-tests.sh && demi file edit tools/run-tests.sh',
    lines: ['edited tools/run-tests.sh'],
    files: [editedFile({ path: 'tools/run-tests.sh', kind: 'modified', added: 1, removed: 1 })],
  },
  {
    kind: 'call',
    description: 'Patch the test status file',
    script: 'demi file edit test/mjsunit/mjsunit.status',
    lines: ['edited test/mjsunit/mjsunit.status'],
    files: [editedFile({ path: 'test/mjsunit/mjsunit.status', kind: 'modified', added: 12, removed: 3 })],
  },
  { kind: 'think', text: '' },
  { kind: 'call', description: 'Run the full mjsunit suite', script: 'tools/run-tests.sh mjsunit', lines: ['[00:04|%  25|+ 103|-   0]', '[00:09|%  60|+ 247|-   0]', '[00:15|% 100|+ 412|-   0]: Done'] },
  { kind: 'think', text: '' },
]
const WORK_REPLY = 'The runner script lacked its execute bit and the status file still listed the old suite paths. All 412 mjsunit tests pass now.'

export interface TurnFlowOptions {
  id?: string
  title?: string
  /** The directory the conversation works in; its messages' relative paths resolve against it. */
  cwd?: string
  blocks?: Block[]
  subagents?: SubagentRecord[]
  terminals?: TerminalRecord[]
  /** How full the context starts; the composer's meter and Compact read it. */
  contextUsage?: ContextUsage
}

export function useTurnFlow(options: TurnFlowOptions = {}) {
  const state: TurnFlowState = reactive({
    id: options.id ?? 'turn-flow',
    cwd: options.cwd ?? '/',
    title: options.title ?? 'Login test',
    blocks: options.blocks ?? [],
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    pendingCalls: [],
    model: {
      providerId: demoModel.providerId,
      modelId: demoModel.model.id,
      thinkingEffort: null,
      serviceTierId: null,
    },
    lastError: null,
    load: 'ready',
    pendingAction: null,
    failures: {},
    contextUsage: options.contextUsage ?? usageAt(0.62),
    archived: false,
    scroll: null,
    subagents: options.subagents ?? [],
    terminals: options.terminals ?? [],
  })
  const timers: number[] = []
  /** The message sent and not yet confirmed, as `ChatSession` shows it. */
  const pendingSubmission = shallowRef<PendingSubmissionState | null>(null)
  /** What the pending message sends, which its user block holds once confirmed. */
  let pendingContent: UserContentBlock[] = []
  let token = 0
  let sequence = 0
  /** The call of the turn that runs now, whose command a stop ends. */
  let runningTool: string | null = null

  function cancel(): void {
    token += 1
    for (const id of timers) {
      window.clearTimeout(id)
    }
    timers.length = 0
    // A stopped action stops the command its running call started.
    if (runningTool) {
      endTerminal(command(runningTool), 'aborted')
      runningTool = null
    }
  }

  function at(run: number, ms: number, fn: () => void): void {
    timers.push(window.setTimeout(() => {
      if (run !== token) {
        return
      }
      fn()
    }, ms))
  }

  function now(): string {
    return new Date().toISOString()
  }

  /** An id no fixture uses: the flow's own id comes first, so `user-1` of a seeded transcript stays unique. */
  function nextId(kind: string): string {
    sequence += 1
    return `${state.id}-${kind}-${sequence}`
  }

  function replace(id: string, next: Block): void {
    state.blocks = state.blocks.map((block) => (block.id === id ? next : block))
  }

  function append(block: Block): void {
    state.blocks = [...state.blocks, block]
  }

  function userBlock(content: UserContentBlock[]): Block {
    return {
      type: 'user',
      id: nextId('user'),
      turnId: nextId('turn'),
      createdAt: now(),
      model: demoModel,
      content,
      preamble: null,
    }
  }

  function thinkingBlock(id: string, createdAt: string, text: string): Block {
    return {
      type: 'thinking',
      id,
      createdAt,
      model: demoModel,
      text,
      signature: null,
    }
  }

  function textBlock(id: string, createdAt: string, text: string): Block {
    return {
      type: 'text',
      id,
      createdAt,
      model: demoModel,
      text,
    }
  }

  function tool(id: string, createdAt: string, status: ToolCallBlock['status']): ToolCallBlock {
    const output = status === 'completed'
      ? [{ type: 'text' as const, text: `${TOOL_LINES.join('\n')}\n` }]
      : []
    return {
      type: 'tool_call',
      id,
      createdAt,
      model: demoModel,
      toolUseId: `${id}-use`,
      toolName: 'shell_exec',
      status,
      input: JSON.stringify({
        script: TOOL_SCRIPT,
        description: TOOL_DESCRIPTION,
      }),
        output,
      view: shellView({
        commandId: `cmd-${id}`,
        status: status === 'executing' ? 'running' : 'exited',
        chunks: [{ stream: 'stdout', text: output[0]?.text ?? '' }],
      }),
    }
  }

  function feedPrefixes(full: string): string[] {
    const units = segmentStreamUnits(full)
    const prefixes: string[] = []
    let acc = ''
    let chunk = ''
    for (const unit of units) {
      chunk += unit
      acc += unit
      if (chunk.length >= FEED_CHARS) {
        prefixes.push(acc)
        chunk = ''
      }
    }
    if (chunk || prefixes.length === 0) {
      prefixes.push(acc)
    }
    return prefixes
  }

  /** Streams `full` into `apply` from `startMs`; returns when the last prefix lands. */
  function streamTextInto(
    run: number,
    startMs: number,
    full: string,
    apply: (text: string) => void,
  ): number {
    const prefixes = feedPrefixes(full)
    prefixes.forEach((text, index) => {
      at(run, startMs + index * FEED_MS, () => apply(text))
    })
    return startMs + Math.max(0, prefixes.length - 1) * FEED_MS
  }

  /** An empty thinking block arrives at `startMs`; its text streams after the handoff. Returns when the text is complete. */
  function think(run: number, startMs: number, text: string): number {
    const id = nextId('think')
    let createdAt = ''
    at(run, startMs, () => {
      createdAt = now()
      append(thinkingBlock(id, createdAt, ''))
    })
    return streamTextInto(run, startMs + ACTIVITY_HANDOFF_MS, text, (partial) => {
      replace(id, thinkingBlock(id, createdAt, partial))
    })
  }

  /** The reply streams from `startMs`; the turn ends after it. */
  function reply(run: number, startMs: number, text: string): void {
    const id = nextId('text')
    let createdAt = ''
    at(run, startMs, () => {
      createdAt = now()
      append(textBlock(id, createdAt, ''))
    })
    const end = streamTextInto(run, startMs + FEED_MS, text, (partial) => {
      replace(id, textBlock(id, createdAt, partial))
    })
    at(run, end + 160, () => {
      state.phase = 'idle'
    })
  }

  function thinkThenReply(run: number, startMs: number, text: string): void {
    const thought = think(run, startMs, text)
    reply(run, thought + 240, REPLY)
  }

  /** The command the turn's call runs, whose output shows under the call as it comes, as the product's live frames bring it. */
  function command(toolId: string): TerminalRecord | undefined {
    return state.terminals.find((terminal) => terminal.id === `cmd-${toolId}`)
  }

  /** Ends a running command as the product's last frame of it would: `exited` on its own, `aborted` when stopped. */
  function endTerminal(terminal: TerminalRecord | undefined, phase: 'exited' | 'aborted'): void {
    if (terminal?.phase === 'running') {
      terminal.phase = phase
      terminal.endedAt = now()
    }
  }

  /** A whole turn on the current transcript: request, think, run a tool, think, reply. */
  function runTurn(run: number): void {
    state.phase = 'running'
    const thought1 = think(run, FIRST_OUTPUT_MS, THINK_1)
    const toolId = nextId('tool')
    let toolStartedAt = ''
    at(run, thought1 + 200, () => {
      toolStartedAt = now()
      append(tool(toolId, toolStartedAt, 'executing'))
      runningTool = toolId
      state.terminals.push({
        id: `cmd-${toolId}`,
        title: TOOL_DESCRIPTION,
        script: TOOL_SCRIPT,
        phase: 'running',
        startedAt: toolStartedAt,
        output: '',
        chars: 0,
        toolUseId: `${toolId}-use`,
      })
    })
    TOOL_LINES.forEach((line, index) => {
      at(run, thought1 + 200 + (index + 1) * 300, () => {
        const terminal = command(toolId)
        if (terminal) {
          printLive(terminal, `${line}\n`)
        }
      })
    })
    const toolDone = thought1 + 200 + TOOL_RUN_MS
    at(run, toolDone, () => {
      endTerminal(command(toolId), 'exited')
      runningTool = null
      replace(toolId, tool(toolId, toolStartedAt, 'completed'))
    })
    thinkThenReply(run, toolDone + WAIT_MS, THINK_2)
  }

  function workCall(id: string, createdAt: string, step: Extract<WorkStep, { kind: 'call' }>, status: ToolCallBlock['status']): ToolCallBlock {
    const text = `${step.lines.join('\n')}\n`
    return {
      type: 'tool_call',
      id,
      createdAt,
      model: demoModel,
      toolUseId: `${id}-use`,
      toolName: 'shell_exec',
      status,
      input: JSON.stringify({ script: step.script, description: step.description }),
      output: status === 'completed' ? [{ type: 'text', text }] : [],
      view: status === 'executing'
        ? null
        : shellView({ commandId: `cmd-${id}`, chunks: [{ stream: 'stdout', text }], exitCode: step.exitCode, files: step.files }),
    }
  }

  /**
   * The `work` turn: each step as the product's runtime brings it. Thinking
   * arrives empty and streams its text, a command's call runs with its
   * output coming live and ends with its result, and the reply streams last.
   */
  function runWork(run: number): void {
    state.phase = 'running'
    state.pendingCalls = []
    let t = FIRST_OUTPUT_MS
    for (const step of WORK_STEPS) {
      if (step.kind === 'think') {
        t = think(run, t, step.text) + (step.text ? 300 : 700)
        continue
      }
      const id = nextId('tool')
      let startedAt = ''
      // The model writes the call first: its row shows as it opens, takes its
      // description once written, and becomes the call's block when whole.
      at(run, t, () => {
        state.pendingCalls = [{ toolUseId: `${id}-use`, toolName: 'shell_exec', description: null }]
      })
      at(run, t + 400, () => {
        state.pendingCalls = [{ toolUseId: `${id}-use`, toolName: 'shell_exec', description: step.description }]
      })
      t += 900
      at(run, t, () => {
        startedAt = now()
        state.pendingCalls = []
        append(workCall(id, startedAt, step, 'executing'))
        state.terminals.push({
          id: `cmd-${id}`,
          title: step.description,
          script: step.script,
          phase: 'running',
          startedAt,
          output: '',
          chars: 0,
          toolUseId: `${id}-use`,
        })
      })
      step.lines.forEach((line, index) => {
        at(run, t + (index + 1) * 300, () => {
          const terminal = command(id)
          if (terminal)
            printLive(terminal, `${line}\n`)
        })
      })
      const done = t + TOOL_RUN_MS
      at(run, done, () => {
        const terminal = command(id)
        if (terminal && step.exitCode !== undefined)
          terminal.exitCode = step.exitCode
        endTerminal(terminal, 'exited')
        replace(id, workCall(id, startedAt, step, 'completed'))
      })
      t = done + WAIT_MS
    }
    reply(run, t, WORK_REPLY)
  }

  /** The `work` turn as it stands once it ended, as a page opened later shows it. */
  function settleWork(): void {
    cancel()
    pendingSubmission.value = null
    state.pendingCalls = []
    state.phase = 'idle'
    const at = now()
    state.blocks = [
      userBlock([{ type: 'text', text: WORK_TEXT }]),
      ...WORK_STEPS.map((step): Block => step.kind === 'think'
        ? thinkingBlock(nextId('think'), at, step.text)
        : workCall(nextId('tool'), at, step, 'completed')),
      textBlock(nextId('text'), at, WORK_REPLY),
    ]
  }

  /** The server acknowledges a recovery: the recovered record settles, the turn runs. */
  function acknowledgeRecovery(): void {
    const tail = state.blocks.at(-1)
    if (tail?.type === 'abort') {
      replace(tail.id, { ...tail, isResumed: true })
    } else if (tail?.type === 'error') {
      state.blocks = state.blocks.slice(0, -1)
    }
    state.pendingAction = null
    state.phase = 'running'
  }

  /** Send a message on the current transcript; its turn runs once the server confirms it. */
  function turn(content: UserContentBlock[], message: SentMessage): void {
    cancel()
    holdMessage(content, message, null)
    deliver(token, false)
  }

  /**
   * The page holds a sent message until the server confirms it, with why its
   * delivery failed, or waiting while the backend cannot be reached.
   */
  function holdMessage(content: UserContentBlock[], message: SentMessage, error: string | null, waiting = false): void {
    pendingSubmission.value = { id: nextId('message'), ...message, error, waiting }
    pendingContent = content
  }

  /**
   * The simulated server answers the pending message: it admits the turn, then
   * writes the message, which confirms it. A delivery that fails is answered
   * with the failure alone.
   */
  function deliver(run: number, fails: boolean, body: (run: number) => void = runTurn): void {
    if (fails) {
      at(run, CONFIRM_MS, () => {
        undeliver(DELIVERY_ERROR)
      })
      return
    }
    at(run, ADMIT_MS, () => {
      state.phase = 'running'
    })
    at(run, CONFIRM_MS, () => {
      confirmMessage()
      body(run)
    })
  }

  /** The pending message's user block is written: the message is no longer pending. */
  function confirmMessage(): void {
    if (pendingSubmission.value?.error !== null) {
      return
    }
    pendingSubmission.value = null
    append(userBlock(pendingContent))
  }

  function undeliver(error: string): void {
    const pending = pendingSubmission.value
    if (pending) {
      pendingSubmission.value = { ...pending, error }
    }
  }

  /** A new conversation whose first message was not delivered, as a page shows it after the failure. */
  function undelivered(content: UserContentBlock[], message: SentMessage, error: string): void {
    cancel()
    state.phase = 'idle'
    state.blocks = []
    holdMessage(content, message, error)
  }

  /** Retry sends the undelivered message again with its id, as the product's does; this time it arrives. */
  function retrySubmission(): void {
    const pending = pendingSubmission.value
    if (!pending?.error) {
      return
    }
    cancel()
    pendingSubmission.value = { ...pending, error: null }
    deliver(token, false)
  }

  /** Resume or retry from the current tail, the way Resume and Retry do in the product. */
  function resume(): void {
    cancel()
    const run = token
    state.lastError = null
    state.pendingAction = 'resume'
    at(run, ACK_MS, acknowledgeRecovery)
    thinkThenReply(run, ACK_MS + WAIT_MS, THINK_2)
  }

  /**
   * Abort a running turn: the transcript ends with an abort record to resume
   * from. A compaction stops with nothing to resume.
   */
  function stop(): void {
    if (state.phase === 'idle') {
      return
    }
    cancel()
    // An admitted message belongs to the turn: the stop writes it first.
    confirmMessage()
    const stopped = state.phase
    state.phase = 'idle'
    if (stopped === 'compacting') {
      return
    }
    append({
      type: 'abort',
      id: nextId('abort'),
      createdAt: now(),
      model: demoModel,
      isResumed: false,
    })
  }

  /**
   * Compact from the context meter's card, as the backend does: the summary
   * goes in before the latest message, the marker after the last block, where
   * the divider then shows, and the context is small again.
   */
  function compact(): void {
    if (state.phase !== 'idle') {
      return
    }
    const run = token
    state.phase = 'compacting'
    at(run, COMPACT_MS, () => {
      const boundaryId = nextId('boundary')
      const cut = Math.max(0, state.blocks.findLastIndex((block) => block.type === 'user'))
      state.blocks = [
        ...state.blocks.slice(0, cut),
        {
          type: 'compaction_boundary',
          id: boundaryId,
          createdAt: now(),
          model: demoModel,
          summary: 'The login test expects the renamed session cookie.',
          summaryTokens: 2400,
        },
        ...state.blocks.slice(cut),
        {
          type: 'compaction_marker',
          id: nextId('marker'),
          createdAt: now(),
          model: demoModel,
          boundaryId,
          compactedTokens: state.contextUsage?.tokens ?? 0,
        },
      ]
      state.contextUsage = usageAt(0.06)
      state.phase = 'idle'
    })
  }

  /** A message sent while a turn runs waits in the queue, as the product's does. */
  function queue(content: UserContentBlock[]): void {
    state.queue = [...state.queue, { id: nextId('queued'), content }]
  }

  function removeQueued(id: string): void {
    state.queue = state.queue.filter((entry) => entry.id !== id)
  }

  /** Send now: the queued message becomes a steer the running turn takes at its next step. */
  function sendQueued(id: string): void {
    const item = state.queue.find((entry) => entry.id === id)
    if (!item) {
      return
    }
    removeQueued(id)
    state.pendingSteers = [...state.pendingSteers, { id: nextId('pending'), content: item.content }]
  }

  function removePendingSteer(id: string): void {
    state.pendingSteers = state.pendingSteers.filter((entry) => entry.id !== id)
  }

  /**
   * As the product delivers a pending steer now: Stop writes the steer into the
   * stopped turn, before its marker, and Continue goes on with it from there.
   */
  function interruptPendingSteer(id: string): void {
    const pending = state.pendingSteers.find((entry) => entry.id === id)
    if (!pending) {
      return
    }
    removePendingSteer(id)
    append({
      type: 'steer',
      id: nextId('steer'),
      turnId: nextId('turn'),
      createdAt: now(),
      model: demoModel,
      content: pending.content,
    })
    stop()
    resume()
  }

  function abortSubagent(id: string): void {
    const agent = state.subagents.find((entry) => entry.id === id)
    if (agent?.phase === 'running') {
      agent.phase = 'aborted'
      agent.endedAt = now()
    }
  }

  function abortSubagents(): void {
    for (const agent of state.subagents) {
      abortSubagent(agent.id)
    }
  }

  function abortTerminal(id: string): void {
    endTerminal(state.terminals.find((entry) => entry.id === id), 'aborted')
  }

  /** Reset to a fixture and play it. */
  function play(kind: TurnFlowKind = 'turn'): void {
    cancel()
    const run = token
    state.lastError = null
    state.pendingAction = null
    state.load = 'ready'
    state.phase = 'idle'
    pendingSubmission.value = null

    if (kind === 'stream') {
      // Thinking is the tail from the first running frame, so nothing is waited for.
      const id = nextId('think')
      const createdAt = now()
      state.blocks = [thinkingBlock(id, createdAt, '')]
      state.phase = 'running'
      const thought = streamTextInto(run, FEED_MS, THINK_1, (partial) => {
        replace(id, thinkingBlock(id, createdAt, partial))
      })
      reply(run, thought + 240, REPLY)
      return
    }

    if (kind === 'connect') {
      // The socket dropped while a turn was running: Connecting wins until it is back.
      state.blocks = [userBlock([{ type: 'text', text: USER_TEXT }])]
      state.phase = 'running'
      state.load = 'reconnecting'
      at(run, ACK_MS, () => {
        state.load = 'ready'
      })
      thinkThenReply(run, ACK_MS + WAIT_MS, THINK_2)
      return
    }

    if (kind === 'resume') {
      state.blocks = [
        userBlock([{ type: 'text', text: USER_TEXT }]),
        {
          type: 'abort',
          id: nextId('abort'),
          createdAt: now(),
          model: demoModel,
          isResumed: false,
        },
      ]
      resume()
      return
    }

    if (kind === 'retry') {
      state.blocks = [
        userBlock([{ type: 'text', text: USER_TEXT }]),
        {
          type: 'error',
          id: nextId('error'),
          createdAt: now(),
          model: demoModel,
          message: RETRY_ERROR,
          code: 'overloaded',
          diagnostics: {
            source: 'http',
            httpStatus: 529,
            providerCode: 'overloaded_error',
          },
        },
      ]
      state.lastError = RETRY_ERROR
      resume()
      return
    }

    state.blocks = []
    if (kind === 'offline') {
      // The backend is out of reach: the message waits, then goes with its id once it is back.
      holdMessage([{ type: 'text', text: USER_TEXT }], { text: USER_TEXT, attachments: [] }, null, true)
      at(run, OFFLINE_MS, () => {
        const pending = pendingSubmission.value
        if (pending) {
          pendingSubmission.value = { ...pending, waiting: false }
        }
        deliver(run, false)
      })
      return
    }
    if (kind === 'work') {
      holdMessage([{ type: 'text', text: WORK_TEXT }], { text: WORK_TEXT, attachments: [] }, null)
      deliver(run, false, runWork)
      return
    }
    holdMessage([{ type: 'text', text: USER_TEXT }], { text: USER_TEXT, attachments: [] }, null)
    deliver(run, kind === 'undelivered')
  }

  onBeforeUnmount(cancel)

  return {
    state,
    pendingSubmission,
    play,
    settleWork,
    turn,
    retrySubmission,
    undelivered,
    resume,
    stop,
    compact,
    queue,
    removeQueued,
    sendQueued,
    removePendingSteer,
    interruptPendingSteer,
    abortSubagent,
    abortSubagents,
    abortTerminal,
  }
}

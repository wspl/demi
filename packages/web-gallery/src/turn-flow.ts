import { computed, onBeforeUnmount, reactive, shallowRef, watch } from 'vue'
import type { AgentMessage, Block, ContextUsage, UserContentBlock } from '@demicodes/protocol'
import { ACTIVITY_HANDOFF_MS } from '@demicodes/web-ui/agent/activity-slot'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { ChatSessionState, ConversationState, PendingSubmissionState } from '@demicodes/web-ui/agent/types'
import type { SendWay } from '@demicodes/web-ui/agent/send-way'
import { segmentStreamUnits } from '@demicodes/web-ui/ui/stream-reveal'
import { demoModel, editedFile, shellView, type ShellView } from './fixtures/blocks'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import { usageAt } from './fixtures/catalog'
import { suitePathsUpdate } from './fixtures/subagents'
import { printLive } from './live-command'

/**
 * `turn` is a full turn from a sent message, whose delivery the server
 * confirms; `undelivered` is a sent message whose delivery fails, until Retry
 * sends it again; `offline` is a message sent while the socket is lost,
 * under Connecting, which goes once the socket is back; `resume` and `retry` recover an aborted or failed tail;
 * `connect` opens over a dropped socket; `stream` is thinking then reply with
 * nothing waited for; `work` is a turn of many steps, thinking without text
 * and commands in a row, one failing, two changing files. Every fixture only changes conversation state,
 * the way the product's runtime does: the transcript's tail row, its faces and
 * the handoff into a block are `web-ui`'s.
 */
export type TurnFlowKind = 'turn' | 'undelivered' | 'offline' | 'resume' | 'retry' | 'connect' | 'stream' | 'work' | 'steer' | 'queue'

/**
 * A sent message as the composer showed it: what the page holds until the
 * server confirms it. `ready` is its content once every file of it has
 * uploaded, for a message sent while one still uploads; it rejects when an
 * upload fails, which leaves the message undelivered.
 */
export type SentMessage = Pick<PendingSubmissionState, 'text' | 'attachments'> & {
  ready?: Promise<UserContentBlock[]>
}

/** The state `ChatSession` reads, over the full live conversation state the runtime keeps. */
/** The gallery's conversation: its whole transcript as `blocks`, which its history holds as one window. */
export type TurnFlowState = ConversationState & ChatSessionState & { blocks: Block[] }

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
const STEER_TEXT = 'Also check the sign-up test.'
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
/**
 * How long the model takes to answer a step's result in the work replay: long
 * enough to see the run's row go on shimmering while it is waited for.
 */
const STEP_ANSWER_MS = 1500
/** Time the model takes before its first output in a turn. */
const FIRST_OUTPUT_MS = 1000
const TOOL_RUN_MS = 1400
/** Time a compaction the user asked for takes. */
const COMPACT_MS = 1200
const FEED_CHARS = 4
const FEED_MS = 90

/**
 * A step of the `work` turn: thinking, without text or with it, a command,
 * which a `background` one leaves running when its call returns, the
 * notice of a background command's end, or an agent's update.
 */
type WorkStep =
  | { kind: 'think', text: string }
  | { kind: 'call', description: string, script: string, lines: string[], exitCode?: number, files?: ShellView['files'], background?: true }
  | { kind: 'notice', of: string }
  | { kind: 'receipt', message: AgentMessage }

const WORK_TEXT = 'The wasm mjsunit tests fail on d8. Fix them.'
const WORK_STEPS: WorkStep[] = [
  { kind: 'think', text: '' },
  { kind: 'call', description: 'Check the tests directory contents', script: 'ls test/mjsunit | head -3', lines: ['array-sort.js', 'array-splice.js', 'regress'] },
  { kind: 'call', description: 'Build d8 for wasm', script: 'tools/build.sh d8 --target=wasm', lines: ['[1/1268] Compiling src/api/api.cc'], background: true },
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
  { kind: 'notice', of: 'Build d8 for wasm' },
  { kind: 'receipt', message: suitePathsUpdate },
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
    get history() {
      return wholeHistory(this.blocks)
    },
    shownAt: null,
    instructions: [],
    summaries: {},
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    pendingCalls: [],
    waitingSince: null,
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
  /** The messages sent and not yet confirmed, in the order they were sent, as `ChatSession` shows them. */
  const pendingSubmissions = shallowRef<PendingSubmissionState[]>([])
  /** What each pending message sends, by its id, which its block holds once confirmed. */
  const contents = new Map<string, UserContentBlock[]>()
  /** The waiting messages whose Remove is on its way, by id, which the list shows nowhere. */
  const removing = shallowRef<string[]>([])
  /**
   * The message that starts a turn is on its way: the turn is as good as
   * running, so the composer shows Stop, and a message sent meanwhile
   * steers it or waits in the queue, as the product's does.
   */
  const startingTurn = computed(() =>
    state.phase === 'idle' && pendingSubmissions.value.some((pending) => pending.error === null))
  /**
   * The simulated server's answers to the steers and queued messages on
   * their way, by id, which a turn cut short or stopped does not cancel, as
   * a session holds what it was sent whatever its turn does.
   */
  const deliveries = new Map<string, number>()
  /** When the last message on its way is delivered, as `Date.now()`: the next one goes after it, never ahead. */
  let deliveredBy = 0
  /** Remove or Send Now pressed on a steer or queued message on its way, by id, carried out once it is held. */
  const wishes = new Map<string, 'remove' | 'sendNow'>()
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

  /**
   * The model is asked, as the server would tell a page when it begins to
   * wait (`runtime.md` § Waiting for the model); the model's first block or
   * call being written ends the wait, and so does the turn's end.
   */
  function waitForModel(): void {
    state.waitingSince = Date.now()
  }

  watch(() => state.phase, (phase) => {
    if (phase !== 'running') {
      state.waitingSince = null
    }
  })

  function append(block: Block): void {
    if (block.type === 'thinking' || block.type === 'text' || block.type === 'tool_call') {
      state.waitingSince = null
    }
    state.blocks = [...state.blocks, block]
  }

  /** A message's block; `turnId` is the id the message was sent with, which the server keeps as its turn's. */
  function userBlock(content: UserContentBlock[], turnId = nextId('turn')): Block {
    return {
      type: 'user',
      id: nextId('user'),
      turnId,
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
      toolName: 'shell',
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
      // A steer that came during the reply is read before the turn ends.
      if (writeSteers()) {
        waitForModel()
        thinkThenReply(run, WAIT_MS, THINK_2)
        return
      }
      state.phase = 'idle'
    })
  }

  /**
   * The continuation boundary of the running turn, where a call returned or a
   * reply ended: the pending steers become `steer` blocks the model reads
   * next, as the product's runtime writes them. Returns whether there were any.
   */
  function writeSteers(): boolean {
    const steers = state.pendingSteers
    if (steers.length === 0) {
      return false
    }
    state.pendingSteers = []
    for (const pending of steers) {
      append({
        type: 'steer',
        id: pending.id,
        turnId: nextId('turn'),
        createdAt: now(),
        model: demoModel,
        content: pending.content,
      })
    }
    return true
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
    waitForModel()
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
      writeSteers()
      waitForModel()
    })
    thinkThenReply(run, toolDone + WAIT_MS, THINK_2)
  }

  /** A call of the `work` turn; a background one returned while its command still ran. */
  function workCall(id: string, createdAt: string, step: Extract<WorkStep, { kind: 'call' }>, status: ToolCallBlock['status']): ToolCallBlock {
    const text = `${step.lines.join('\n')}\n`
    return {
      type: 'tool_call',
      id,
      createdAt,
      model: demoModel,
      toolUseId: `${id}-use`,
      toolName: 'shell',
      status,
      input: JSON.stringify({ script: step.script, description: step.description }),
      output: status === 'completed' ? [{ type: 'text', text }] : [],
      view: status === 'executing'
        ? null
        : shellView({
            commandId: `cmd-${id}`,
            chunks: [{ stream: 'stdout', text }],
            exitCode: step.exitCode,
            files: step.files,
            ...(step.background ? { status: 'running' } : {}),
          }),
    }
  }

  /** The notice of the end of the background command `id` started, titled `title`, which the call that started it names. */
  function workNotice(id: string, createdAt: string, commandId: string, title: string): Block {
    return {
      type: 'wakeup',
      id,
      turnId: `${id}-turn`,
      createdAt,
      model: demoModel,
      placement: 'steer',
      reports: [{ commandId, title, event: { kind: 'ended', exitCode: 0 }, output: '' }],
    }
  }

  /** An agent's update as its receipt in the turn. */
  function workReceipt(createdAt: string, message: AgentMessage): Block {
    return { type: 'agent_message', id: message.id, turnId: `${message.id}-turn`, createdAt, model: demoModel, message }
  }

  /** The command of the `work` turn's call titled `description`: the one a notice reports the end of. */
  function workCommand(description: string): TerminalRecord | undefined {
    return state.terminals.find((terminal) => terminal.title === description)
  }

  /**
   * The `work` turn: each step as the product's runtime brings it. Thinking
   * arrives empty and streams its text, a command's call runs with its
   * output coming live and ends with its result, and the reply streams last.
   */
  function runWork(run: number): void {
    state.phase = 'running'
    state.pendingCalls = []
    waitForModel()
    let t = FIRST_OUTPUT_MS
    for (const step of WORK_STEPS) {
      if (step.kind === 'think') {
        t = think(run, t, step.text) + (step.text ? 300 : 700)
        continue
      }
      // A notice and a receipt enter at the agent's next step, and the model is asked again.
      if (step.kind === 'notice') {
        at(run, t, () => {
          const terminal = workCommand(step.of)
          if (!terminal)
            return
          printLive(terminal, '[1268/1268] Linking d8\n')
          endTerminal(terminal, 'exited')
          terminal.exitCode = 0
          append(workNotice(nextId('notice'), now(), terminal.id, step.of))
          waitForModel()
        })
        t += STEP_ANSWER_MS
        continue
      }
      if (step.kind === 'receipt') {
        at(run, t, () => {
          append(workReceipt(now(), step.message))
          waitForModel()
        })
        t += STEP_ANSWER_MS
        continue
      }
      const id = nextId('tool')
      let startedAt = ''
      // The model writes the call first: its row shows as it opens, takes its
      // description once written, and becomes the call's block when whole.
      at(run, t, () => {
        state.waitingSince = null
        state.pendingCalls = [{ toolUseId: `${id}-use`, toolName: 'shell', description: null }]
      })
      at(run, t + 400, () => {
        state.pendingCalls = [{ toolUseId: `${id}-use`, toolName: 'shell', description: step.description }]
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
        // A background command runs on after its call returned, until its notice.
        if (!step.background)
          endTerminal(terminal, 'exited')
        replace(id, workCall(id, startedAt, step, 'completed'))
        writeSteers()
        waitForModel()
      })
      t = done + STEP_ANSWER_MS
    }
    reply(run, t, WORK_REPLY)
  }

  /** The `work` turn as it stands once it ended, as a page opened later shows it. */
  function settleWork(): void {
    cancel()
    forgetSubmissions()
    state.pendingCalls = []
    state.phase = 'idle'
    const at = now()
    const calls = new Map<string, string>()
    state.blocks = [
      userBlock([{ type: 'text', text: WORK_TEXT }]),
      ...WORK_STEPS.map((step): Block => {
        switch (step.kind) {
          case 'think':
            return thinkingBlock(nextId('think'), at, step.text)
          case 'notice':
            return workNotice(nextId('notice'), at, calls.get(step.of) ?? '', step.of)
          case 'receipt':
            return workReceipt(at, step.message)
          case 'call': {
            const id = nextId('tool')
            calls.set(step.description, `cmd-${id}`)
            return workCall(id, at, step, 'completed')
          }
        }
      }),
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
    const id = holdMessage(content, message, null)
    afterUploads(id, message.ready, () => deliver(token, false))
  }

  /**
   * Goes on with the message `id` once its files have uploaded, as the
   * product's delivery waits for them, with the content `ready` answers; an
   * upload that fails leaves the message undelivered.
   */
  function afterUploads(id: string, ready: Promise<UserContentBlock[]> | undefined, go: () => void): void {
    if (!ready) {
      go()
      return
    }
    const run = token
    const current = () => run === token && pendingSubmissions.value.some((pending) => pending.id === id && pending.error === null)
    ready.then((content) => {
      if (current()) {
        contents.set(id, content)
        go()
      }
    }, (error: unknown) => {
      if (current()) {
        changeSubmission(id, { error: error instanceof Error ? error.message : String(error) })
      }
    })
  }

  /**
   * The page holds a sent message until the server confirms it, with why its
   * delivery failed, as what it will be: the next message, a steer or a
   * queued message. Answers its id.
   */
  function holdMessage(content: UserContentBlock[], message: SentMessage, error: string | null, way: SendWay = 'send'): string {
    const id = nextId('message')
    pendingSubmissions.value = [...pendingSubmissions.value, { id, way, text: message.text, attachments: message.attachments, error }]
    contents.set(id, content)
    return id
  }

  function changeSubmission(id: string, change: Partial<PendingSubmissionState>): void {
    pendingSubmissions.value = pendingSubmissions.value.map((pending) => pending.id === id ? { ...pending, ...change } : pending)
  }

  /** The message `id` is no longer on its way; answers what it sends. */
  function dropSubmission(id: string): UserContentBlock[] {
    pendingSubmissions.value = pendingSubmissions.value.filter((pending) => pending.id !== id)
    const content = contents.get(id) ?? []
    contents.delete(id)
    return content
  }

  /** Lets go of every message on its way and what waits for one, as a fixture starting over does. */
  function forgetSubmissions(): void {
    for (const timer of deliveries.values()) {
      window.clearTimeout(timer)
    }
    deliveries.clear()
    wishes.clear()
    contents.clear()
    pendingSubmissions.value = []
    removing.value = []
    deliveredBy = 0
  }

  /** The simulated server answers a message on its way once it has answered the one before, as the product delivers them. */
  function nextDelivery(): number {
    deliveredBy = Math.max(Date.now(), deliveredBy) + CONFIRM_MS
    return deliveredBy - Date.now()
  }

  /**
   * A steer or a queued message on its way, shown as what it will be from the
   * send: once the simulated server has answered the messages before it,
   * it holds this one, a steer in the running turn's steers and a queued
   * message in the queue, under the id it was sent with; with nothing
   * running, it is refused and goes as a message, which starts a turn, as
   * the product sends it. Remove or Send Now pressed meanwhile is carried
   * out then.
   */
  function deliverWaiting(content: UserContentBlock[], message: SentMessage, way: 'steer' | 'queue'): void {
    const id = holdMessage(content, message, null, way)
    deliveries.set(id, window.setTimeout(() => arrive(id), nextDelivery()))
  }

  function arrive(id: string): void {
    deliveries.delete(id)
    const pending = pendingSubmissions.value.find((each) => each.id === id)
    if (!pending || pending.error !== null) {
      return
    }
    const wished = wishes.get(id)
    wishes.delete(id)
    if (state.phase === 'idle') {
      if (wished === 'remove') {
        // A message removed before it went is gone with its delivery.
        dropSubmission(id)
        removing.value = removing.value.filter((each) => each !== id)
        return
      }
      changeSubmission(id, { way: 'send' })
      deliver(token, false)
      return
    }
    const held = { id, content: dropSubmission(id) }
    if (pending.way === 'steer') {
      state.pendingSteers = [...state.pendingSteers, held]
    } else {
      state.queue = [...state.queue, held]
    }
    if (wished === 'remove' && pending.way === 'steer') {
      removePendingSteer(id)
    } else if (wished === 'remove') {
      removeQueued(id)
    } else if (wished === 'sendNow' && pending.way === 'steer') {
      interruptPendingSteer(id)
    } else if (wished === 'sendNow') {
      sendQueued(id)
    }
  }

  /** Keeps Remove or Send Now pressed on a message on its way as the wish; false for any other message. */
  function keepWish(id: string, wished: 'remove' | 'sendNow'): boolean {
    if (!deliveries.has(id)) {
      return false
    }
    wishes.set(id, wished)
    return true
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
    deliveredBy = Math.max(deliveredBy, Date.now() + CONFIRM_MS)
    at(run, CONFIRM_MS, () => {
      confirmMessage()
      body(run)
    })
  }

  /**
   * The message on its way that starts the turn, sent as the next message;
   * a steer or a queued message on its way is not the turn's message.
   */
  function turnMessage(): PendingSubmissionState | undefined {
    return pendingSubmissions.value.find((pending) => pending.error === null && pending.way === 'send')
  }

  /** The turn's message is written, under the id it was sent with: the message is no longer pending. */
  function confirmMessage(): void {
    const pending = turnMessage()
    if (pending) {
      append(userBlock(dropSubmission(pending.id), pending.id))
    }
  }

  function undeliver(error: string): void {
    const pending = turnMessage()
    if (pending) {
      changeSubmission(pending.id, { error })
    }
  }

  /** A new conversation whose first message was not delivered, as a page shows it after the failure. */
  function undelivered(content: UserContentBlock[], message: SentMessage, error: string): void {
    cancel()
    state.phase = 'idle'
    state.blocks = []
    forgetSubmissions()
    holdMessage(content, message, error)
  }

  /**
   * Retry sends the undelivered message again with its id, as a message, as
   * the product's does, once `ready`, the upload it tries again, has its
   * files; this time it arrives: it starts a turn while nothing runs, and
   * waits in the queue otherwise.
   */
  function retrySubmission(id: string, ready?: Promise<UserContentBlock[]>): void {
    const pending = pendingSubmissions.value.find((each) => each.id === id)
    if (!pending?.error) {
      return
    }
    if (state.phase !== 'idle') {
      changeSubmission(id, { error: null, way: 'queue' })
      afterUploads(id, ready, () => deliveries.set(id, window.setTimeout(() => arrive(id), nextDelivery())))
      return
    }
    cancel()
    changeSubmission(id, { error: null, way: 'send' })
    afterUploads(id, ready, () => deliver(token, false))
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
      // Steers that came during the pass are read once it ends.
      if (writeSteers()) {
        thinkThenReply(run, WAIT_MS, THINK_2)
        state.phase = 'running'
        return
      }
      state.phase = 'idle'
    })
  }

  /**
   * A message sent as a steer: it shows as one from the send, and once held
   * it waits as a pending steer until the turn's next boundary. With nothing
   * running, the steer is refused and the message goes as a message, as the
   * product sends it; a message on its way that starts a turn counts as
   * running.
   */
  function steer(content: UserContentBlock[], message: SentMessage): void {
    if (state.phase === 'idle' && !startingTurn.value) {
      turn(content, message)
      return
    }
    deliverWaiting(content, message, 'steer')
  }

  /** A message sent while a turn runs waits in the queue, as the product's does, and shows there from the send. */
  function queue(content: UserContentBlock[], message: SentMessage): void {
    deliverWaiting(content, message, 'queue')
  }

  /** Remove takes the row away at once, as the product's does; the simulated server holds the message no more right after. */
  function removeQueued(id: string): void {
    if (keepWish(id, 'remove')) {
      removing.value = [...removing.value, id]
      return
    }
    state.queue = state.queue.filter((entry) => entry.id !== id)
    removing.value = removing.value.filter((each) => each !== id)
  }

  /**
   * Send now on a queued message, as the product's: while a turn runs, the
   * turn ends at once without stopping anything, with its pending steers
   * written, and the message starts the next turn; otherwise the message
   * moves to the front of the queue.
   */
  function sendQueued(id: string): void {
    if (keepWish(id, 'sendNow')) {
      return
    }
    const item = state.queue.find((entry) => entry.id === id)
    if (!item) {
      return
    }
    removeQueued(id)
    if (state.phase === 'idle') {
      state.queue = [item, ...state.queue]
      return
    }
    cutRound()
    for (const pending of state.pendingSteers) {
      append(steerBlock(pending.content))
    }
    state.pendingSteers = []
    append(userBlock(item.content, item.id))
    runTurn(token)
  }

  function removePendingSteer(id: string): void {
    if (keepWish(id, 'remove')) {
      removing.value = [...removing.value, id]
      return
    }
    state.pendingSteers = state.pendingSteers.filter((entry) => entry.id !== id)
    removing.value = removing.value.filter((each) => each !== id)
  }

  /**
   * Send now on a pending steer, as the product's: the running round is cut
   * short without stopping anything, the steer is written, and the turn
   * goes on with it.
   */
  function interruptPendingSteer(id: string): void {
    if (keepWish(id, 'sendNow')) {
      return
    }
    const pending = state.pendingSteers.find((entry) => entry.id === id)
    if (!pending) {
      return
    }
    removePendingSteer(id)
    cutRound()
    append(steerBlock(pending.content))
    thinkThenReply(token, WAIT_MS, THINK_2)
  }

  /**
   * Cuts the running round short for a message sent now: the scripted reply
   * stops where it is, and the running call returns at once while its
   * command moves to the background and keeps running.
   */
  function cutRound(): void {
    const call = runningTool
    // The command is not stopped: `cancel` ends only the running call's.
    runningTool = null
    cancel()
    const block = state.blocks.find((entry) => entry.id === call)
    if (block?.type === 'tool_call') {
      replace(block.id, { ...block, status: 'completed' })
    }
  }

  function steerBlock(content: UserContentBlock[]): Block {
    return {
      type: 'steer',
      id: nextId('steer'),
      turnId: nextId('turn'),
      createdAt: now(),
      model: demoModel,
      content,
    }
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
    forgetSubmissions()
    const run = token
    state.lastError = null
    state.pendingAction = null
    state.load = 'ready'
    state.phase = 'idle'
    // A play starts over: what waited for the last one goes with it.
    state.pendingSteers = []
    state.queue = []

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

    if (kind === 'steer' || kind === 'queue') {
      // The agent's command runs; the user sends a steer, or a message to
      // the queue, which shows as one from the send, its line and controls
      // in view while it is delivered.
      const toolId = nextId('tool')
      state.blocks = [userBlock([{ type: 'text', text: USER_TEXT }]), tool(toolId, now(), 'executing')]
      state.phase = 'running'
      runningTool = toolId
      at(run, ACK_MS, () => {
        const content: UserContentBlock[] = [{ type: 'text', text: STEER_TEXT }]
        const message: SentMessage = { text: STEER_TEXT, attachments: [] }
        if (kind === 'steer') {
          steer(content, message)
        } else {
          queue(content, message)
        }
      })
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
      // The socket is lost: the tail row says Connecting under the message,
      // which goes with its id once the socket is back.
      state.load = 'reconnecting'
      holdMessage([{ type: 'text', text: USER_TEXT }], { text: USER_TEXT, attachments: [] }, null)
      at(run, OFFLINE_MS, () => {
        state.load = 'ready'
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

  onBeforeUnmount(() => {
    cancel()
    forgetSubmissions()
  })

  return {
    state,
    pendingSubmissions,
    removing,
    startingTurn,
    play,
    settleWork,
    turn,
    retrySubmission,
    undelivered,
    resume,
    stop,
    compact,
    steer,
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

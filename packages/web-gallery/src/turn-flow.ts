import { computed, onBeforeUnmount, reactive, shallowRef, watch } from 'vue'
import type { AgentMessage, Block, ChangedFile, ContextUsage, MessageComment, UserContentBlock } from '@demicodes/protocol'
import { CHROME_ROLL_MS } from '@demicodes/web-ui/ui/chrome-roll'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { ChatSessionState, ConversationState, PendingSubmissionState } from '@demicodes/web-ui/agent/types'
import { pressed, refused, serverWaits, waitClock } from '@demicodes/web-ui/agent/wait-clock'
import { agentWorks, markSent, markWithdrawn, turnStartingMessage, type SendWay } from '@demicodes/web-ui/agent/send-way'
import { segmentStreamUnits } from '@demicodes/web-ui/ui/stream-reveal'
import { contentCopy, type CopiedMessage } from '@demicodes/web-ui/agent/message-input/copied-message'
import { showRecalled } from '@demicodes/web-ui/agent/recalled-toast'
import { productWould } from './product-would'
import { demoModel, editedFile, inRequests, inRun, listedFiles, shellView, withLists, type ShellView } from './fixtures/blocks'
import { wholeHistory } from '@demicodes/web-ui/agent/history'
import { usageAt } from './fixtures/catalog'
import { suitePathsUpdate } from './fixtures/subagents'
import { printLive } from './live-command'

/**
 * The flows a specimen runs:
 *
 * - `turn`: a full turn from a sent message, whose delivery the server
 *   confirms.
 * - `undelivered`: a sent message whose delivery fails, until Retry sends it
 *   again.
 * - `offline`: a message sent while the socket is lost, under Connecting, which
 *   goes once the socket is back.
 * - `resume` and `retry`: recover an aborted or failed tail.
 * - `connect`: opens over a dropped socket.
 * - `stream`: thinking then reply with nothing waited for.
 * - `work`: a turn of many steps, thinking without text and commands in a row,
 *   one failing, two changing files.
 *
 * Every fixture only changes conversation state, the way the product's runtime
 * does: the transcript's tail row, its faces and the handoff into a block are
 * `web-ui`'s.
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

/**
 * The composer a specimen shows beside its flow, where Recall gives a
 * message back as the product's does.
 */
export interface RecallComposer {
  /** Puts the message into the composer when it is empty, the caret at its end; false when it holds a draft, which stays. */
  takeBack(message: CopiedMessage): boolean
  /** Sends the message again the way it was sent, its files attached again without their bytes. */
  sendAgain(message: CopiedMessage, way: 'steer' | 'queue'): void
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
/** How long a replay shows the Stop or the error before it presses Continue or Resume, so the row is seen to roll. */
const PRESS_MS = 900
/** The simulated server admits a sent message: the turn starts before the message is written, as the backend's does. */
const ADMIT_MS = 400
/** The simulated server writes the sent message to the transcript, which confirms its delivery; past a second, so the wait shows its clock first. */
const CONFIRM_MS = 1600
/**
 * What a simulated delivery failure says: the backend's refusal of a send
 * for a failure inside it, with the request to quote; a lost connection marks
 * nothing, the page sends again.
 */
const DELIVERY_ERROR = 'The message couldn’t be sent.'
const DELIVERY_REQUEST = 'req_ffb05w4w6kxpdtz0'
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
/** How long a stopped command shows Stopping… before its runner says that it ended. */
const STOPPING_MS = 3000
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
  // A model may think twice in a row; the page shows the two as one step.
  { kind: 'think', text: 'The status file is the only other place that names the old paths, so one edit covers it.' },
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
    blocks: inRequests(options.blocks ?? []),
    get history() {
      return wholeHistory(this.blocks)
    },
    shownAt: null,
    instructions: [],
    summaries: {},
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    starting: null,
    pendingCalls: [],
    waitingSince: null, waitAction: null, requestOutput: null, turnSpeed: null,
    model: {
      providerId: demoModel.providerId,
      modelId: demoModel.model.id,
      thinkingEffort: null,
      serviceTierId: null,
    },
    lastError: null,
    load: 'ready',
    pendingAction: null,
    waitClock: waitClock(),
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
  /** The waiting messages whose Recall is on its way, by id, which the list shows nowhere. */
  const removing = shallowRef<string[]>([])
  /** The composer the specimen shows, while it is mounted. */
  let composer: RecallComposer | null = null
  /**
   * The message that starts a turn is on its way: the turn is as good as
   * running, so the composer shows Stop, and a message sent meanwhile
   * steers it or waits in the queue, as the product's does.
   */
  const startingTurn = computed(() => turnStartingMessage(state.phase, pendingSubmissions.value) !== undefined)
  /**
   * The simulated server's answers to the steers and queued messages on
   * their way, by id, which a turn cut short or stopped does not cancel, as
   * a session holds what it was sent whatever its turn does.
   */
  const deliveries = new Map<string, number>()
  /** When the last message on its way is delivered, as `Date.now()`: the next one goes after it, never ahead. */
  let deliveredBy = 0
  /** Recall or Steer pressed on a steer or queued message on its way, by id, carried out once it is held. */
  const wishes = new Map<string, 'recall' | 'sendNow'>()
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
    // A stop whose end the cleared timers would have brought ends now.
    for (const terminal of state.terminals) {
      if (terminal.stopping) {
        endTerminal(terminal, 'aborted')
      }
    }
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

  /** A block written again keeps the run and the request it was stored in. */
  function replace(id: string, next: Block): void {
    state.blocks = state.blocks.map((block) => (block.id === id
      ? ({ ...next, ...('run' in block ? { run: block.run } : {}), ...(block.request !== undefined ? { request: block.request } : {}) }) as Block
      : block))
  }

  /**
   * The model is asked, as the server would tell a page when it begins to
   * wait: from the turn's start and the end of each step. The model's first
   * block, or a call being written once its title is, ends the wait, and so
   * does the turn's end.
   */
  function waitForModel(action: string | null = null): void {
    tellWait(Date.now(), action)
  }

  /**
   * The server tells the agent's wait, begun by the action `action` when the
   * page's own action began it, as the product's runtime takes it.
   */
  function tellWait(since: number | null, action: string | null = null): void {
    state.waitingSince = since
    state.waitAction = since === null ? null : action
    serverWaits(state.waitClock, state.waitAction)
  }

  watch(() => state.phase, (phase) => {
    if (phase !== 'running') {
      tellWait(null)
    }
  })

  function append(block: Block): void {
    if (block.type === 'thinking' || block.type === 'text' || block.type === 'tool_call') {
      tellWait(null)
    }
    // Each block names its request, as the backend stores it: a message
    // starts one, and the rest belong to the one before them.
    const request = block.type === 'user' ? block.id : state.blocks.at(-1)?.request
    state.blocks = [...state.blocks, request === undefined ? block : ({ ...block, request } as Block)]
  }

  /**
   * Writes a step of the agent's work in its run, as the backend stores
   * it: the run of the step before it, past a `resume` and a Stop the user
   * continued, which end no run; a run of its own after anything else the
   * flow writes.
   */
  function appendStep(block: Block): void {
    const before = state.blocks.findLast((each) => each.type !== 'resume' && !(each.type === 'abort' && each.isResumed))
    const run = before && 'run' in before && before.run !== undefined ? before.run : block.id
    append({ ...block, run } as Block)
  }

  /**
   * A message's block, with the comments it carries; `turnId` is the id the
   * message was sent with, which the server keeps as its turn's.
   */
  function userBlock(content: UserContentBlock[], turnId = nextId('turn'), comments: readonly MessageComment[] = []): Block {
    return {
      type: 'user',
      id: nextId('user'),
      turnId,
      createdAt: now(),
      model: demoModel,
      content,
      ...(comments.length > 0 ? { comments: [...comments] } : {}),
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
      // It returns as the replay replaces it.
      endedAt: status === 'executing' ? null : now(),
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

  /** An empty thinking block arrives at `startMs`; its text streams once its row has rolled in. Returns when the text is complete. */
  function think(run: number, startMs: number, text: string): number {
    const id = nextId('think')
    let createdAt = ''
    at(run, startMs, () => {
      createdAt = now()
      appendStep(thinkingBlock(id, createdAt, ''))
    })
    return streamTextInto(run, startMs + CHROME_ROLL_MS, text, (partial) => {
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
        comments: pending.comments,
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
      delete terminal.stopping
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
      appendStep(tool(toolId, toolStartedAt, 'executing'))
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

  /** A call of the `work` turn, which returned at `endedAt`, the moment it is built unless said; a background one returned while its command still ran. */
  function workCall(id: string, createdAt: string, step: Extract<WorkStep, { kind: 'call' }>, status: ToolCallBlock['status'], endedAt = now()): ToolCallBlock {
    const text = `${step.lines.join('\n')}\n`
    return {
      type: 'tool_call',
      id,
      createdAt,
      model: demoModel,
      toolUseId: `${id}-use`,
      toolName: 'shell',
      status,
      endedAt: status === 'executing' ? null : endedAt,
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
    // The files the run changed so far, as each call's lists store them.
    let listed: ChangedFile[] = []
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
          appendStep(workNotice(nextId('notice'), now(), terminal.id, step.of))
          waitForModel()
        })
        t += STEP_ANSWER_MS
        continue
      }
      if (step.kind === 'receipt') {
        at(run, t, () => {
          appendStep(workReceipt(now(), step.message))
          waitForModel()
        })
        t += STEP_ANSWER_MS
        continue
      }
      const id = nextId('tool')
      let startedAt = ''
      // The model writes the call first: it shows nothing until its
      // description is written, then its row shows, which ends the wait,
      // and becomes the call's block when whole.
      at(run, t, () => {
        state.pendingCalls = [{ toolUseId: `${id}-use`, toolName: 'shell', description: null }]
      })
      at(run, t + 400, () => {
        tellWait(null)
        state.pendingCalls = [{ toolUseId: `${id}-use`, toolName: 'shell', description: step.description }]
      })
      t += 900
      at(run, t, () => {
        startedAt = now()
        state.pendingCalls = []
        appendStep(workCall(id, startedAt, step, 'executing'))
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
        const done = workCall(id, startedAt, step, 'completed')
        listed = [...listed, ...listedFiles(done)]
        replace(id, withLists(done, listed))
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
    // Each step starts where the one before ended and the model answered, as in the replay.
    let clock = Date.now() - WORK_STEPS.length * (TOOL_RUN_MS + STEP_ANSWER_MS)
    const next = (ms: number) => {
      const at = new Date(clock).toISOString()
      clock += ms
      return at
    }
    const calls = new Map<string, string>()
    let listed: ChangedFile[] = []
    state.blocks = [
      userBlock([{ type: 'text', text: WORK_TEXT }]),
      ...inRun(...WORK_STEPS.map((step): Block => {
        switch (step.kind) {
          case 'think':
            return thinkingBlock(nextId('think'), next(STEP_ANSWER_MS), step.text)
          case 'notice':
            return workNotice(nextId('notice'), next(0), calls.get(step.of) ?? '', step.of)
          case 'receipt':
            return workReceipt(next(0), step.message)
          case 'call': {
            const id = nextId('tool')
            calls.set(step.description, `cmd-${id}`)
            const startedAt = next(TOOL_RUN_MS)
            const done = workCall(id, startedAt, step, 'completed', new Date(clock).toISOString())
            clock += STEP_ANSWER_MS
            listed = [...listed, ...listedFiles(done)]
            return withLists(done, listed)
          }
        }
      })),
      textBlock(nextId('text'), next(0), WORK_REPLY),
    ]
  }

  /** The server acknowledges a recovery with the phase, before the transcript says so, as the backend does. */
  function acknowledgeRecovery(): void {
    state.phase = 'running'
    waitForModel(state.pendingAction?.id ?? null)
    state.pendingAction = null
  }

  /**
   * The recovery unwinds the turn, as the backend's does: the Stop it
   * continues is marked resumed, or the error it recovers from goes, and a
   * `resume` block says where the turn goes on.
   */
  function unwindRecovery(): void {
    const tail = state.blocks.at(-1)
    if (tail?.type === 'abort') {
      replace(tail.id, { ...tail, isResumed: true })
    } else if (tail?.type === 'error') {
      state.blocks = state.blocks.slice(0, -1)
    }
    append({ type: 'resume', id: nextId('resume'), turnId: nextId('turn'), createdAt: now(), model: demoModel, reason: null })
  }

  /** Send a message, with the comments it carries, on the current transcript; its turn runs once the server confirms it. */
  function turn(content: UserContentBlock[], message: SentMessage, comments: readonly MessageComment[] = []): void {
    cancel()
    const id = holdMessage(content, message, null, 'send', comments)
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
  function holdMessage(
    content: UserContentBlock[],
    message: SentMessage,
    error: string | null,
    way: SendWay = 'send',
    comments: readonly MessageComment[] = [],
    requestId: string | null = null,
  ): string {
    const id = nextId('message')
    sendLast({ id, way, text: message.text, attachments: message.attachments, comments, error, requestId })
    contents.set(id, content)
    return id
  }

  /** The message `message` is sent now, after every message held, which it follows as the product's do. */
  function sendLast(message: PendingSubmissionState): void {
    const next = inOrder([...pendingSubmissions.value, message])
    markSent(next, message.id)
    pendingSubmissions.value = next
    // A message that starts a turn makes the agent wait from its press.
    pressed(state.waitClock, message.id)
  }

  /** Copies of the messages on their way, whose order the next change marks. */
  function inOrder(submissions: readonly PendingSubmissionState[]) {
    return submissions.map((pending) => ({ ...pending, followers: [...pending.followers ?? []] }))
  }

  /**
   * The message `id` leaves the order without staying where it was sent:
   * the messages sent before it keep their place by the ones sent after it.
   */
  function withdrawFromOrder(id: string): void {
    const next = inOrder(pendingSubmissions.value.filter((pending) => pending.id !== id))
    markWithdrawn(next, id)
    pendingSubmissions.value = next
  }

  /** The message `id` leaves the messages on their way without being delivered. Answers what it sends. */
  function withdrawSubmission(id: string): UserContentBlock[] {
    withdrawFromOrder(id)
    const content = contents.get(id) ?? []
    contents.delete(id)
    return content
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
   * send: once the simulated server has answered the messages before it, it
   * holds this one, a steer in the running turn's steers and a queued message
   * in the queue, under the id it was sent with.
   *
   * - With nothing running, it is refused and goes as a message, which starts a
   *   turn, as the product sends it.
   * - Recall or Steer pressed meanwhile is carried out then.
   */
  function deliverWaiting(
    content: UserContentBlock[],
    message: SentMessage,
    way: 'steer' | 'queue',
    comments: readonly MessageComment[] = [],
  ): void {
    const id = holdMessage(content, message, null, way, comments)
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
      if (wished === 'recall') {
        // A message taken back before it went goes back to the user instead.
        removing.value = removing.value.filter((each) => each !== id)
        giveBack(withdrawSubmission(id), pending.way === 'steer' ? 'steer' : 'queue')
        return
      }
      changeSubmission(id, { way: 'send' })
      deliver(token, false)
      return
    }
    const held = { id, content: dropSubmission(id), comments: [...pending.comments ?? []] }
    if (pending.way === 'steer') {
      state.pendingSteers = [...state.pendingSteers, held]
    } else {
      state.queue = [...state.queue, held]
    }
    if (wished === 'recall' && pending.way === 'steer') {
      recallSteer(id)
    } else if (wished === 'recall') {
      recallQueued(id)
    } else if (wished === 'sendNow' && pending.way === 'steer') {
      interruptPendingSteer(id)
    } else if (wished === 'sendNow') {
      sendQueued(id)
    }
  }

  /** Keeps Recall or Steer pressed on a message on its way as the wish; false for any other message. */
  function keepWish(id: string, wished: 'recall' | 'sendNow'): boolean {
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
        undeliver(DELIVERY_ERROR, DELIVERY_REQUEST)
      })
      return
    }
    // The server starts the turn, and its wait, before it writes the message.
    at(run, ADMIT_MS, () => {
      state.phase = 'running'
      // Its wait names the message that began it.
      const message = turnMessage()?.id ?? null
      admitMessage()
      waitForModel(message)
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

  /**
   * The session took the turn's message: until the turn writes it, it is the
   * turn's starting message, which the server's state holds, and the page's
   * own row gives way to it, as the product's does.
   */
  function admitMessage(): void {
    const pending = turnMessage()
    if (pending) {
      state.starting = { id: pending.id, content: dropSubmission(pending.id), comments: [...pending.comments ?? []] }
    }
  }

  /** The turn's message is written, under the id it was sent with: it is no longer the turn's starting message. */
  function confirmMessage(): void {
    admitMessage()
    const starting = state.starting
    if (starting) {
      append(userBlock(starting.content, starting.id, starting.comments))
      state.starting = null
    }
  }

  function undeliver(error: string, requestId: string | null): void {
    const pending = turnMessage()
    if (pending) {
      changeSubmission(pending.id, { error, requestId })
      refused(state.waitClock, pending.id)
    }
  }

  /**
   * A new conversation whose first message was not delivered, as a page
   * shows it after the backend refused it: `error` in the user's words, and
   * the request ID of a failure inside the backend. `lasting` is a refusal
   * sending again cannot change, such as a file of the message that is gone,
   * whose row offers Edit first and no Retry.
   */
  function undelivered(content: UserContentBlock[], message: SentMessage, error: string, requestId: string | null = null, lasting = false): void {
    cancel()
    state.phase = 'idle'
    state.blocks = []
    forgetSubmissions()
    const id = holdMessage(content, message, error, 'send', [], requestId)
    if (lasting) {
      changeSubmission(id, { retryable: false })
    }
  }

  /**
   * Edit on the undelivered message, as the product's: its row goes, and the
   * message goes back into the specimen's composer when it is empty; a
   * specimen with no composer, or one holding a draft, says what the product
   * does, which puts it after the draft.
   */
  function editSubmission(id: string): void {
    if (!pendingSubmissions.value.some((pending) => pending.id === id && pending.error)) {
      return
    }
    const copy = contentCopy(withdrawSubmission(id))
    if (!composer?.takeBack(copy)) {
      productWould('Put the Message Back in the Composer')
    }
  }

  /** Delete on the undelivered message: its row goes at once. */
  function deleteSubmission(id: string): void {
    if (pendingSubmissions.value.some((pending) => pending.id === id && pending.error)) {
      withdrawSubmission(id)
    }
  }

  /**
   * The dock's Retry, as the product's: each undelivered message a resend can
   * deliver goes again with its id, in the order they were sent, as a new
   * message at the end, once `ready` answers the upload it tries again for it;
   * this time each arrives.
   *
   * - The first starts a turn while nothing runs, and the rest wait in the
   *   queue, as one does while a turn runs or a message on its way starts one.
   * - One a resend cannot deliver stays.
   */
  function retrySubmissions(ready?: (id: string) => Promise<UserContentBlock[]> | undefined): void {
    const again = pendingSubmissions.value.filter((pending) => pending.error !== null && pending.retryable !== false)
    for (const pending of again) {
      resend(pending, ready?.(pending.id))
    }
  }

  function resend(pending: PendingSubmissionState, ready: Promise<UserContentBlock[]> | undefined): void {
    const content = withdrawSubmission(pending.id)
    contents.set(pending.id, content)
    const works = agentWorks(state.phase, pendingSubmissions.value)
    // A new message at the end, which no message followed yet.
    sendLast({ ...pending, error: null, requestId: null, way: works ? 'queue' : 'send', followers: [] })
    if (works) {
      afterUploads(pending.id, ready, () => deliveries.set(pending.id, window.setTimeout(() => arrive(pending.id), nextDelivery())))
      return
    }
    cancel()
    afterUploads(pending.id, ready, () => deliver(token, false))
  }

  /** Resume or retry from the current tail, the way Resume and Retry do in the product. */
  function resume(): void {
    cancel()
    const run = token
    state.lastError = null
    const id = nextId('resume')
    state.pendingAction = { kind: 'resume', id }
    pressed(state.waitClock, id)
    at(run, ACK_MS, acknowledgeRecovery)
    at(run, ACK_MS + WAIT_MS / 2, unwindRecovery)
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
          kept: [],
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
      state.contextUsage = usageAt(0.06, { summary: 2_400 })
      // Steers that came during the pass are read once it ends.
      if (writeSteers()) {
        thinkThenReply(run, WAIT_MS, THINK_2)
        state.phase = 'running'
        waitForModel()
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
  function steer(content: UserContentBlock[], message: SentMessage, comments: readonly MessageComment[] = []): void {
    if (!agentWorks(state.phase, pendingSubmissions.value)) {
      turn(content, message, comments)
      return
    }
    deliverWaiting(content, message, 'steer', comments)
  }

  /** A message sent while a turn runs waits in the queue, as the product's does, and shows there from the send. */
  function queue(content: UserContentBlock[], message: SentMessage, comments: readonly MessageComment[] = []): void {
    deliverWaiting(content, message, 'queue', comments)
  }

  /**
   * Recall takes the row away at once, as the product's does, and the
   * message goes back to the user; the simulated server holds it no more
   * right after, so it never refuses.
   */
  function recallQueued(id: string): void {
    recall(id, 'queue')
  }

  function recallSteer(id: string): void {
    recall(id, 'steer')
  }

  function recall(id: string, way: 'steer' | 'queue'): void {
    if (keepWish(id, 'recall')) {
      removing.value = [...removing.value, id]
      return
    }
    const held = (way === 'steer' ? state.pendingSteers : state.queue).find((entry) => entry.id === id)
    drop(id)
    removing.value = removing.value.filter((each) => each !== id)
    if (held) {
      withdrawFromOrder(id)
      giveBack(held.content, way)
    }
  }

  /** The simulated server holds the steer or queued message `id` no more. */
  function drop(id: string): void {
    state.pendingSteers = state.pendingSteers.filter((entry) => entry.id !== id)
    state.queue = state.queue.filter((entry) => entry.id !== id)
  }

  /**
   * Gives a message Recall took back to the user, as the product does:
   * into the specimen's composer when it is empty; otherwise a toast's Undo
   * sends it again the way it was sent, through the composer when the
   * specimen has one.
   */
  function giveBack(content: UserContentBlock[], way: 'steer' | 'queue'): void {
    const copy = contentCopy(content)
    if (composer?.takeBack(copy)) {
      return
    }
    showRecalled(() => {
      if (composer) {
        composer.sendAgain(copy, way)
      } else {
        deliverWaiting(content, { text: copy.text, attachments: [] }, way)
      }
    })
  }

  /** The specimen's composer takes back what Recall gives; answers the call that lets it go again. */
  function attachComposer(host: RecallComposer): () => void {
    composer = host
    return () => {
      if (composer === host) {
        composer = null
      }
    }
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
    drop(id)
    if (state.phase === 'idle') {
      state.queue = [item, ...state.queue]
      return
    }
    cutRound()
    for (const pending of state.pendingSteers) {
      append(steerBlock(pending.content, pending.comments))
    }
    state.pendingSteers = []
    append(userBlock(item.content, item.id, item.comments))
    runTurn(token)
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
    drop(id)
    cutRound()
    append(steerBlock(pending.content, pending.comments))
    waitForModel()
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

  function steerBlock(content: UserContentBlock[], comments: readonly MessageComment[] = []): Block {
    return {
      type: 'steer',
      id: nextId('steer'),
      turnId: nextId('turn'),
      createdAt: now(),
      model: demoModel,
      content,
      ...(comments.length > 0 ? { comments: [...comments] } : {}),
    }
  }

  /**
   * Stop Agent: the child stops at once, and each command it still runs
   * shows Stopping… until its runner, simulated here, says that it ended
   * (`crates/agent-server/src/server/tree/supervisor.rs`).
   */
  function abortSubagent(id: string): void {
    const agent = state.subagents.find((entry) => entry.id === id)
    if (agent?.phase === 'running') {
      agent.phase = 'aborted'
      agent.endedAt = now()
      for (const terminal of state.terminals) {
        if (terminal.subagentId === id) {
          abortTerminal(terminal.id)
        }
      }
    }
  }

  function abortSubagents(): void {
    for (const agent of state.subagents) {
      abortSubagent(agent.id)
    }
  }

  /**
   * End Command: the command shows Stopping… until its runner, simulated
   * here, says that it ended (`crates/runner-shell/src/job.rs`).
   */
  function abortTerminal(id: string): void {
    const terminal = state.terminals.find((entry) => entry.id === id)
    if (terminal?.phase !== 'running' || terminal.stopping) {
      return
    }
    terminal.stopping = true
    // A specimen that ran its commands again meanwhile took the stop back.
    timers.push(window.setTimeout(() => terminal.stopping && endTerminal(terminal, 'aborted'), STOPPING_MS))
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
    state.starting = null

    if (kind === 'stream') {
      // Thinking is the tail from the first running frame, so nothing is waited for.
      const id = nextId('think')
      const createdAt = now()
      state.blocks = inRun(thinkingBlock(id, createdAt, ''))
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
      state.blocks = [userBlock([{ type: 'text', text: USER_TEXT }]), ...inRun(tool(toolId, now(), 'executing'))]
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
      // The server tells the wait it went on with once the socket is back.
      const asked = Date.now()
      at(run, ACK_MS, () => {
        state.load = 'ready'
        tellWait(asked)
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
      at(run, PRESS_MS, resume)
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
      at(run, PRESS_MS, resume)
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
    retrySubmissions,
    editSubmission,
    deleteSubmission,
    undelivered,
    resume,
    stop,
    compact,
    steer,
    queue,
    recallQueued,
    sendQueued,
    recallSteer,
    interruptPendingSteer,
    attachComposer,
    abortSubagent,
    abortSubagents,
    abortTerminal,
  }
}

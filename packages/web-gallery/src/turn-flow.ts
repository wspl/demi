import { onBeforeUnmount, reactive } from 'vue'
import type { Block } from '@demicodes/core'
import { ACTIVITY_HANDOFF_MS } from '@demicodes/web-ui/agent/activity-slot'
import type { ToolCallBlock } from '@demicodes/web-ui/agent/block-types'
import { conversationStatus } from '@demicodes/web-ui/agent/conversation-status'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import type { ChatSessionState, ConversationState } from '@demicodes/web-ui/agent/types'
import { segmentStreamUnits } from '@demicodes/web-ui/ui/stream-reveal'
import { demoModel } from './fixtures/blocks'

/**
 * `turn` is a full turn from a sent message; `resume` and `retry` recover an
 * aborted or failed tail; `connect` opens over a dropped socket; `stream` is
 * thinking then reply with nothing waited for. Every fixture only changes
 * conversation state, the way the product's runtime does: the transcript's
 * tail row, its faces and the handoff into a block are `web-ui`'s.
 */
export type TurnFlowKind = 'turn' | 'resume' | 'retry' | 'connect' | 'stream'

/** The state `ChatSession` reads, over the full conversation state `conversationStatus` derives from. */
export type TurnFlowState = ConversationState & ChatSessionState

const THINK_1 = 'The cookie name changed from sid to session. The helper already writes the new header. The test is the one still looking for sid.'
const THINK_2 = 'The helper is fine. Update the assertion in auth.test.ts and leave cookie.ts alone.'
const REPLY = 'The cookie helper is fine. The test still expects `sid`.\n\nI updated the assertion in `auth.test.ts` and left `cookie.ts` alone.'
const USER_TEXT = 'The login test in packages/web/src/auth.test.ts is failing after the session cookie rename.'
const RETRY_ERROR = 'Anthropic API request failed with HTTP 529: Overloaded. The upstream service is temporarily unavailable.'
/** The simulated server's acknowledgement of a recovery or a reconnect. */
const ACK_MS = 800
const WAIT_MS = 80
/** Time the model takes before its first output in a turn. */
const FIRST_OUTPUT_MS = 1000
const TOOL_RUN_MS = 1400
const FEED_CHARS = 4
const FEED_MS = 90

export interface TurnFlowOptions {
  id?: string
  title?: string
  blocks?: Block[]
  subagents?: SubagentRecord[]
  terminals?: TerminalRecord[]
}

export function useTurnFlow(options: TurnFlowOptions = {}) {
  const state: TurnFlowState = reactive({
    id: options.id ?? 'turn-flow',
    cwd: '/',
    title: options.title ?? 'Login test',
    createdAt: new Date().toISOString(),
    blocks: options.blocks ?? [],
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    model: {
      providerId: demoModel.providerId,
      modelId: demoModel.model.id,
      thinkingEffort: null,
      serviceTierId: null,
    },
    draft: null,
    isResultSeen: true,
    hasContent: false,
    lastError: null,
    load: 'ready',
    pendingAction: null,
    archived: false,
    get status() {
      return conversationStatus(state)
    },
    scroll: null,
    subagents: options.subagents ?? [],
    terminals: options.terminals ?? [],
  })
  const timers: number[] = []
  let token = 0
  let sequence = 0

  function cancel(): void {
    token += 1
    for (const id of timers) {
      window.clearTimeout(id)
    }
    timers.length = 0
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

  function nextId(kind: string): string {
    sequence += 1
    return `${kind}-${sequence}`
  }

  function replace(id: string, next: Block): void {
    state.blocks = state.blocks.map((block) => (block.id === id ? next : block))
  }

  function append(block: Block): void {
    state.blocks = [...state.blocks, block]
  }

  function userBlock(text: string): Block {
    return {
      type: 'user',
      id: nextId('user'),
      turnId: nextId('turn'),
      createdAt: now(),
      model: demoModel,
      content: [{ type: 'text', text }],
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
      ? [
          {
            type: 'text' as const,
            text: 'packages/web/src/auth.test.ts:18:    expect(cookie.name).toBe("sid")\n',
          },
        ]
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
        script: 'rg -n "sid" packages/web/src/auth.test.ts',
        description: 'Find the old cookie name in the login test',
      }),
      streamingOutput: [],
      output,
      view: {
        chunks: [{ stream: 'stdout', text: output[0]?.text ?? '' }],
      },
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

  /** A whole turn on the current transcript: request, think, run a tool, think, reply. */
  function runTurn(run: number): void {
    state.phase = 'running'
    const thought1 = think(run, FIRST_OUTPUT_MS, THINK_1)
    const toolId = nextId('tool')
    let toolStartedAt = ''
    at(run, thought1 + 200, () => {
      toolStartedAt = now()
      append(tool(toolId, toolStartedAt, 'executing'))
    })
    const toolDone = thought1 + 200 + TOOL_RUN_MS
    at(run, toolDone, () => {
      replace(toolId, tool(toolId, toolStartedAt, 'completed'))
    })
    thinkThenReply(run, toolDone + WAIT_MS, THINK_2)
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

  /** Send a message on the current transcript. */
  function turn(text: string): void {
    cancel()
    const run = token
    append(userBlock(text.trim() || USER_TEXT))
    runTurn(run)
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

  /** Abort a running turn: the transcript ends with an abort record to resume from. */
  function stop(): void {
    if (state.phase !== 'running') {
      return
    }
    cancel()
    state.phase = 'idle'
    append({
      type: 'abort',
      id: nextId('abort'),
      createdAt: now(),
      model: demoModel,
      isResumed: false,
    })
  }

  /** Reset to a fixture and play it. */
  function play(kind: TurnFlowKind = 'turn'): void {
    cancel()
    const run = token
    state.lastError = null
    state.pendingAction = null
    state.load = 'ready'
    state.phase = 'idle'

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
      state.blocks = [userBlock(USER_TEXT)]
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
        userBlock(USER_TEXT),
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
        userBlock(USER_TEXT),
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
    turn(USER_TEXT)
  }

  onBeforeUnmount(cancel)

  return {
    state,
    play,
    turn,
    resume,
    stop,
  }
}

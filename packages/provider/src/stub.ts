import type { AgentProvider, InferenceRequest, ProviderEvent } from './types'

/**
 * Scripted provider for testing. Each "turn" is a list of events to yield.
 *
 * The provider advances through turns sequentially: each `run()` call
 * consumes the next turn's events. This lets tests script multi-turn
 * flows like "respond with text → request a tool call → receive tool
 * result → respond with follow-up text".
 *
 * Tool call round-trip: a turn yields `tool_call_requested`. AgentSession
 * executes the tool, then calls `run()` again with the tool result in
 * `items`. The next turn's events are then yielded.
 *
 * Turn scripts can also be functions of the incoming request, so a test
 * can inspect the items it received (e.g. assert tool_result present).
 */
export class StubProvider implements AgentProvider {
  private turns: TurnScript[]
  private cursor = 0

  constructor(turns: TurnScript[]) {
    this.turns = turns
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const turn = this.turns[this.cursor]
    this.cursor += 1
    if (turn === undefined) {
      throw new Error(`StubProvider: no turn scripted for call #${this.cursor} (ran out of turns)`)
    }
    const events = typeof turn === 'function' ? turn(request) : turn
    for (const event of events) {
      yield event
    }
  }

  /** Number of turns consumed so far. Useful for assertions. */
  get consumedTurns(): number {
    return this.cursor
  }
}

type TurnScript = ProviderEvent[] | ((request: InferenceRequest) => ProviderEvent[])

// ── common event helpers ────────────────────────────────────────────

export const events = {
  text: (text: string): ProviderEvent => ({ type: 'text_delta', text }),
  response: (usage?: Partial<UsageLike>): ProviderEvent => ({
    type: 'response',
    usage: {
      inputTokens: usage?.inputTokens ?? 0,
      outputTokens: usage?.outputTokens ?? 0,
      cacheReadTokens: usage?.cacheReadTokens ?? 0,
      cacheWriteTokens: usage?.cacheWriteTokens ?? 0,
    },
  }),
  toolCall: (toolUseId: string, toolName: string, input: unknown): ProviderEvent => ({
    type: 'tool_call_requested',
    toolUseId,
    toolName,
    input,
  }),
  error: (message: string, code?: string): ProviderEvent => ({
    type: 'error',
    message,
    code: code ?? null,
  }),
  abort: (): ProviderEvent => ({ type: 'abort' }),
}

interface UsageLike {
  inputTokens: number
  outputTokens: number
  cacheReadTokens: number
  cacheWriteTokens: number
}

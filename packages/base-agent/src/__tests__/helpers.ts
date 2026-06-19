import { expect } from 'bun:test'
import type { Block, ModelSelection, UserContentBlock } from '@demi/core'
import type { AgentProvider, InferenceItem, InferenceRequest, ProviderEvent } from '@demi/provider'
import {
  AgentSession,
  Transcript,
  type AgentDefinition,
  type AgentSessionOptions,
  type AgentSessionSnapshot,
} from '../index'

export interface TestState {
  toolCalls: number
}

export const model: ModelSelection = {
  providerId: 'stub',
  model: {
    id: 'test-model',
    name: 'Test Model',
    contextWindow: 100_000,
    inputLimit: null,
    thinking: [],
    acceptedExtensions: [],
  },
  thinking: null,
}

export function text(value: string): UserContentBlock[] {
  return [{ type: 'text', text: value }]
}

export function createDefinition(
  overrides: Partial<AgentDefinition<TestState>> = {},
): AgentDefinition<TestState> {
  return {
    name: 'test-agent',
    initialState: () => ({ toolCalls: 0 }),
    systemPrompt: () => 'system prompt',
    preamble: () => 'preamble',
    tools: () => [],
    ...overrides,
  }
}

export function createSession(
  provider: AgentProvider,
  definition: AgentDefinition<TestState> = createDefinition(),
  transcript?: Transcript,
  selection: ModelSelection = model,
  options: Partial<AgentSessionOptions<TestState>> = {},
): AgentSession<TestState> {
  let id = 0
  return new AgentSession(
    {
      provider,
      model: selection,
      cwd: '/workspace',
      definition,
      transcript,
    },
    {
      idFactory: () => `id-${++id}`,
      now: () => '2026-06-17T00:00:00.000Z',
      compaction: { keepRecentTokens: 1 },
      ...options,
    },
  )
}

export function makeTranscript(): Transcript {
  let id = 0
  return new Transcript([], {
    idFactory: () => `seed-${++id}`,
    now: () => '2026-06-17T00:00:00.000Z',
  })
}

export class MemorySessionStore<State> {
  readonly snapshots: Array<AgentSessionSnapshot<State>> = []

  saveSnapshot(snapshot: AgentSessionSnapshot<State>): void {
    this.snapshots.push(snapshot)
  }
}

export class RecordingProvider implements AgentProvider {
  readonly requests: InferenceRequest[] = []
  private cursor = 0

  constructor(private readonly turns: TurnScript[]) {}

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const turn = this.turns[this.cursor]
    this.cursor += 1
    this.requests.push(request)
    if (turn === undefined) {
      throw new Error(`RecordingProvider: no turn scripted for call #${this.cursor}`)
    }
    const output = await (typeof turn === 'function' ? turn(request) : turn)
    if (isAsyncIterable(output)) {
      for await (const event of output) yield event
      return
    }
    for (const event of output) yield event
  }
}

export function assertNoOrphanToolItems(items: InferenceItem[]): void {
  const openToolUses = new Set<string>()
  const completedToolUses = new Set<string>()
  for (const item of items) {
    if (item.type === 'tool_use') {
      expect(completedToolUses.has(item.toolUseId)).toBe(false)
      openToolUses.add(item.toolUseId)
      continue
    }
    if (item.type === 'tool_result') {
      expect(openToolUses.has(item.toolUseId)).toBe(true)
      openToolUses.delete(item.toolUseId)
      completedToolUses.add(item.toolUseId)
    }
  }
  expect(openToolUses.size).toBe(0)
}

export function assertTranscriptInvariants(blocks: Block[]): void {
  const ids = new Set<string>()
  const boundaryIds = new Set<string>()
  const toolUseIds = new Set<string>()
  for (const block of blocks) {
    expect(block.id).toBeTruthy()
    expect(block.createdAt).toBeTruthy()
    expect(ids.has(block.id)).toBe(false)
    ids.add(block.id)
    if (block.type === 'compaction_boundary') {
      boundaryIds.add(block.id)
    }
    if (block.type === 'compaction_marker') {
      expect(boundaryIds.has(block.boundaryId)).toBe(true)
      expect(block.compactedTokens).toBeGreaterThanOrEqual(0)
    }
    if (block.type === 'tool_call') {
      expect(toolUseIds.has(block.toolUseId)).toBe(false)
      toolUseIds.add(block.toolUseId)
    }
    if (block.type === 'tool_call' && block.status !== 'executing') {
      expect(block.output.length).toBeGreaterThan(0)
    }
  }
}

type TurnOutput = ProviderEvent[] | AsyncIterable<ProviderEvent> | Promise<ProviderEvent[] | AsyncIterable<ProviderEvent>>
type TurnScript = TurnOutput | ((request: InferenceRequest) => TurnOutput)

function isAsyncIterable(value: unknown): value is AsyncIterable<ProviderEvent> {
  return value !== null && typeof value === 'object' && Symbol.asyncIterator in value
}

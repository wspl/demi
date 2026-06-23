import { expect, test } from 'bun:test'
import type { ModelSelection, UserContentBlock } from '@demi/core'
import type { AgentProvider, InferenceRequest, InferenceSteer, ProviderEvent, ProviderRun } from '@demi/provider'
import { StubProvider, createProviderRun, events } from '@demi/provider/testing'
import {
  AgentSession,
  Transcript,
  type AgentHarnessRuntime,
  type AgentSessionOptions,
  type AgentSessionSnapshot,
  type AgentToolInvokeResult,
  type SessionEvent,
} from '../index'

const model: ModelSelection = {
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

function text(value: string): UserContentBlock[] {
  return [{ type: 'text', text: value }]
}

function createRuntime(overrides: Partial<AgentHarnessRuntime<{ toolCalls: number }>> = {}): AgentHarnessRuntime<{ toolCalls: number }> {
  return {
    harnessName: 'test-agent',
    initialState: () => ({ toolCalls: 0 }),
    systemPrompt: () => 'system prompt',
    preamble: () => 'preamble',
    tools: () => [],
    ...overrides,
  }
}

function createSession(
  provider: AgentProvider,
  runtime: AgentHarnessRuntime<{ toolCalls: number }> = createRuntime(),
  transcript?: Transcript,
  selection: ModelSelection = model,
  options: Partial<AgentSessionOptions<{ toolCalls: number }>> = {},
): AgentSession<{ toolCalls: number }> {
  let id = 0
  return new AgentSession(
    {
      provider,
      model: selection,
      cwd: '/workspace',
      runtime,
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

class RecordingProvider implements AgentProvider {
  readonly runModelIds: string[] = []
  disposed = false
  constructor(private readonly reply: string) {}
  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    this.runModelIds.push(request.modelId)
    yield { type: 'text_delta', text: this.reply }
    yield { type: 'response', usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 } }
  }
  async dispose(): Promise<void> {
    this.disposed = true
  }
}

test('updateModel swaps the provider, disposes the old one, and the next turn uses the new model', async () => {
  const providerA = new RecordingProvider('from A')
  const providerB = new RecordingProvider('from B')
  const modelA: ModelSelection = { ...model, model: { ...model.model, id: 'model-a' } }
  const modelB: ModelSelection = { ...model, model: { ...model.model, id: 'model-b' } }
  const session = createSession(providerA, createRuntime(), undefined, modelA)

  await session.send(text('hi'))
  expect(providerA.runModelIds).toEqual(['model-a'])
  expect(providerA.disposed).toBe(false)

  // Switch to a different provider/model (what a cross-provider model switch does). Recorded now,
  // applied at the next turn.
  session.updateModel(providerB, modelB)
  await session.send(text('again'))

  // The conversation keeps working: the old provider is released, the new one runs the new model,
  // and the old provider is never touched after the swap.
  expect(providerA.disposed).toBe(true)
  expect(providerA.runModelIds).toEqual(['model-a'])
  expect(providerB.runModelIds).toEqual(['model-b'])
  expect(session.transcript().blocks.filter((block) => block.type === 'user')).toHaveLength(2)
})

test('switching to a smaller-window model defers compaction to the next turn, done by the OLD model', async () => {
  const big = new RecordingProvider('summary from the big model')
  const small = new RecordingProvider('small reply')
  const bigModel: ModelSelection = { ...model, model: { ...model.model, id: 'big', contextWindow: 100_000 } }
  // Tiny window so the accumulated history is over threshold for the target model.
  const smallModel: ModelSelection = { ...model, model: { ...model.model, id: 'small', contextWindow: 8 } }
  const session = createSession(big, createRuntime(), undefined, bigModel)

  await session.send(text('first turn with some content to fill the transcript'))
  await session.send(text('second turn with even more content to fill the transcript'))
  const bigRunsBeforeSwitch = big.runModelIds.length

  // Recording the switch is lazy: nothing compacts, nothing swaps, no model is touched yet.
  session.updateModel(small, smallModel)
  expect(big.runModelIds.length).toBe(bigRunsBeforeSwitch)
  expect(small.runModelIds).toEqual([])

  // The next turn's preflight compacts with the OLD (big) model — because the small model can't
  // fit the history — produces a summary boundary, and only then runs the turn on the small model.
  await session.send(text('after switch'))
  expect(big.runModelIds.length).toBeGreaterThan(bigRunsBeforeSwitch) // big did the compaction
  expect(big.runModelIds.every((id) => id === 'big')).toBe(true)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)
  expect(small.runModelIds.length).toBeGreaterThan(0) // small ran the actual turn
  expect(small.runModelIds.every((id) => id === 'small')).toBe(true)
})

test('switching to a same-or-larger window model does NOT compact (no unnecessary compaction)', async () => {
  const a = new RecordingProvider('a')
  const b = new RecordingProvider('b')
  const modelA: ModelSelection = { ...model, model: { ...model.model, id: 'a', contextWindow: 100_000 } }
  const modelB: ModelSelection = { ...model, model: { ...model.model, id: 'b', contextWindow: 100_000 } }
  const session = createSession(a, createRuntime(), undefined, modelA)

  await session.send(text('turn one'))
  await session.send(text('turn two'))

  session.updateModel(b, modelB)
  await session.send(text('after switch'))

  // The new model fits the history, so no compaction boundary is created.
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(b.runModelIds).toEqual(['b'])
})

test('AgentSession send writes user turn before provider request and records response', async () => {
  const requests: InferenceRequest[] = []
  const provider = new StubProvider([
    (request) => {
      requests.push(request)
      expect(request.systemPrompt).toBe('system prompt')
      expect(request.cwd).toBe('/workspace')
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, ...text('hello')] },
      ])
      return [events.text('hi'), events.response({ outputTokens: 2 })]
    },
  ])
  const session = createSession(provider)
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  await session.send(text('hello'))

  expect(requests).toHaveLength(1)
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  expect(session.phase()).toBe('idle')
  expect(emitted.map((event) => event.type)).toContain('phase_changed')
  expect(emitted.map((event) => event.type)).toContain('transcript_changed')
})

test('AgentSession rejects, emits, and records provider error events', async () => {
  const provider = new StubProvider([[events.error('provider failed', 'auth')]])
  const session = createSession(provider)
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  await expect(session.send(text('hello'))).rejects.toThrow('provider failed')

  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'error'])
  expect(session.transcript().blocks[1]).toMatchObject({
    type: 'error',
    message: 'provider failed',
    code: 'auth',
  })
  expect(emitted.find((event) => event.type === 'error')).toMatchObject({
    type: 'error',
    error: expect.objectContaining({ name: 'ProviderStreamError', message: 'provider failed' }),
  })
})

test('AgentSession closes provider iterators when provider error events stop a turn early', async () => {
  const provider = new ClosingProvider()
  const session = createSession(provider)

  await expect(session.send(text('hello'))).rejects.toThrow('provider failed')

  expect(provider.returned).toBe(true)
})

test('AgentSession records provider iterator exceptions and continues queued sends', async () => {
  const provider = new ThrowingProvider()
  const session = createSession(provider)

  const first = session.send(text('first'))
  await provider.waitForPartial()

  const second = session.send(text('second'))
  expect(session.queuedMessages()).toMatchObject([{ text: 'second' }])

  provider.releaseThrow()
  await expect(first).rejects.toThrow('transport disconnected')
  await second

  expect(session.phase()).toBe('idle')
  expect(session.queuedMessages()).toEqual([])
  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'error',
    'user',
    'text',
    'response',
  ])
  expect(session.transcript().blocks[2]).toMatchObject({
    type: 'error',
    message: 'transport disconnected',
    code: 'TRANSPORT_CLOSED',
  })
  expect(provider.requests).toHaveLength(2)
})

test('AgentSession resolves references before writing user turns and provider requests', async () => {
  const provider = new StubProvider([
    (request) => {
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'expanded ref' }] },
      ])
      return [events.response()]
    },
  ])
  const runtime = createRuntime({
    resolveReferences: (_ctx, content) =>
      content.map((block) => (block.type === 'reference' ? { type: 'text', text: 'expanded ref' } : block)),
  })
  const session = createSession(provider, runtime)

  await session.send([{ type: 'reference', reference: 'file.txt' }])

  expect(session.transcript().blocks[0]).toMatchObject({
    type: 'user',
    content: [{ type: 'text', text: 'expanded ref' }],
  })
})

test('AgentSession abort is not blocked by reference resolution', async () => {
  const provider = new StubProvider([
    () => {
      throw new Error('provider should not run')
    },
  ])
  let markStarted!: () => void
  const started = new Promise<void>((resolve) => {
    markStarted = resolve
  })
  const runtime = createRuntime({
    resolveReferences: () => {
      markStarted()
      return new Promise<UserContentBlock[]>(() => {})
    },
  })
  const session = createSession(provider, runtime)
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  const sending = session.send([{ type: 'reference', reference: 'slow.txt' }])
  await started
  const aborted = await session.abort()
  await sending

  expect(aborted).toBe(true)
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['abort'])
  expect(emitted).toContainEqual({ type: 'phase_changed', phase: 'running' })
  expect(emitted).toContainEqual({ type: 'phase_changed', phase: 'idle' })
})

test('AgentSession abort is not blocked by a provider that does not yield', async () => {
  const provider = new HangingProvider()
  const session = createSession(provider)

  const sending = session.send(text('hang provider'))
  await provider.started.promise
  const aborted = await session.abort()
  await withTimeout(sending)

  expect(aborted).toBe(true)
  expect(provider.cancelled).toBe(true)
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'abort'])
})

test('AgentSession abort is not blocked by a hanging compaction summary provider', async () => {
  const provider = new HangingProvider()
  const transcript = new Transcript([], {
    idFactory: (() => {
      let id = 0
      return () => `seed-${++id}`
    })(),
    now: () => '2026-06-17T00:00:00.000Z',
  })
  transcript.pushUserTurn('test-turn', model, text('old'))
  transcript.applyProviderEvent(model, events.text('answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('recent'))
  const session = createSession(provider, createRuntime(), transcript)

  const compacting = session.compact()
  await provider.started.promise
  const aborted = await session.abort()
  await withTimeout(compacting)

  expect(aborted).toBe(true)
  expect(provider.cancelled).toBe(true)
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.at(-1)).toMatchObject({ type: 'abort' })
})

test('AgentSession persists transcript snapshots through an injected store', async () => {
  const store = new MemorySessionStore<{ toolCalls: number }>()
  const provider = new StubProvider([[events.text('persisted'), events.response()]])
  const session = createSession(provider, createRuntime(), undefined, model, { store })

  await session.send(text('save this'))

  expect(store.snapshots.length).toBeGreaterThan(0)
  expect(store.snapshots.at(-1)?.harnessName).toBe('test-agent')
  expect(store.snapshots.at(-1)?.cwd).toBe('/workspace')
  expect(store.snapshots.at(-1)?.transcript.blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

test('AgentSession store snapshots are insulated from later state mutations', async () => {
  const store = new MemorySessionStore<{ toolCalls: number }>()
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'count_tool', {}), events.response()],
    [events.text('done'), events.response()],
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'count_tool',
        description: 'Mutates agent state.',
        inputSchema: { type: 'object' },
        invoke: (ctx) => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: 'counted' }] }
        },
      },
    ],
  })
  const session = createSession(provider, runtime, undefined, model, { store })

  await session.send(text('count once'))

  expect(store.snapshots[0]?.state).toEqual({ toolCalls: 0 })
  expect(store.snapshots.at(-1)?.state).toEqual({ toolCalls: 1 })

  if (!store.snapshots[0]) throw new Error('missing first snapshot')
  store.snapshots[0].state.toolCalls = 99
  expect(session.state()).toEqual({ toolCalls: 1 })
})

test('AgentSession persists extension state snapshots appended by lifecycle hooks', async () => {
  const store = new MemorySessionStore<{ toolCalls: number }>()
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'echo_tool', { value: 'hello' }), events.response()],
    [events.text('done'), events.response()],
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'echo_tool',
        description: 'Echoes a value.',
        inputSchema: { type: 'object' },
        invoke: (ctx, input) => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: JSON.stringify(input) }] }
        },
      },
    ],
    lifecycle: (event) => {
      if (event.type !== 'after_tool_call') return
      event.transcript.appendExtensionStateSnapshot('test-extension', { toolCalls: event.state.toolCalls })
    },
  })
  const session = createSession(provider, runtime, undefined, model, { store })

  await session.send(text('use tool'))

  const snapshotsWithExtension = store.snapshots.filter((snapshot) => {
    return snapshot.transcript.blocks.some((block) => block.type === 'extension_state_snapshot')
  })
  expect(snapshotsWithExtension.length).toBeGreaterThan(0)
  expect(snapshotsWithExtension.at(-1)?.transcript.blocks).toContainEqual(
    expect.objectContaining({
      type: 'extension_state_snapshot',
      extensionName: 'test-extension',
      state: { toolCalls: 1 },
    }),
  )
})

test('AgentSession executes requested tools and continues provider roundtrip with tool result', async () => {
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'echo_tool', { value: 'hello' }), events.response()],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      return [events.text('done'), events.response()]
    },
  ])
  const runtime = createRuntime({
    tools: (ctx) => [
      {
        name: 'echo_tool',
        description: 'Echoes a value.',
        inputSchema: { type: 'object' },
        invoke: (_toolCtx, input) => {
          ctx.state.toolCalls += 1
          return {
            output: [{ type: 'text', text: JSON.stringify(input) }],
          }
        },
      },
    ],
  })
  const session = createSession(provider, runtime)

  await session.send(text('use tool'))

  expect(session.state().toolCalls).toBe(1)
  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'response',
    'text',
    'response',
  ])
  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

test('AgentSession continues when a provider pauses after a tool call without a response event', async () => {
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'echo_tool', { value: 'hello' })],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      expect(request.items[2]).toMatchObject({
        type: 'tool_result',
        toolUseId: 'tool-1',
        isError: false,
        output: [{ type: 'text', text: '{"value":"hello"}' }],
      })
      return [events.text('continued after paused tool call'), events.response()]
    },
  ])
  const runtime = createRuntime({
    tools: (ctx) => [
      {
        name: 'echo_tool',
        description: 'Echoes a value.',
        inputSchema: { type: 'object' },
        invoke: (_toolCtx, input) => {
          ctx.state.toolCalls += 1
          return {
            output: [{ type: 'text', text: JSON.stringify(input) }],
          }
        },
      },
    ],
  })
  const session = createSession(provider, runtime)

  await session.send(text('use tool'))

  expect(session.state().toolCalls).toBe(1)
  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'text',
    'response',
  ])
  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

test('AgentSession can stop a turn after a terminal tool result', async () => {
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'terminal_tool', { value: 'stop' }), events.response()],
    () => {
      throw new Error('provider should not be called after terminal tool result')
    },
  ])
  const runtime = createRuntime({
    tools: (ctx) => [
      {
        name: 'terminal_tool',
        description: 'Stops the current turn.',
        inputSchema: { type: 'object' },
        invoke: () => {
          ctx.state.toolCalls += 1
          return {
            output: [{ type: 'text', text: 'stop here' }],
            isError: true,
            stopAfterToolResult: true,
          }
        },
      },
    ],
  })
  const session = createSession(provider, runtime)

  await session.send(text('use terminal tool'))

  expect(session.state().toolCalls).toBe(1)
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'tool_call', 'response'])
  const toolBlock = session.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(toolBlock).toMatchObject({
    type: 'tool_call',
    status: 'error',
    output: [{ type: 'text', text: 'stop here' }],
  })
  expect(session.phase()).toBe('idle')
})

test('AgentSession records thrown tool invocations as error tool results and continues', async () => {
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'failing_tool', { value: 'hello' }), events.response()],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      expect(request.items[2]).toMatchObject({
        type: 'tool_result',
        toolUseId: 'tool-1',
        isError: true,
        output: [{ type: 'text', text: 'Tool failed: broken tool' }],
      })
      return [events.text('recovered'), events.response()]
    },
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'failing_tool',
        description: 'Throws.',
        inputSchema: { type: 'object' },
        invoke: () => {
          throw new Error('broken tool')
        },
      },
    ],
  })
  const session = createSession(provider, runtime)

  await session.send(text('use failing tool'))

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'response',
    'text',
    'response',
  ])
  expect(session.transcript().blocks[1]).toMatchObject({
    type: 'tool_call',
    toolUseId: 'tool-1',
    status: 'error',
    output: [{ type: 'text', text: 'Tool failed: broken tool' }],
    metadata: { error: 'broken tool' },
  })
  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

test('AgentSession emits tool_progress events from tool invocations', async () => {
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'progress_tool', { value: 'hello' }), events.response()],
    [events.response()],
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'progress_tool',
        description: 'Emits progress.',
        inputSchema: { type: 'object' },
        invoke: (ctx) => {
          ctx.emitProgress({ step: 'started' })
          return { output: [{ type: 'text', text: 'done' }] }
        },
      },
    ],
  })
  const session = createSession(provider, runtime)
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  await session.send(text('use progress tool'))

  expect(emitted).toContainEqual({
    type: 'tool_progress',
    toolCallId: 'tool-1',
    toolName: 'progress_tool',
    progress: { step: 'started' },
  })
})

test('AgentSession queues sends while a run is active and drains them in order', async () => {
  const provider = new GateProvider([
    [events.text('first'), events.response()],
    [events.text('second'), events.response()],
  ])
  const session = createSession(provider)

  const first = session.send(text('first'))
  await provider.waitForRun(0)

  const second = session.send(text('second'))
  expect(session.queuedMessages()).toMatchObject([{ text: 'second' }])

  provider.release(0)
  await provider.waitForRun(1)
  expect(session.queuedMessages()).toEqual([])

  provider.release(1)
  await Promise.all([first, second])

  const userTexts = session
    .transcript()
    .blocks.filter((block) => block.type === 'user')
    .map((block) => (block.type === 'user' && block.content[0]?.type === 'text' ? block.content[0].text : ''))
  expect(userTexts).toEqual(['first', 'second'])
})

test('AgentSession rejects steer while idle without changing the transcript', async () => {
  const session = createSession(new StubProvider([]))

  await expect(session.steer(text('too late'))).rejects.toThrow('no active turn to steer')

  expect(session.transcript().blocks).toEqual([])
  expect(session.queuedMessages()).toEqual([])
})

test('AgentSession delivers multiple steers to an active steerable provider run without queueing', async () => {
  const provider = new SteerableGateProvider([[events.text('done'), events.response()]])
  const session = createSession(provider)
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  const sending = session.send(text('start'))
  await provider.waitForRun(0)

  await session.steer(text('first steer'))
  await session.steer(text('second steer'))

  expect(provider.steers.map((steer) => steer.content)).toEqual([text('first steer'), text('second steer')])
  expect(provider.steers.map((steer) => steer.turnId)).toEqual(['id-1', 'id-1'])
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'steer', 'steer'])
  expect(
    session
      .transcript()
      .blocks.filter((block) => block.type === 'steer')
      .map((block) => (block.type === 'steer' ? block.turnId : '')),
  ).toEqual(['id-1', 'id-1'])
  expect(session.queuedMessages()).toEqual([])
  expect(emitted.some((event) => event.type === 'queue_changed')).toBe(false)

  provider.release(0)
  await sending

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'steer', 'steer', 'text', 'response'])
})

test('AgentSession rejects provider-stream steer when the active run lacks steer support', async () => {
  const provider = new GateProvider([[events.text('done'), events.response()]])
  const session = createSession(provider)

  const sending = session.send(text('start'))
  await provider.waitForRun(0)

  await expect(session.steer(text('unsupported'))).rejects.toThrow('does not support steering')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user'])

  provider.release(0)
  await sending
})

test('AgentSession does not append steer blocks when native provider steer rejects', async () => {
  const provider = new SteerableGateProvider([[events.text('done'), events.response()]], () => {
    throw new Error('steer rejected')
  })
  const session = createSession(provider)

  const sending = session.send(text('start'))
  await provider.waitForRun(0)

  await expect(session.steer(text('rejected'))).rejects.toThrow('steer rejected')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user'])

  provider.release(0)
  await sending

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

test('AgentSession abort after an accepted steer preserves steer history', async () => {
  const provider = new SteerableGateProvider([[events.text('unreachable'), events.response()]])
  const session = createSession(provider)

  const sending = session.send(text('start'))
  await provider.waitForRun(0)
  await session.steer(text('accepted before abort'))

  await session.abort()
  await sending

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'steer', 'abort'])
  expect(session.transcript().collectInferenceItems()).toEqual([
    { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, ...text('start')] },
    { type: 'user_steer', turnId: 'id-1', content: text('accepted before abort') },
  ])
})

test('AgentSession records tool-execution steer for the next provider continuation without queueing', async () => {
  const toolStarted = deferred<void>()
  const releaseTool = deferred<void>()
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'slow_tool', {}), events.response()],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result', 'user_steer'])
      expect(request.items[3]).toEqual({
        type: 'user_steer',
        turnId: 'id-1',
        content: text('while tool runs'),
      })
      return [events.text('continued'), events.response()]
    },
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'slow_tool',
        description: 'Waits for the test to release it.',
        inputSchema: {},
        invoke: async (): Promise<AgentToolInvokeResult> => {
          toolStarted.resolve(undefined)
          await releaseTool.promise
          return { output: [{ type: 'text', text: 'tool done' }] }
        },
      },
    ],
  })
  const session = createSession(provider, runtime)
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  const sending = session.send(text('use tool'))
  await toolStarted.promise

  await session.steer(text('while tool runs'))
  expect(session.queuedMessages()).toEqual([])
  expect(emitted.some((event) => event.type === 'queue_changed')).toBe(false)

  releaseTool.resolve(undefined)
  await sending

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'response',
    'steer',
    'text',
    'response',
  ])
})

test('AgentSession retry truncates the last assistant response and reruns the latest user turn', async () => {
  const provider = new StubProvider([
    [events.text('old'), events.response()],
    (request) => {
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, ...text('question')] },
      ])
      return [events.text('new'), events.response()]
    },
  ])
  const session = createSession(provider)

  await session.send(text('question'))
  await session.retry()

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  expect(session.transcript().blocks[1]).toMatchObject({ type: 'text', text: 'new' })
})

test('AgentSession retry preserves accepted steers for the retried turn', async () => {
  const transcript = new Transcript([], {
    idFactory: (() => {
      let id = 0
      return () => `seed-${++id}`
    })(),
    now: () => '2026-06-17T00:00:00.000Z',
  })
  transcript.pushUserTurn('turn-a', model, text('question'), 'preamble')
  transcript.pushSteer('turn-a', model, text('extra constraint'))
  transcript.applyProviderEvent(model, events.text('old'))
  transcript.applyProviderEvent(model, events.response())
  const provider = new StubProvider([
    (request) => {
      expect(request.turnId).toBe('turn-a')
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, ...text('question')] },
        { type: 'user_steer', turnId: 'turn-a', content: text('extra constraint') },
      ])
      return [events.text('new'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript)

  await session.retry()

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'steer', 'text', 'response'])
  expect(session.transcript().blocks[1]).toMatchObject({
    type: 'steer',
    turnId: 'turn-a',
    content: text('extra constraint'),
  })
  expect(session.transcript().blocks[2]).toMatchObject({ type: 'text', text: 'new' })
})

test('AgentSession resume marks abort as resumed and adds a resume turn', async () => {
  const provider = new StubProvider([[events.text('continued'), events.response()]])
  const transcript = new Transcript([], {
    idFactory: () => 'seed-id',
    now: () => '2026-06-17T00:00:00.000Z',
  })
  transcript.pushUserTurn('test-turn', model, text('question'))
  transcript.pushAbort(model)
  const session = createSession(provider, createRuntime(), transcript)

  await session.resume()

  expect(session.transcript().blocks[1]).toMatchObject({ type: 'abort', isResumed: true })
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'abort', 'resume', 'text', 'response'])
})

test('AgentSession resume replays accepted steers from the aborted turn', async () => {
  const provider = new StubProvider([
    (request) => {
      expect(request.items).toEqual([
        { type: 'user_message', content: text('question') },
        { type: 'user_steer', turnId: 'turn-a', content: text('extra constraint') },
        { type: 'user_message', content: [{ type: 'text', text: 'Continue from where you left off.' }] },
      ])
      return [events.text('continued'), events.response()]
    },
  ])
  const transcript = new Transcript([], {
    idFactory: () => 'seed-id',
    now: () => '2026-06-17T00:00:00.000Z',
  })
  transcript.pushUserTurn('turn-a', model, text('question'))
  transcript.pushSteer('turn-a', model, text('extra constraint'))
  transcript.pushAbort(model)
  const session = createSession(provider, createRuntime(), transcript)

  await session.resume()

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'steer',
    'abort',
    'resume',
    'text',
    'response',
  ])
})

test('AgentSession abort is not blocked by a long-running tool invocation', async () => {
  const provider = new StubProvider([[events.toolCall('tool-1', 'slow_tool', {}), events.response()]])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'slow_tool',
        description: 'Never finishes unless aborted.',
        inputSchema: {},
        invoke: async (): Promise<AgentToolInvokeResult> => {
          await new Promise(() => {})
          return { output: [{ type: 'text', text: 'unreachable' }] }
        },
      },
    ],
  })
  const session = createSession(provider, runtime)
  const running = session.send(text('run slow tool'))
  await waitFor(() => session.transcript().pendingToolCalls().length === 1)

  const aborted = await session.abort()
  await running

  expect(aborted).toBe(true)
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.at(-1)).toMatchObject({ type: 'abort' })
})

test('AgentSession completes pending tool calls as errors before resuming after abort', async () => {
  const provider = new StubProvider([
    [events.toolCall('tool-1', 'slow_tool', { value: 'pending' }), events.response()],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result', 'user_message'])
      const toolResult = request.items.find((item) => item.type === 'tool_result')
      expect(toolResult).toMatchObject({
        type: 'tool_result',
        toolUseId: 'tool-1',
        isError: true,
        output: [{ type: 'text', text: 'Tool call aborted: slow_tool' }],
      })
      return [events.text('continued'), events.response()]
    },
  ])
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'slow_tool',
        description: 'Never finishes unless aborted.',
        inputSchema: {},
        invoke: async (): Promise<AgentToolInvokeResult> => {
          await new Promise(() => {})
          return { output: [{ type: 'text', text: 'unreachable' }] }
        },
      },
    ],
  })
  const session = createSession(provider, runtime)

  const running = session.send(text('run slow tool'))
  await waitFor(() => session.transcript().pendingToolCalls().length === 1)
  await session.abort()
  await running

  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
  expect(session.transcript().blocks).toContainEqual(
    expect.objectContaining({
      type: 'tool_call',
      toolUseId: 'tool-1',
      status: 'error',
      output: [{ type: 'text', text: 'Tool call aborted: slow_tool' }],
    }),
  )

  await session.resume()

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'tool_call',
    'response',
    'abort',
    'resume',
    'text',
    'response',
  ])
})

test('AgentSession compact inserts boundary and marker without deleting old blocks', async () => {
  const provider = new StubProvider([[events.text('summary'), events.response()]])
  const transcript = new Transcript([], {
    idFactory: (() => {
      let id = 0
      return () => `seed-${++id}`
    })(),
    now: () => '2026-06-17T00:00:00.000Z',
  })
  transcript.pushUserTurn('test-turn', model, text('old'))
  transcript.applyProviderEvent(model, events.text('answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('recent'))
  const session = createSession(provider, createRuntime(), transcript)

  await session.compact()

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'compaction_boundary',
    'user',
    'compaction_marker',
  ])
  expect(session.transcript().collectInferenceItems()[0]).toEqual({
    type: 'user_message',
    content: [{ type: 'text', text: 'Previous conversation summary:\nsummary' }],
  })
})

test('AgentSession auto-recovers by compacting and resuming when usage nears context limit', async () => {
  const smallModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 10 },
  }
  const provider = new StubProvider([
    [events.text('answer'), events.response({ inputTokens: 8 })],
    [events.text('summary'), events.response()],
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\nsummary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'Continue from where you left off.' }] },
      ])
      return [events.text('continued'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), undefined, smallModel)

  await session.send(text('question'))

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'compaction_boundary',
    'compaction_marker',
    'resume',
    'text',
    'response',
  ])
})

test('AgentSession mutation guard rejects reservations while busy or already reserved', async () => {
  const provider = new GateProvider([[events.text('ok'), events.response()]])
  const session = createSession(provider)

  const firstReservation = session.reserveMutation()
  expect(() => session.reserveMutation()).toThrow('cannot reserve mutation')
  firstReservation.release()

  const running = session.send(text('hello'))
  await provider.waitForRun(0)
  expect(() => session.reserveMutation()).toThrow('cannot reserve mutation')

  provider.release(0)
  await running
})

test('AgentSession mutation guard rejects queued actions while an external mutation is reserved', async () => {
  const provider = new StubProvider([[events.text('ok'), events.response()]])
  const session = createSession(provider)
  const emitted: SessionEvent[] = []
  session.subscribe((event) => emitted.push(event))

  const reservation = session.reserveMutation()
  await expect(session.send(text('blocked'))).rejects.toThrow('external mutation is reserved')
  await expect(session.retry()).rejects.toThrow('external mutation is reserved')
  await expect(session.resume()).rejects.toThrow('external mutation is reserved')
  await expect(session.compact()).rejects.toThrow('external mutation is reserved')

  expect(session.phase()).toBe('idle')
  expect(session.queuedMessages()).toEqual([])
  expect(emitted).toEqual([])

  reservation.release()
  await session.send(text('allowed'))

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
})

class GateProvider implements AgentProvider {
  private readonly turns: ProviderEvent[][]
  private readonly gates: Array<Deferred<void>>
  private readonly started = new Map<number, Deferred<void>>()
  readonly requests: InferenceRequest[] = []

  constructor(turns: ProviderEvent[][]) {
    this.turns = turns
    this.gates = turns.map(() => deferred<void>())
  }

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const index = this.requests.length
    this.requests.push(request)
    this.started.get(index)?.resolve()
    await this.gates[index].promise
    for (const event of this.turns[index] ?? []) yield event
  }

  waitForRun(index: number): Promise<void> {
    if (this.requests.length > index) return Promise.resolve()
    const existing = this.started.get(index)
    if (existing) return existing.promise
    const next = deferred<void>()
    this.started.set(index, next)
    return next.promise
  }

  release(index: number): void {
    this.gates[index].resolve()
  }
}

class SteerableGateProvider implements AgentProvider {
  private readonly turns: ProviderEvent[][]
  private readonly gates: Array<Deferred<void>>
  private readonly started = new Map<number, Deferred<void>>()
  readonly requests: InferenceRequest[] = []
  readonly steers: InferenceSteer[] = []

  constructor(
    turns: ProviderEvent[][],
    private readonly steerImpl?: (input: InferenceSteer) => Promise<void> | void,
  ) {
    this.turns = turns
    this.gates = turns.map(() => deferred<void>())
  }

  run(request: InferenceRequest): ProviderRun {
    const index = this.requests.length
    this.requests.push(request)
    const gate = this.gates[index]
    const turn = this.turns[index] ?? []
    const output = async function* (): AsyncIterable<ProviderEvent> {
      await gate.promise
      for (const event of turn) yield event
    }
    this.started.get(index)?.resolve(undefined)
    return createProviderRun(output(), {
      steer: (input) => {
        if (this.steerImpl) return this.steerImpl(input)
        this.steers.push(input)
      },
    })
  }

  waitForRun(index: number): Promise<void> {
    if (this.requests.length > index) return Promise.resolve()
    const existing = this.started.get(index)
    if (existing) return existing.promise
    const next = deferred<void>()
    this.started.set(index, next)
    return next.promise
  }

  release(index: number): void {
    this.gates[index].resolve(undefined)
  }
}

class HangingProvider implements AgentProvider {
  readonly started = deferred<void>()
  cancelled = false

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    request.cancel.addEventListener(
      'abort',
      () => {
        this.cancelled = true
      },
      { once: true },
    )
    this.started.resolve()
    await new Promise<never>(() => {})
  }
}

class ClosingProvider implements AgentProvider {
  returned = false

  run(): AsyncIterable<ProviderEvent> {
    let index = 0
    return {
      [Symbol.asyncIterator]: () => ({
        next: async () => {
          index += 1
          if (index === 1) return { done: false, value: events.error('provider failed') }
          return { done: false, value: events.text('should be closed') }
        },
        return: async () => {
          this.returned = true
          return { done: true, value: undefined }
        },
      }),
    }
  }
}

class ThrowingProvider implements AgentProvider {
  readonly requests: InferenceRequest[] = []
  private readonly partial = deferred<void>()
  private readonly release = deferred<void>()

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    const index = this.requests.length
    this.requests.push(request)
    if (index === 0) {
      yield events.text('partial')
      this.partial.resolve()
      await this.release.promise
      const error = new Error('transport disconnected')
      Object.assign(error, { code: 'TRANSPORT_CLOSED' })
      throw error
    }

    yield events.text('second ok')
    yield events.response()
  }

  waitForPartial(): Promise<void> {
    return this.partial.promise
  }

  releaseThrow(): void {
    this.release.resolve()
  }
}

class MemorySessionStore<State> {
  readonly snapshots: Array<AgentSessionSnapshot<State>> = []

  saveSnapshot(snapshot: AgentSessionSnapshot<State>): void {
    this.snapshots.push(snapshot)
  }
}

interface Deferred<T> {
  promise: Promise<T>
  resolve(value: T): void
  reject(error: unknown): void
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void
  let reject!: (error: unknown) => void
  const promise = new Promise<T>((res, rej) => {
    resolve = res
    reject = rej
  })
  return { promise, resolve, reject }
}

async function waitFor(predicate: () => boolean): Promise<void> {
  const startedAt = Date.now()
  while (!predicate()) {
    if (Date.now() - startedAt > 1_000) throw new Error('Timed out waiting for predicate')
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
}

function withTimeout<T>(promise: Promise<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error('Timed out waiting for promise')), 500)
    promise.then(
      (value) => {
        clearTimeout(timeout)
        resolve(value)
      },
      (error) => {
        clearTimeout(timeout)
        reject(error)
      },
    )
  })
}

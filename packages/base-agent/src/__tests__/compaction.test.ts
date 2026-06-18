import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { events, type AgentProvider, type InferenceRequest, type ProviderEvent } from '@demi/provider'
import { Transcript } from '../index'
import {
  assertNoOrphanToolItems,
  assertTranscriptInvariants,
  createDefinition,
  createSession,
  makeTranscript,
  MemorySessionStore,
  model,
  RecordingProvider,
  text,
} from './helpers'

test('preflight compaction summarizes before the model request and keeps the incoming user once', async () => {
  const thinkingModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
    thinking: { type: 'effort', effort: 'medium', summary: null },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn(thinkingModel, text('old question'))
  transcript.applyProviderEvent(thinkingModel, events.text('old answer'))
  transcript.applyProviderEvent(thinkingModel, events.response())

  const provider = new RecordingProvider([
    (request) => {
      expect(request.modelId).toBe('test-model')
      expect(request.cwd).toBe('/workspace')
      expect(request.thinking).toEqual({ type: 'effort', effort: 'medium', summary: null })
      expect(request.tools).toEqual([])
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'assistant_text'])
      return [events.text('old summary'), events.response()]
    },
    (request) => {
      expect(request.systemPrompt).toBe('system prompt')
      expect(request.tools.map((tool) => tool.name)).toEqual(['noop_tool'])
      const incomingUserMessages = request.items.filter((item) => {
        return (
          item.type === 'user_message' &&
          item.content.some((block) => block.type === 'text' && block.text === 'new question')
        )
      })
      expect(incomingUserMessages).toHaveLength(1)
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\nold summary' }],
        },
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'new question' }],
        },
      ])
      return [events.text('new answer'), events.response()]
    },
  ])
  const definition = createDefinition({
    tools: () => [
      {
        name: 'noop_tool',
        description: 'No-op.',
        inputSchema: { type: 'object' },
        invoke: () => ({ output: [{ type: 'text', text: 'ok' }] }),
      },
    ],
  })
  const session = createSession(provider, definition, transcript, thinkingModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.01 },
  })

  await session.send(text('new question'))

  expect(provider.requests).toHaveLength(2)
  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'compaction_boundary',
    'user',
    'compaction_marker',
    'text',
    'response',
  ])
  assertTranscriptInvariants(session.transcript().blocks)
})

test('compaction summary provider errors do not leave boundary or marker blocks', async () => {
  const transcript = oldAndRecentTranscript()
  const before = transcript.snapshot()
  const provider = new RecordingProvider([[events.error('summary failed', 'rate_limit')]])
  const session = createSession(provider, createDefinition(), transcript)

  await expect(session.compact()).rejects.toThrow('summary failed')

  expect(session.phase()).toBe('idle')
  expect(session.transcript().snapshot()).toEqual(before)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
})

test('empty compaction summaries are no-op and keep the session usable', async () => {
  const transcript = oldAndRecentTranscript()
  const provider = new RecordingProvider([
    [events.response()],
    [events.text('after empty summary'), events.response()],
  ])
  const session = createSession(provider, createDefinition(), transcript)

  await session.compact()
  await session.send(text('follow up'))

  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after empty summary' })
})

test('compaction summary input keeps completed tool_use and tool_result paired', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('inspect with tool'))
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'read_file', { path: 'a.txt' }))
  transcript.completeToolCall('tool-1', [{ type: 'text', text: 'file content' }])
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('recent question'))

  const provider = new RecordingProvider([
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      assertNoOrphanToolItems(request.items)
      return [events.text('tool summary'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)

  await session.compact()

  expect(session.transcript().collectInferenceItems()).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'Previous conversation summary:\ntool summary' }],
    },
    { type: 'user_message', content: [{ type: 'text', text: 'recent question' }] },
  ])
})

test('multiple compactions replay only the latest boundary summary', () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('first question'))
  transcript.applyProviderEvent(model, events.text('first answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('second question'))
  transcript.applyProviderEvent(model, events.text('second answer'))
  transcript.applyProviderEvent(model, events.response())

  const firstBoundary = transcript.insertCompactionBoundary(3, model, 'first summary', 3)
  transcript.appendCompactionMarker(model, firstBoundary.id, 30)

  transcript.pushUserTurn(model, text('third question'))
  transcript.applyProviderEvent(model, events.text('third answer'))
  transcript.applyProviderEvent(model, events.response())
  const thirdQuestionIndex = transcript.blocks.findIndex((block) => {
    return block.type === 'user' && block.content[0]?.type === 'text' && block.content[0].text === 'third question'
  })
  const secondBoundary = transcript.insertCompactionBoundary(thirdQuestionIndex, model, 'second summary', 4)
  transcript.appendCompactionMarker(model, secondBoundary.id, 50)

  const items = transcript.collectInferenceItems()

  expect(items).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'Previous conversation summary:\nsecond summary' }],
    },
    { type: 'user_message', content: [{ type: 'text', text: 'third question' }] },
    { type: 'assistant_text', modelId: 'test-model', text: 'third answer' },
  ])
  expect(JSON.stringify(items)).not.toContain('first summary')
})

test('manual compaction after an existing boundary summarizes only the latest replay window', async () => {
  const transcript = oldAndRecentTranscript()
  const provider = new RecordingProvider([
    [events.text('summary one'), events.response()],
    [events.text('after answer'), events.response()],
    (request) => {
      expect(JSON.stringify(request.items)).toContain('summary one')
      expect(JSON.stringify(request.items)).toContain('recent question')
      expect(JSON.stringify(request.items)).toContain('after compact')
      expect(JSON.stringify(request.items)).not.toContain('old question')
      return [events.text('summary two'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)

  await session.compact()
  await session.send(text('after compact'))
  await session.compact()

  const boundaries = session.transcript().blocks.filter((block) => block.type === 'compaction_boundary')
  expect(boundaries).toHaveLength(2)
  expect(session.transcript().collectInferenceItems()).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'Previous conversation summary:\nsummary two' }],
    },
  ])
})

test('single oversized turn can be compacted at a block boundary without orphaning tool history', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('single turn'))
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'read_file', { path: 'large.txt' }))
  transcript.completeToolCall('tool-1', [{ type: 'text', text: 'large output '.repeat(200) }])
  transcript.applyProviderEvent(model, events.text('recent assistant text'))

  const provider = new RecordingProvider([
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      assertNoOrphanToolItems(request.items)
      return [events.text('single turn summary'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript, model, {
    compaction: { keepRecentTokens: 2 },
  })

  await session.compact()

  expect(session.transcript().collectInferenceItems()).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'Previous conversation summary:\nsingle turn summary' }],
    },
    { type: 'assistant_text', modelId: 'test-model', text: 'recent assistant text' },
  ])
})

test('compaction is a no-op while a tool call is still pending', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('start tool'))
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'slow_tool', {}))
  const provider = new RecordingProvider([])
  const session = createSession(provider, createDefinition(), transcript)

  await session.compact()

  expect(provider.requests).toHaveLength(0)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().pendingToolCalls()).toHaveLength(1)
})

test('manual compaction with no compressible history is a no-op', async () => {
  const provider = new RecordingProvider([])
  const session = createSession(provider)

  await session.compact()

  expect(provider.requests).toHaveLength(0)
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks).toEqual([])
})

test('queued send during compaction drains after the summary commits', async () => {
  const transcript = oldAndRecentTranscript()
  const provider = new CompactGateProvider()
  const session = createSession(provider, createDefinition(), transcript)

  const compacting = session.compact()
  await provider.summaryStarted.promise
  const queued = session.send(text('queued question'))

  expect(session.phase()).toBe('compacting')
  expect(session.queuedMessages()).toMatchObject([{ text: 'queued question' }])

  provider.summaryRelease.resolve(undefined)
  await Promise.all([compacting, queued])

  expect(provider.requests).toHaveLength(2)
  expect(provider.requests[1]?.items).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'Previous conversation summary:\ngated summary' }],
    },
    { type: 'user_message', content: [{ type: 'text', text: 'recent question' }] },
    { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'queued question' }] },
  ])
  expect(session.phase()).toBe('idle')
  expect(session.queuedMessages()).toEqual([])
})

test('auto compaction after a tool result resumes without re-executing the tool', async () => {
  const smallModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 10 },
  }
  const provider = new RecordingProvider([
    [events.toolCall('tool-1', 'count_tool', { value: 1 }), events.response({ inputTokens: 9 })],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      assertNoOrphanToolItems(request.items)
      return [events.text('tool summary'), events.response()]
    },
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\ntool summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'Continue from where you left off.' }] },
      ])
      return [events.text('continued'), events.response()]
    },
  ])
  const definition = createDefinition({
    tools: (ctx) => [
      {
        name: 'count_tool',
        description: 'Counts calls.',
        inputSchema: { type: 'object' },
        invoke: () => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: 'counted' }] }
        },
      },
    ],
  })
  const session = createSession(provider, definition, undefined, smallModel)

  await session.send(text('use tool'))

  expect(session.state().toolCalls).toBe(1)
  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

test('compaction boundary and marker survive snapshot reconstruction', async () => {
  const store = new MemorySessionStore()
  const transcript = oldAndRecentTranscript()
  const provider = new RecordingProvider([[events.text('persisted summary'), events.response()]])
  const session = createSession(provider, createDefinition(), transcript, model, { store })

  await session.compact()

  const snapshot = store.snapshots.at(-1)
  expect(snapshot).toBeDefined()
  expect(snapshot?.transcript.blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)
  expect(snapshot?.transcript.blocks.some((block) => block.type === 'compaction_marker')).toBe(true)

  const restoredTranscript = new Transcript(snapshot?.transcript.blocks)
  const restoredProvider = new RecordingProvider([
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\npersisted summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'recent question' }] },
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'after restore' }],
        },
      ])
      return [events.text('restored ok'), events.response()]
    },
  ])
  const restored = createSession(restoredProvider, createDefinition(), restoredTranscript)

  await restored.send(text('after restore'))

  expect(restoredProvider.requests).toHaveLength(1)
})

function oldAndRecentTranscript(): Transcript {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('recent question'))
  return transcript
}

class CompactGateProvider implements AgentProvider {
  readonly requests: InferenceRequest[] = []
  readonly summaryStarted = deferred<void>()
  readonly summaryRelease = deferred<void>()

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    this.requests.push(request)
    if (request.tools.length === 0) {
      this.summaryStarted.resolve(undefined)
      await this.summaryRelease.promise
      yield events.text('gated summary')
      yield events.response()
      return
    }
    yield events.text('queued answer')
    yield events.response()
  }
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve
  })
  return { promise, resolve }
}

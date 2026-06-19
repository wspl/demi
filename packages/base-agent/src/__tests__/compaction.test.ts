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

test('retry triggers preflight compaction before rerunning the latest user', async () => {
  const preflightModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`old answer ${'y'.repeat(300)}`))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('retry this'))
  transcript.applyProviderEvent(model, events.text('bad answer'))
  transcript.applyProviderEvent(model, events.response())

  const provider = new RecordingProvider([
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: `old question ${'x'.repeat(200)}` }] },
        { type: 'assistant_text', modelId: 'test-model', text: `old answer ${'y'.repeat(300)}` },
      ])
      return [events.text('retry summary'), events.response()]
    },
    (request) => {
      expect(request.systemPrompt).toBe('system prompt')
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\nretry summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'retry this' }] },
      ])
      return [events.text('retried answer'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition({ preamble: () => null }), transcript, preflightModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.5 },
  })

  await session.retry()

  expect(provider.requests).toHaveLength(2)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'retried answer' })
  assertTranscriptInvariants(session.transcript().blocks)
})

test('resume triggers preflight compaction before continuing an aborted long context', async () => {
  const preflightModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`partial answer ${'y'.repeat(300)}`))
  transcript.pushAbort(model)

  const provider = new RecordingProvider([
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: `old question ${'x'.repeat(200)}` }] },
        { type: 'assistant_text', modelId: 'test-model', text: `partial answer ${'y'.repeat(300)}` },
      ])
      return [events.text('resume summary'), events.response()]
    },
    (request) => {
      expect(request.systemPrompt).toBe('system prompt')
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\nresume summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'Continue from where you left off.' }] },
      ])
      return [events.text('continued after compact'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition({ preamble: () => null }), transcript, preflightModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.5 },
  })

  await session.resume()

  expect(provider.requests).toHaveLength(2)
  expect(session.transcript().blocks.some((block) => block.type === 'abort' && block.isResumed)).toBe(true)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'continued after compact' })
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
  expect(provider.requests).toHaveLength(1)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
})

test('aborting a hanging compaction summary does not leave boundary or marker blocks', async () => {
  const transcript = oldAndRecentTranscript()
  const before = transcript.snapshot()
  const provider = new HangingSummaryProvider()
  const session = createSession(provider, createDefinition(), transcript)

  const compacting = session.compact()
  await provider.summaryStarted.promise
  const aborted = await session.abort()
  await withTimeout(compacting)

  expect(aborted).toBe(true)
  await provider.cancelled.promise
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.slice(0, before.blocks.length)).toEqual(before.blocks)
  expect(session.transcript().blocks.at(-1)).toMatchObject({ type: 'abort' })
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
  assertTranscriptInvariants(session.transcript().blocks)
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

test('compaction summary input keeps aborted text progress', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('long task'))
  transcript.applyProviderEvent(model, events.text('partial progress before abort'))
  transcript.pushAbort(model)
  transcript.pushUserTurn(model, text('recent question'))

  const provider = new RecordingProvider([
    (request) => {
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: 'long task' }] },
        { type: 'assistant_text', modelId: 'test-model', text: 'partial progress before abort' },
      ])
      return [events.text('aborted progress summary'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)

  await session.compact()

  expect(session.transcript().collectInferenceItems()[0]).toEqual({
    type: 'user_message',
    content: [{ type: 'text', text: 'Previous conversation summary:\naborted progress summary' }],
  })
})

test('compaction summary context overflow errors are atomic and classified when no smaller slice is available', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text(`old question ${'x'.repeat(200)}`))
  transcript.pushUserTurn(model, text('recent question'))
  const before = transcript.snapshot()
  const provider = new RecordingProvider([
    [events.error('summary context overflow', 'context_length_exceeded')],
    (request) => {
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: `old question ${'x'.repeat(200)}` }] },
        { type: 'user_message', content: [{ type: 'text', text: 'recent question' }] },
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'recover after overflow' }],
        },
      ])
      return [events.text('recovered'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)
  const errors: Error[] = []
  session.subscribe((event) => {
    if (event.type === 'error') errors.push(event.error)
  })

  await expect(session.compact()).rejects.toThrow('summary context overflow')

  expect(session.transcript().snapshot()).toEqual(before)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(errors).toHaveLength(1)
  expect(errors[0]).toMatchObject({ message: 'summary context overflow', code: 'context_length_exceeded' })

  await session.send(text('recover after overflow'))

  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'recovered' })
})

test('compaction summary context overflow retries with a smaller summary slice', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('middle question'))
  transcript.applyProviderEvent(model, events.text('middle answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('recent question'))

  const provider = new RecordingProvider([
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: 'old question' }] },
        { type: 'assistant_text', modelId: 'test-model', text: 'old answer' },
        { type: 'user_message', content: [{ type: 'text', text: 'middle question' }] },
        { type: 'assistant_text', modelId: 'test-model', text: 'middle answer' },
      ])
      return [events.error('summary context overflow', 'context_length_exceeded')]
    },
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      expect(request.items).toEqual([
        { type: 'user_message', content: [{ type: 'text', text: 'old question' }] },
        { type: 'assistant_text', modelId: 'test-model', text: 'old answer' },
      ])
      return [events.text('trimmed summary'), events.response()]
    },
    (request) => {
      expect(request.systemPrompt).toBe('system prompt')
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\ntrimmed summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'middle question' }] },
        { type: 'assistant_text', modelId: 'test-model', text: 'middle answer' },
        { type: 'user_message', content: [{ type: 'text', text: 'recent question' }] },
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'after trimmed compact' }],
        },
      ])
      return [events.text('after trimmed compact answer'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)

  await session.compact()
  await session.send(text('after trimmed compact'))

  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after trimmed compact answer' })
  assertTranscriptInvariants(session.transcript().blocks)
})

test('compaction summary input preserves thinking, redacted thinking, and tool metadata boundaries', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('inspect deeply'))
  transcript.applyProviderEvent(model, { type: 'thinking_start' })
  transcript.applyProviderEvent(model, { type: 'thinking_delta', text: 'private chain' })
  transcript.applyProviderEvent(model, { type: 'thinking_signature', signature: 'sig-1' })
  transcript.applyProviderEvent(model, { type: 'redacted_thinking', data: 'redacted-data' })
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'read_file', { path: 'a.txt' }))
  transcript.completeToolCall('tool-1', [{ type: 'text', text: 'file content' }], false, { bytes: 12 })
  transcript.appendExtensionStateSnapshot('todo', { internal: true })
  transcript.applyProviderEvent(model, events.text('recent visible text'))

  const provider = new RecordingProvider([
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual([
        'user_message',
        'assistant_thinking',
        'assistant_redacted_thinking',
        'tool_use',
        'tool_result',
      ])
      expect(request.items[1]).toMatchObject({ type: 'assistant_thinking', signature: 'sig-1' })
      assertNoOrphanToolItems(request.items)
      expect(JSON.stringify(request.items)).not.toContain('internal')
      return [events.text('complex summary'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript, model, {
    compaction: { keepRecentTokens: 2 },
  })

  await session.compact()

  expect(session.transcript().collectInferenceItems()).toEqual([
    {
      type: 'user_message',
      content: [{ type: 'text', text: 'Previous conversation summary:\ncomplex summary' }],
    },
    { type: 'assistant_text', modelId: 'test-model', text: 'recent visible text' },
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

test('aborting during preflight compaction stops before the model request and stays atomic', async () => {
  const smallModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = oldAndRecentTranscript()
  const provider = new HangingSummaryProvider()
  const session = createSession(provider, createDefinition(), transcript, smallModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.01 },
  })

  const sending = session.send(text('new question'))
  await provider.summaryStarted.promise
  const aborted = await session.abort()
  await withTimeout(sending)

  expect(aborted).toBe(true)
  await provider.cancelled.promise
  expect(provider.requests).toHaveLength(1)
  expect(provider.requests[0]?.systemPrompt).toContain('Summarize the previous conversation')
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'user',
    'user',
    'abort',
  ])
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
  assertTranscriptInvariants(session.transcript().blocks)
})

test('queued send during preflight compaction drains after the original send', async () => {
  const preflightModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`old answer ${'y'.repeat(300)}`))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('recent question'))

  const provider = new CompactGateProvider([
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\ngated summary' }],
        },
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'first question' }],
        },
      ])
      return [events.text('first answer'), events.response()]
    },
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\ngated summary' }],
        },
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'first question' }],
        },
        { type: 'assistant_text', modelId: 'test-model', text: 'first answer' },
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'second question' }],
        },
      ])
      return [events.text('second answer'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript, preflightModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.5 },
  })

  const first = session.send(text('first question'))
  await provider.summaryStarted.promise
  const second = session.send(text('second question'))

  expect(session.phase()).toBe('compacting')
  expect(session.queuedMessages()).toMatchObject([{ text: 'second question' }])

  provider.summaryRelease.resolve(undefined)
  await Promise.all([first, second])

  const summaryRequests = provider.requests.filter((request) => {
    return request.systemPrompt.includes('Summarize the previous conversation')
  })
  expect(summaryRequests).toHaveLength(1)
  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'second answer' })
  expect(session.queuedMessages()).toEqual([])
  assertTranscriptInvariants(session.transcript().blocks)
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

test('retry queued during compaction reruns the latest user after the summary commits', async () => {
  const transcript = oldAndRecentTranscript()
  const provider = new CompactGateProvider([
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\ngated summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'recent question' }] },
      ])
      return [events.text('retried after compact'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)

  const compacting = session.compact()
  await provider.summaryStarted.promise
  const retrying = session.retry()

  provider.summaryRelease.resolve(undefined)
  await Promise.all([compacting, retrying])

  expect(session.transcript().blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'compaction_boundary',
    'user',
    'text',
    'response',
  ])
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'retried after compact' })
})

test('resume queued during compaction continues from the compacted abort point', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushAbort(model)
  const provider = new CompactGateProvider([
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\ngated summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'Continue from where you left off.' }] },
      ])
      return [events.text('resumed after compact'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)

  const compacting = session.compact()
  await provider.summaryStarted.promise
  const resuming = session.resume()

  provider.summaryRelease.resolve(undefined)
  await Promise.all([compacting, resuming])

  expect(session.transcript().blocks.some((block) => block.type === 'abort' && block.isResumed)).toBe(true)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'resumed after compact' })
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
  private normalCursor = 0

  constructor(private readonly normalTurns: Array<(request: InferenceRequest) => ProviderEvent[]> = []) {}

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    this.requests.push(request)
    if (request.systemPrompt.includes('Summarize the previous conversation')) {
      this.summaryStarted.resolve(undefined)
      await this.summaryRelease.promise
      yield events.text('gated summary')
      yield events.response()
      return
    }
    const turn = this.normalTurns[this.normalCursor]
    this.normalCursor += 1
    if (turn) {
      for (const event of turn(request)) yield event
      return
    }
    yield events.text('queued answer')
    yield events.response()
  }
}

class HangingSummaryProvider implements AgentProvider {
  readonly requests: InferenceRequest[] = []
  readonly summaryStarted = deferred<void>()
  readonly cancelled = deferred<void>()

  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    this.requests.push(request)
    if (!request.systemPrompt.includes('Summarize the previous conversation')) {
      throw new Error('HangingSummaryProvider received a non-summary request')
    }
    this.summaryStarted.resolve(undefined)
    await new Promise<void>((resolve) => {
      request.cancel.addEventListener(
        'abort',
        () => {
          this.cancelled.resolve(undefined)
          resolve()
        },
        { once: true },
      )
    })
  }
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve
  })
  return { promise, resolve }
}

function withTimeout<T>(promise: Promise<T>, ms = 1_000): Promise<T> {
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => reject(new Error(`Timed out after ${ms}ms`)), ms)
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

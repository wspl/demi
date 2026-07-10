import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import type { ModelSelection } from '@demicodes/core'
import type { AgentProvider, InferenceRequest, ProviderEvent } from '@demicodes/provider'
import { events } from '@demicodes/provider/testing'
import { TranscriptLog } from '../index'
import {
  assertNoOrphanToolItems,
  assertTranscriptInvariants,
  createRuntime,
  createSession,
  makeTranscript,
  MemorySessionStore,
  model,
  RecordingProvider,
  type TestState,
  text,
} from './helpers'

// The compaction summary request is a single user turn containing the to-compact transcript as
// inert, delimited material (not a replayed conversation), with no thinking. This asserts that
// shape and returns the rendered transcript text so tests can check it carries the right material.
function summaryText(request: InferenceRequest): string {
  expect(request.tools).toEqual([])
  expect(request.thinking).toBeNull()
  expect(request.items).toHaveLength(1)
  const item = request.items[0]
  expect(item?.type).toBe('user_message')
  const rendered = item?.type === 'user_message' ? item.content.map((b) => (b.type === 'text' ? b.text : '')).join('') : ''
  expect(rendered).toContain('BEGIN TRANSCRIPT')
  return rendered
}

test('preflight compaction summarizes before the model request and keeps the incoming user once', async () => {
  const thinkingModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
    thinking: { type: 'effort', effort: 'medium', summary: null },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', thinkingModel, text('old question'))
  transcript.applyProviderEvent(thinkingModel, events.text('old answer'))
  transcript.applyProviderEvent(thinkingModel, events.response())

  const provider = new RecordingProvider([
    (request) => {
      expect(request.modelId).toBe('test-model')
      expect(request.cwd).toBe('/workspace')
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain('old question')
      expect(summary).toContain('old answer')
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
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'noop_tool',
        description: 'No-op.',
        inputSchema: { type: 'object' },
        invoke: () => ({ output: [{ type: 'text', text: 'ok' }] }),
      },
    ],
  })
  const session = createSession(provider, runtime, transcript, thinkingModel, {
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
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`old answer ${'y'.repeat(300)}`))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('retry this'))
  transcript.applyProviderEvent(model, events.text('bad answer'))
  transcript.applyProviderEvent(model, events.response())

  const provider = new RecordingProvider([
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain(`old question ${'x'.repeat(200)}`)
      expect(summary).toContain(`old answer ${'y'.repeat(300)}`)
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
  const session = createSession(provider, createRuntime({ preamble: () => null }), transcript, preflightModel, {
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
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`partial answer ${'y'.repeat(300)}`))
  transcript.pushAbort(model)

  const provider = new RecordingProvider([
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain(`old question ${'x'.repeat(200)}`)
      expect(summary).toContain(`partial answer ${'y'.repeat(300)}`)
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
  const session = createSession(provider, createRuntime({ preamble: () => null }), transcript, preflightModel, {
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
  const before = transcript.toJSON()
  const provider = new RecordingProvider([[events.error('summary failed', 'rate_limit')]])
  const session = createSession(provider, createRuntime(), transcript)

  await expect(session.compact()).rejects.toThrow('summary failed')

  expect(session.phase()).toBe('idle')
  expect(session.transcript().toJSON()).toEqual(before)
  expect(provider.requests).toHaveLength(1)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
})

test('aborting a hanging compaction summary does not leave boundary or marker blocks', async () => {
  const transcript = oldAndRecentTranscript()
  const before = transcript.toJSON()
  const provider = new HangingSummaryProvider()
  const session = createSession(provider, createRuntime(), transcript)

  const compacting = session.compact()
  await provider.summaryStarted.promise
  const aborted = await session.abort()
  await withTimeout(compacting)

  expect(aborted.aborted).toBe(true)
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
  const session = createSession(provider, createRuntime(), transcript)

  await session.compact()
  await session.send(text('follow up'))

  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after empty summary' })
})

test('compaction summary input keeps completed tool_use and tool_result paired', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text('inspect with tool'))
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'read_file', { path: 'a.txt' }))
  transcript.completeToolCall('tool-1', [{ type: 'text', text: 'file content' }])
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('recent question'))

  const provider = new RecordingProvider([
    (request) => {
      const summary = summaryText(request)
      expect(summary).toContain('file content') // the tool result is carried into the summary input
      return [events.text('tool summary'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript)

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
  transcript.pushUserTurn('test-turn', model, text('long task'))
  transcript.applyProviderEvent(model, events.text('partial progress before abort'))
  transcript.pushAbort(model)
  transcript.pushUserTurn('test-turn', model, text('recent question'))

  const provider = new RecordingProvider([
    (request) => {
      const summary = summaryText(request)
      expect(summary).toContain('long task')
      expect(summary).toContain('partial progress before abort')
      return [events.text('aborted progress summary'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript)

  await session.compact()

  expect(session.transcript().collectInferenceItems()[0]).toEqual({
    type: 'user_message',
    content: [{ type: 'text', text: 'Previous conversation summary:\naborted progress summary' }],
  })
})

test('compaction summary context overflow errors are atomic and classified when no smaller slice is available', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.pushUserTurn('test-turn', model, text('recent question'))
  const before = transcript.toJSON()
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
  const session = createSession(provider, createRuntime(), transcript)
  const errors: Error[] = []
  session.subscribe((event) => {
    if (event.type === 'error') errors.push(event.error)
  })

  await expect(session.compact()).rejects.toThrow('summary context overflow')

  expect(session.transcript().toJSON()).toEqual(before)
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
  transcript.pushUserTurn('test-turn', model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('middle question'))
  transcript.applyProviderEvent(model, events.text('middle answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('recent question'))

  const provider = new RecordingProvider([
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain('old question')
      expect(summary).toContain('middle answer')
      return [events.error('summary context overflow', 'context_length_exceeded')]
    },
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain('old question')
      expect(summary).not.toContain('middle question') // retried with a smaller slice
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
  const session = createSession(provider, createRuntime(), transcript)

  await session.compact()
  await session.send(text('after trimmed compact'))

  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after trimmed compact answer' })
  assertTranscriptInvariants(session.transcript().blocks)
})

test('compaction summary iterator context overflow retries with a smaller summary slice', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('middle question'))
  transcript.applyProviderEvent(model, events.text('middle answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('recent question'))

  const provider = new RecordingProvider([
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain('old question')
      expect(summary).toContain('middle answer')
      return throwingProviderError('iterator summary context overflow', 'context_length_exceeded')
    },
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain('old question')
      expect(summary).not.toContain('middle question') // retried with a smaller slice
      return [events.text('iterator trimmed summary'), events.response()]
    },
    (request) => {
      expect(request.items[0]).toEqual({
        type: 'user_message',
        content: [{ type: 'text', text: 'Previous conversation summary:\niterator trimmed summary' }],
      })
      return [events.text('after iterator compact'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript)

  await session.compact()
  await session.send(text('after iterator retry'))

  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after iterator compact' })
  assertTranscriptInvariants(session.transcript().blocks)
})

test('compaction summary input carries tools and visible text but omits thinking and internal state', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text('inspect deeply'))
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
      const summary = summaryText(request)
      expect(summary).toContain('inspect deeply') // user message
      expect(summary).toContain('read_file') // tool call
      expect(summary).toContain('file content') // tool result
      // private reasoning and internal extension state never enter the summary input
      expect(summary).not.toContain('private chain')
      expect(summary).not.toContain('internal')
      return [events.text('complex summary'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript, model, {
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
  transcript.pushUserTurn('test-turn', model, text('first question'))
  transcript.applyProviderEvent(model, events.text('first answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('second question'))
  transcript.applyProviderEvent(model, events.text('second answer'))
  transcript.applyProviderEvent(model, events.response())

  const firstBoundary = transcript.insertCompactionBoundary(3, model, 'first summary', 3)
  transcript.appendCompactionMarker(model, firstBoundary.id, 30)

  transcript.pushUserTurn('test-turn', model, text('third question'))
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
  const session = createSession(provider, createRuntime(), transcript)

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
  transcript.pushUserTurn('test-turn', model, text('single turn'))
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'read_file', { path: 'large.txt' }))
  transcript.completeToolCall('tool-1', [{ type: 'text', text: 'large output '.repeat(200) }])
  transcript.applyProviderEvent(model, events.text('recent assistant text'))

  const provider = new RecordingProvider([
    (request) => {
      const summary = summaryText(request)
      expect(summary).toContain('large output') // the oversized tool result is carried into the summary
      return [events.text('single turn summary'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript, model, {
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
  transcript.pushUserTurn('test-turn', model, text('start tool'))
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'slow_tool', {}))
  const provider = new RecordingProvider([])
  const session = createSession(provider, createRuntime(), transcript)

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
  const session = createSession(provider, createRuntime(), transcript, smallModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.01 },
  })

  const sending = session.send(text('new question'))
  await provider.summaryStarted.promise
  const aborted = await session.abort()
  await withTimeout(sending)

  expect(aborted.aborted).toBe(true)
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

test('aborting during retry preflight compaction stops before rerunning the model request', async () => {
  const preflightModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`old answer ${'y'.repeat(300)}`))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('retry this'))
  transcript.applyProviderEvent(model, events.text('bad answer'))
  transcript.applyProviderEvent(model, events.response())
  const provider = new HangingSummaryProvider()
  const session = createSession(provider, createRuntime(), transcript, preflightModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.5 },
  })

  const retrying = session.retry()
  await provider.summaryStarted.promise
  const aborted = await session.abort()
  await withTimeout(retrying)

  expect(aborted.aborted).toBe(true)
  await provider.cancelled.promise
  expect(provider.requests).toHaveLength(1)
  expect(provider.requests[0]?.systemPrompt).toContain('Summarize the previous conversation')
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'response', 'user', 'abort'])
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_marker')).toBe(false)
  assertTranscriptInvariants(session.transcript().blocks)
})

test('aborting during resume preflight compaction stops before continuing the model request', async () => {
  const preflightModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`partial answer ${'y'.repeat(300)}`))
  transcript.pushAbort(model)
  const provider = new HangingSummaryProvider()
  const session = createSession(provider, createRuntime(), transcript, preflightModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.5 },
  })

  const resuming = session.resume()
  await provider.summaryStarted.promise
  const aborted = await session.abort()
  await withTimeout(resuming)

  expect(aborted.aborted).toBe(true)
  await provider.cancelled.promise
  expect(provider.requests).toHaveLength(1)
  expect(provider.requests[0]?.systemPrompt).toContain('Summarize the previous conversation')
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'text', 'abort', 'resume', 'abort'])
  expect(session.transcript().blocks.some((block) => block.type === 'abort' && block.isResumed)).toBe(true)
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
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`old answer ${'y'.repeat(300)}`))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('recent question'))

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
  const session = createSession(provider, createRuntime(), transcript, preflightModel, {
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

test('retry queued during preflight compaction reruns the original send after it completes', async () => {
  const preflightModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`old answer ${'y'.repeat(300)}`))
  transcript.applyProviderEvent(model, events.response())

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
      ])
      return [events.text('retried answer'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript, preflightModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.5 },
  })

  const first = session.send(text('first question'))
  await provider.summaryStarted.promise
  const retrying = session.retry()

  provider.summaryRelease.resolve(undefined)
  await Promise.all([first, retrying])

  const summaryRequests = provider.requests.filter((request) => {
    return request.systemPrompt.includes('Summarize the previous conversation')
  })
  expect(summaryRequests).toHaveLength(1)
  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().blocks.some((block) => block.type === 'text' && block.text === 'first answer')).toBe(false)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'retried answer' })
  assertTranscriptInvariants(session.transcript().blocks)
})

test('resume queued during preflight compaction continues after the original send', async () => {
  const preflightModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 100 },
  }
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text(`old question ${'x'.repeat(200)}`))
  transcript.applyProviderEvent(model, events.text(`partial answer ${'y'.repeat(300)}`))
  transcript.pushAbort(model)

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
        { type: 'user_message', content: [{ type: 'text', text: 'Continue from where you left off.' }] },
      ])
      return [events.text('resumed answer'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript, preflightModel, {
    compaction: { keepRecentTokens: 1, preflightThresholdRatio: 0.5 },
  })

  const first = session.send(text('first question'))
  await provider.summaryStarted.promise
  const resuming = session.resume()

  provider.summaryRelease.resolve(undefined)
  await Promise.all([first, resuming])

  const summaryRequests = provider.requests.filter((request) => {
    return request.systemPrompt.includes('Summarize the previous conversation')
  })
  expect(summaryRequests).toHaveLength(1)
  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().blocks.some((block) => block.type === 'abort' && block.isResumed)).toBe(true)
  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'resumed answer' })
  assertTranscriptInvariants(session.transcript().blocks)
})

test('queued send during compaction drains after the summary commits', async () => {
  const transcript = oldAndRecentTranscript()
  const provider = new CompactGateProvider()
  const session = createSession(provider, createRuntime(), transcript)

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
  const session = createSession(provider, createRuntime(), transcript)

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
  transcript.pushUserTurn('test-turn', model, text('old question'))
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
  const session = createSession(provider, createRuntime(), transcript)

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
      const summary = summaryText(request)
      expect(summary).toContain('counted') // the tool result is carried into the summary input
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
  const runtime = createRuntime({
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
  const session = createSession(provider, runtime, undefined, smallModel)

  await session.send(text('use tool'))

  expect(session.state().toolCalls).toBe(1)
  expect(provider.requests).toHaveLength(3)
  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
})

test('auto compaction counts cache usage as context pressure', async () => {
  const smallModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 10 },
  }
  const provider = new RecordingProvider([
    [events.text('cached answer'), events.response({ inputTokens: 1, outputTokens: 1, cacheWriteTokens: 9 })],
    (request) => {
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      const summary = summaryText(request)
      expect(summary).toContain('cache-heavy question')
      expect(summary).toContain('cached answer')
      return [events.text('cache pressure summary'), events.response()]
    },
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\ncache pressure summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'Continue from where you left off.' }] },
      ])
      return [events.text('continued after cache pressure'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime({ preamble: () => null }), undefined, smallModel, {
    compaction: { keepRecentTokens: 1 },
  })

  await session.send(text('cache-heavy question'))

  expect(provider.requests).toHaveLength(3)
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
  assertTranscriptInvariants(session.transcript().blocks)
})

test('compaction boundary and marker survive snapshot reconstruction', async () => {
  const store = new MemorySessionStore<TestState>()
  const transcript = oldAndRecentTranscript()
  const provider = new RecordingProvider([[events.text('persisted summary'), events.response()]])
  const session = createSession(provider, createRuntime(), transcript, model, { store })

  await session.compact()

  const snapshot = store.snapshots.at(-1)
  expect(snapshot).toBeDefined()
  expect(snapshot?.transcript.blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)
  expect(snapshot?.transcript.blocks.some((block) => block.type === 'compaction_marker')).toBe(true)

  const restoredTranscript = new TranscriptLog(snapshot?.transcript.blocks)
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
  const restored = createSession(restoredProvider, createRuntime(), restoredTranscript)

  await restored.send(text('after restore'))

  expect(restoredProvider.requests).toHaveLength(1)
})

function oldAndRecentTranscript(): TranscriptLog {
  const transcript = makeTranscript()
  transcript.pushUserTurn('test-turn', model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn('test-turn', model, text('recent question'))
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

function throwingProviderError(message: string, code: string): AsyncIterable<ProviderEvent> {
  return (async function* (): AsyncIterable<ProviderEvent> {
    const error = new Error(message)
    Object.assign(error, { code })
    throw error
  })()
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

// A provider that always reports usage far over the limit (and returns a tiny summary for summary
// requests). With a misconfigured/too-low threshold the recent window stays "over limit" no matter
// what, which without a guard makes the turn compact forever.
class AlwaysOverLimitProvider implements AgentProvider {
  async *run(request: InferenceRequest): AsyncIterable<ProviderEvent> {
    if (request.systemPrompt.includes('Summarize the previous conversation')) {
      yield events.text('summary')
      yield events.response()
      return
    }
    yield events.text('answer')
    yield events.response({ inputTokens: 1_000_000 })
  }
}

test('auto compaction is bounded per turn — no storm when usage stays over a too-low threshold', async () => {
  const smallModel: ModelSelection = { ...model, model: { ...model.model, contextWindow: 10 } }
  const transcript = makeTranscript()
  for (let i = 0; i < 6; i += 1) {
    transcript.pushUserTurn('test-turn', model, text(`question ${i} ${'x'.repeat(120)}`))
    transcript.applyProviderEvent(model, events.text(`answer ${i} ${'y'.repeat(120)}`))
    transcript.applyProviderEvent(model, events.response())
  }
  const session = createSession(new AlwaysOverLimitProvider(), createRuntime(), transcript, smallModel, {
    compaction: { keepRecentTokens: 1 },
  })

  // Without the guard this never returns (it compacts its own summaries forever).
  await session.send(text('trigger'))

  // Bounded, not a storm: at most one preflight compaction plus the auto-recover cap of 3.
  // (Before the guard this looped ~40 times until the model rejected the fabricated history.)
  const boundaries = session.transcript().blocks.filter((block) => block.type === 'compaction_boundary')
  expect(boundaries.length).toBeGreaterThan(0)
  expect(boundaries.length).toBeLessThanOrEqual(4)
  assertTranscriptInvariants(session.transcript().blocks)
})

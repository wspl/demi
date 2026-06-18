import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { events } from '@demi/provider'
import { Transcript } from '../index'
import {
  createDefinition,
  createSession,
  makeTranscript,
  model,
  RecordingProvider,
  text,
} from './helpers'

test('provider request items preserve a stable prefix across ordinary turns', async () => {
  const provider = new RecordingProvider([
    [events.text('first answer'), events.response()],
    [events.text('second answer'), events.response()],
    [events.text('third answer'), events.response()],
  ])
  const definition = createDefinition({
    tools: () => [
      {
        name: 'stable_tool',
        description: 'Stable tool schema.',
        inputSchema: { type: 'object', properties: { value: { type: 'string' } } },
        invoke: () => ({ output: [{ type: 'text', text: 'ok' }] }),
      },
    ],
  })
  const session = createSession(provider, definition)

  await session.send(text('first'))
  await session.send(text('second'))
  await session.send(text('third'))

  expect(provider.requests).toHaveLength(3)
  expect(provider.requests[1]?.items.slice(0, provider.requests[0]?.items.length)).toEqual(provider.requests[0]?.items)
  expect(provider.requests[2]?.items.slice(0, provider.requests[1]?.items.length)).toEqual(provider.requests[1]?.items)
  expect(provider.requests.map((request) => request.systemPrompt)).toEqual(['system prompt', 'system prompt', 'system prompt'])
  expect(provider.requests.map((request) => request.tools)).toEqual([
    provider.requests[0]?.tools,
    provider.requests[0]?.tools,
    provider.requests[0]?.tools,
  ])
})

test('provider request is built from effective transcript without internal blocks', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  const boundary = transcript.insertCompactionBoundary(3, model, 'summary', 2)
  transcript.pushUserTurn(model, text('recent question'))
  transcript.appendCompactionMarker(model, boundary.id, 10)
  transcript.appendExtensionStateSnapshot('todo', { secret: 'internal state' })

  const provider = new RecordingProvider([
    (request) => {
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\nsummary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'recent question' }] },
        { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'new question' }] },
      ])
      expect(JSON.stringify(request.items)).not.toContain('old question')
      expect(JSON.stringify(request.items)).not.toContain('internal state')
      expect(JSON.stringify(request.items)).not.toContain('compactedTokens')
      return [events.text('answer'), events.response()]
    },
  ])
  const session = createSession(provider, createDefinition(), transcript)

  await session.send(text('new question'))
})

test('retry and resume provider requests match Transcript.collectInferenceItems', async () => {
  let session = createSession(new RecordingProvider([]))
  const provider = new RecordingProvider([
    [events.text('old answer'), events.response()],
    (request) => {
      expect(request.items).toEqual(session.transcript().collectInferenceItems())
      return [events.abort()]
    },
    (request) => {
      expect(request.items).toEqual(session.transcript().collectInferenceItems())
      return [events.text('resumed answer'), events.response()]
    },
  ])
  session = createSession(provider)

  await session.send(text('question'))
  await session.retry()
  await session.resume()

  expect(session.transcript().blocks.map((block) => block.type)).toEqual(['user', 'abort', 'resume', 'text', 'response'])
})

test('cache usage is recorded without changing tool loop or compaction behavior', async () => {
  const smallModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 10 },
  }
  const provider = new RecordingProvider([
    [
      events.toolCall('tool-1', 'cache_tool', { value: 1 }),
      events.response({ inputTokens: 1, outputTokens: 1, cacheReadTokens: 500, cacheWriteTokens: 300 }),
    ],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      return [events.text('done'), events.response({ cacheReadTokens: 700, cacheWriteTokens: 0 })]
    },
  ])
  const definition = createDefinition({
    tools: (ctx) => [
      {
        name: 'cache_tool',
        description: 'Checks cache transparency.',
        inputSchema: { type: 'object' },
        invoke: () => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: 'tool ok' }] }
        },
      },
    ],
  })
  const session = createSession(provider, definition, undefined, smallModel)

  await session.send(text('use tool'))

  expect(session.state().toolCalls).toBe(1)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  const responseBlocks = session.transcript().blocks.filter((block) => block.type === 'response')
  expect(responseBlocks[0]).toMatchObject({
    type: 'response',
    usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 500, cacheWriteTokens: 300 },
  })
  expect(responseBlocks[1]).toMatchObject({
    type: 'response',
    usage: { inputTokens: 0, outputTokens: 0, cacheReadTokens: 700, cacheWriteTokens: 0 },
  })
})

test('context overflow provider errors are explicit and keep the session recoverable', async () => {
  const provider = new RecordingProvider([
    [events.error('context window exceeded', 'context_length_exceeded')],
    [events.text('after overflow'), events.response()],
  ])
  const session = createSession(provider)

  await expect(session.send(text('too much context'))).rejects.toThrow('context window exceeded')
  expect(session.phase()).toBe('idle')
  expect(session.transcript().blocks.at(-1)).toMatchObject({
    type: 'error',
    message: 'context window exceeded',
    code: 'context_length_exceeded',
  })

  await session.send(text('recover'))

  expect(session.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'after overflow' })
})

test('snapshot reconstruction preserves model-visible context exactly', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('old'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  const boundary = transcript.insertCompactionBoundary(3, model, 'restored summary', 4)
  transcript.pushUserTurn(model, text('recent'))
  transcript.appendCompactionMarker(model, boundary.id, 12)

  const restored = new Transcript(transcript.snapshot().blocks)

  expect(restored.collectInferenceItems()).toEqual(transcript.collectInferenceItems())
})

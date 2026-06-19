import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demi/core'
import { events } from '@demi/provider/testing'
import { Transcript } from '../index'
import {
  createRuntime,
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
  const runtime = createRuntime({
    tools: () => [
      {
        name: 'stable_tool',
        description: 'Stable tool schema.',
        inputSchema: { type: 'object', properties: { value: { type: 'string' } } },
        invoke: () => ({ output: [{ type: 'text', text: 'ok' }] }),
      },
    ],
  })
  const session = createSession(provider, runtime)

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

test('provider request text content is bounded without mutating the transcript audit log', async () => {
  const longPreamble = `pre-${'p'.repeat(20_000)}-amble`
  const longReference = `<file path="big.txt">\nhead-${'r'.repeat(20_000)}-tail\n</file>`
  const longToolOutput = `tool-head-${'t'.repeat(20_000)}-tool-tail`
  const longAssistant = `assistant-head-${'a'.repeat(20_000)}-assistant-tail`
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, [{ type: 'text', text: longReference }], longPreamble)
  transcript.applyProviderEvent(model, events.toolCall('tool-1', 'read_file', { path: 'big.txt' }))
  transcript.completeToolCall('tool-1', [{ type: 'text', text: longToolOutput }])
  transcript.applyProviderEvent(model, events.text(longAssistant))

  const provider = new RecordingProvider([
    (request) => {
      const serialized = JSON.stringify(request.items)
      expect(serialized).toContain('[... truncated ')
      expect(serialized).toContain('pre-')
      expect(serialized).toContain('-amble')
      expect(serialized).toContain('head-')
      expect(serialized).toContain('-tail')
      expect(serialized).toContain('tool-head-')
      expect(serialized).toContain('-tool-tail')
      expect(serialized).toContain('assistant-head-')
      expect(serialized).toContain('-assistant-tail')
      expect(serialized).not.toContain('p'.repeat(18_000))
      expect(serialized).not.toContain('r'.repeat(18_000))
      expect(serialized).not.toContain('t'.repeat(18_000))
      expect(serialized).not.toContain('a'.repeat(18_000))
      return [events.text('bounded'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript)

  await session.send(text('continue'))

  const userBlock = session.transcript().blocks.find((block) => block.type === 'user')
  expect(userBlock).toMatchObject({ type: 'user', preamble: longPreamble })
  const toolBlock = session.transcript().blocks.find((block) => block.type === 'tool_call')
  expect(toolBlock).toMatchObject({
    type: 'tool_call',
    output: [{ type: 'text', text: longToolOutput }],
  })
  const textBlock = session.transcript().blocks.find((block) => block.type === 'text')
  expect(textBlock).toMatchObject({ type: 'text', text: longAssistant })
})

test('stable prompt prefix is byte-identical for equivalent histories and changes on cache inputs', async () => {
  const basePrefix = await prefixAfterTwoTurns({
    systemPrompt: 'system prompt',
    firstPreamble: 'preamble',
    toolDescription: 'Stable tool schema.',
    secondUser: 'second A',
  })
  const equivalentPrefix = await prefixAfterTwoTurns({
    systemPrompt: 'system prompt',
    firstPreamble: 'preamble',
    toolDescription: 'Stable tool schema.',
    secondUser: 'second B',
  })
  const changedSystem = await prefixAfterTwoTurns({
    systemPrompt: 'changed system',
    firstPreamble: 'preamble',
    toolDescription: 'Stable tool schema.',
    secondUser: 'second A',
  })
  const changedPreamble = await prefixAfterTwoTurns({
    systemPrompt: 'system prompt',
    firstPreamble: 'changed preamble',
    toolDescription: 'Stable tool schema.',
    secondUser: 'second A',
  })
  const changedTools = await prefixAfterTwoTurns({
    systemPrompt: 'system prompt',
    firstPreamble: 'preamble',
    toolDescription: 'Changed tool schema.',
    secondUser: 'second A',
  })
  const changedModel = await prefixAfterTwoTurns({
    systemPrompt: 'system prompt',
    firstPreamble: 'preamble',
    toolDescription: 'Stable tool schema.',
    secondUser: 'second A',
    modelId: 'changed-model',
  })

  expect(equivalentPrefix).toBe(basePrefix)
  expect(changedSystem).not.toBe(basePrefix)
  expect(changedPreamble).not.toBe(basePrefix)
  expect(changedTools).not.toBe(basePrefix)
  expect(changedModel).not.toBe(basePrefix)
})

test('provider request prefix restabilizes after compaction replaces old history', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  transcript.pushUserTurn(model, text('recent question'))
  transcript.applyProviderEvent(model, events.text('recent answer'))
  transcript.applyProviderEvent(model, events.response())
  const provider = new RecordingProvider([
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual([
        'user_message',
        'assistant_text',
        'user_message',
        'assistant_text',
      ])
      return [events.text('compacted summary'), events.response()]
    },
    [events.text('after compact one'), events.response()],
    [events.text('after compact two'), events.response()],
  ])
  const session = createSession(provider, createRuntime(), transcript, model, {
    compaction: { keepRecentTokens: 1 },
  })

  await session.compact()
  await session.send(text('after compact question one'))
  await session.send(text('after compact question two'))

  const firstPostCompact = provider.requests[1]
  const secondPostCompact = provider.requests[2]
  if (!firstPostCompact || !secondPostCompact) throw new Error('missing post-compact provider requests')
  expect(firstPostCompact.items[0]).toEqual({
    type: 'user_message',
    content: [{ type: 'text', text: 'Previous conversation summary:\ncompacted summary' }],
  })
  expect(JSON.stringify(firstPostCompact.items)).not.toContain('old question')
  expect(JSON.stringify(secondPostCompact.items.slice(0, firstPostCompact.items.length))).toBe(
    JSON.stringify(firstPostCompact.items),
  )
})

test('provider request is built from effective transcript without internal blocks', async () => {
  const transcript = makeTranscript()
  transcript.pushUserTurn(model, text('old question'))
  transcript.applyProviderEvent(model, events.text('old answer'))
  transcript.applyProviderEvent(model, events.response())
  const boundary = transcript.insertCompactionBoundary(3, model, 'summary', 2)
  transcript.pushUserTurn(model, text('recent question'))
  transcript.applyProviderEvent(model, events.response({ inputTokens: 9, outputTokens: 4, cacheReadTokens: 3 }))
  transcript.applyProviderEvent(model, events.error('provider internal failure', 'context_length_exceeded'))
  transcript.pushAbort(model, true)
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
      expect(JSON.stringify(request.items)).not.toContain('cacheReadTokens')
      expect(JSON.stringify(request.items)).not.toContain('provider internal failure')
      expect(JSON.stringify(request.items)).not.toContain('context_length_exceeded')
      expect(JSON.stringify(request.items)).not.toContain('isResumed')
      return [events.text('answer'), events.response()]
    },
  ])
  const session = createSession(provider, createRuntime(), transcript)

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

test('cache usage is recorded without leaking into model context or breaking tool loop', async () => {
  const smallModel: ModelSelection = {
    ...model,
    model: { ...model.model, contextWindow: 10_000 },
  }
  const provider = new RecordingProvider([
    [
      events.toolCall('tool-1', 'cache_tool', { value: 1 }),
      events.response({ inputTokens: 1, outputTokens: 1, cacheReadTokens: 500, cacheWriteTokens: 300 }),
    ],
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      return [events.text('done'), events.response({ inputTokens: 1, outputTokens: 1 })]
    },
  ])
  const runtime = createRuntime({
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
  const session = createSession(provider, runtime, undefined, smallModel)

  await session.send(text('use tool'))

  expect(session.state().toolCalls).toBe(1)
  expect(provider.requests).toHaveLength(2)
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(false)
  const responseBlocks = session.transcript().blocks.filter((block) => block.type === 'response')
  expect(responseBlocks[0]).toMatchObject({
    type: 'response',
    usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 500, cacheWriteTokens: 300 },
  })
  expect(responseBlocks[1]).toMatchObject({
    type: 'response',
    usage: { inputTokens: 1, outputTokens: 1, cacheReadTokens: 0, cacheWriteTokens: 0 },
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

async function prefixAfterTwoTurns(options: {
  systemPrompt: string
  firstPreamble: string
  toolDescription: string
  secondUser: string
  modelId?: string
}): Promise<string> {
  let round = 0
  const selection: ModelSelection = {
    ...model,
    model: { ...model.model, id: options.modelId ?? model.model.id },
  }
  const provider = new RecordingProvider([
    [events.text('first answer'), events.response()],
    [events.text('second answer'), events.response()],
  ])
  const runtime = createRuntime({
    systemPrompt: () => options.systemPrompt,
    preamble: () => {
      round += 1
      return round === 1 ? options.firstPreamble : 'second preamble'
    },
    tools: () => [
      {
        name: 'stable_tool',
        description: options.toolDescription,
        inputSchema: { type: 'object', properties: { value: { type: 'string' } } },
        invoke: () => ({ output: [{ type: 'text', text: 'ok' }] }),
      },
    ],
  })
  const session = createSession(provider, runtime, undefined, selection)

  await session.send(text('first'))
  await session.send(text(options.secondUser))

  const secondRequest = provider.requests[1]
  if (!secondRequest) throw new Error('missing second request')
  return JSON.stringify({
    modelId: secondRequest.modelId,
    systemPrompt: secondRequest.systemPrompt,
    cwd: secondRequest.cwd,
    tools: secondRequest.tools,
    thinking: secondRequest.thinking,
    items: secondRequest.items.slice(0, -1),
  })
}

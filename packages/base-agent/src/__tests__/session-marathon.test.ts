import { expect, test } from 'bun:test'
import type { InferenceRequest, ProviderEvent } from '@demi/provider'
import { events } from '@demi/provider'
import { AgentSession, Transcript } from '../index'
import {
  assertNoOrphanToolItems,
  assertTranscriptInvariants,
  createDefinition,
  createSession,
  MemorySessionStore,
  RecordingProvider,
  text,
} from './helpers'

test('AgentSession marathon keeps requests and transcript consistent across long-lived actions', async () => {
  const slowGate = deferred<void>()
  const slowStarted = deferred<void>()
  let session = createSession(new RecordingProvider([]))
  const provider = new RecordingProvider([
    (request) => {
      expectRequestMatchesTranscript(request, session)
      return [events.toolCall('tool-1', 'echo_tool', { value: 'tool step' }), events.response()]
    },
    (request) => {
      expectRequestMatchesTranscript(request, session)
      expect(request.items.map((item) => item.type)).toEqual(['user_message', 'tool_use', 'tool_result'])
      return [events.text('tool complete'), events.response()]
    },
    async function* (request): AsyncIterable<ProviderEvent> {
      expectRequestMatchesTranscript(request, session)
      expect(latestUserText(request)).toBe('queued first')
      slowStarted.resolve(undefined)
      await slowGate.promise
      yield events.text('queued first answer')
      yield events.response()
    },
    (request) => {
      expectRequestMatchesTranscript(request, session)
      expect(latestUserText(request)).toBe('queued second')
      return [events.error('transient failure', 'rate_limit')]
    },
    (request) => {
      expectRequestMatchesTranscript(request, session)
      expect(latestUserText(request)).toBe('queued second')
      return [events.text('retry recovered'), events.response()]
    },
    (request) => {
      expectRequestMatchesTranscript(request, session)
      expect(latestUserText(request)).toBe('abort me')
      return [events.abort()]
    },
    (request) => {
      expectRequestMatchesTranscript(request, session)
      expect(request.items.at(-1)).toEqual({
        type: 'user_message',
        content: [{ type: 'text', text: 'Continue from where you left off.' }],
      })
      return [events.text('resume recovered'), events.response()]
    },
    (request) => {
      expect(request.tools).toEqual([])
      expect(request.systemPrompt).toContain('Summarize the previous conversation')
      assertNoOrphanToolItems(request.items)
      expect(JSON.stringify(request.items)).toContain('retry recovered')
      expect(JSON.stringify(request.items)).toContain('resume recovered')
      return [events.text('marathon summary'), events.response()]
    },
    (request) => {
      expectRequestMatchesTranscript(request, session)
      expect(request.items).toEqual([
        {
          type: 'user_message',
          content: [{ type: 'text', text: 'Previous conversation summary:\nmarathon summary' }],
        },
        { type: 'user_message', content: [{ type: 'text', text: 'preamble' }, { type: 'text', text: 'after compact' }] },
      ])
      return [events.text('finished'), events.response()]
    },
  ])
  const definition = createDefinition({
    tools: (ctx) => [
      {
        name: 'echo_tool',
        description: 'Echoes input.',
        inputSchema: { type: 'object' },
        invoke: (_toolCtx, input) => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: JSON.stringify(input) }] }
        },
      },
    ],
  })
  session = createSession(provider, definition)

  await session.send(text('tool step'))
  expect(session.state().toolCalls).toBe(1)
  assertTranscriptInvariants(session.transcript().blocks)

  const firstQueued = session.send(text('queued first'))
  await slowStarted.promise
  const secondQueued = session.send(text('queued second'))
  expect(session.queuedMessages()).toMatchObject([{ text: 'queued second' }])
  slowGate.resolve(undefined)
  await firstQueued
  await expect(secondQueued).rejects.toThrow('transient failure')
  expect(session.phase()).toBe('idle')
  assertTranscriptInvariants(session.transcript().blocks)

  await session.retry()
  expect(session.transcript().blocks.some((block) => block.type === 'error')).toBe(false)

  await session.send(text('abort me'))
  expect(session.transcript().blocks.at(-1)).toMatchObject({ type: 'abort' })
  await session.resume()
  expect(session.transcript().blocks.some((block) => block.type === 'abort' && block.isResumed)).toBe(true)

  await session.compact()
  expect(session.transcript().blocks.some((block) => block.type === 'compaction_boundary')).toBe(true)
  await session.send(text('after compact'))

  expect(provider.requests).toHaveLength(9)
  expect(session.phase()).toBe('idle')
  expect(session.queuedMessages()).toEqual([])
  expect(session.transcript().pendingToolCalls()).toHaveLength(0)
  assertTranscriptInvariants(session.transcript().blocks)
})

test('restored session continues from snapshot without re-executing completed tools', async () => {
  const store = new MemorySessionStore()
  const firstProvider = new RecordingProvider([
    [events.toolCall('tool-1', 'write_once', { path: 'file.txt' }), events.response()],
    [events.text('wrote file'), events.response()],
  ])
  const definition = createDefinition({
    tools: (ctx) => [
      {
        name: 'write_once',
        description: 'Represents a non-idempotent write.',
        inputSchema: { type: 'object' },
        invoke: () => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: 'created file.txt' }] }
        },
      },
    ],
  })
  const first = createSession(firstProvider, definition, undefined, undefined, { store })

  await first.send(text('write file'))
  expect(first.state().toolCalls).toBe(1)

  const snapshot = store.snapshots.at(-1)
  expect(snapshot).toBeDefined()
  const restoredProvider = new RecordingProvider([
    (request) => {
      expect(request.items.map((item) => item.type)).toEqual([
        'user_message',
        'tool_use',
        'tool_result',
        'assistant_text',
        'user_message',
      ])
      assertNoOrphanToolItems(request.items)
      return [events.text('restored follow-up'), events.response()]
    },
  ])
  const restored = createSession(restoredProvider, definition, snapshot ? new Transcript(snapshot.transcript.blocks) : undefined)

  await restored.send(text('continue after restore'))

  expect(restored.state().toolCalls).toBe(0)
  expect(restored.transcript().pendingToolCalls()).toHaveLength(0)
})

test('AgentSession.fromSnapshot restores state and model context with a fresh idle runtime', async () => {
  const store = new MemorySessionStore<{ toolCalls: number }>()
  const firstProvider = new RecordingProvider([
    [events.toolCall('tool-1', 'write_once', { path: 'file.txt' }), events.response()],
    [events.text('wrote file'), events.response()],
  ])
  const definition = createDefinition({
    tools: (ctx) => [
      {
        name: 'write_once',
        description: 'Represents a non-idempotent write.',
        inputSchema: { type: 'object' },
        invoke: () => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: 'created file.txt' }] }
        },
      },
    ],
  })
  const first = createSession(firstProvider, definition, undefined, undefined, { store })

  await first.send(text('write file'))

  const snapshot = structuredClone(store.snapshots.at(-1))
  if (!snapshot) throw new Error('missing snapshot')
  snapshot.phase = 'running'
  snapshot.queue = [{ id: 'queued-after-crash', text: 'queued after crash', content: text('queued after crash') }]
  const restoredProvider = new RecordingProvider([
    (request) => {
      expect(request.cwd).toBe(snapshot.cwd)
      expect(request.modelId).toBe(snapshot.model.model.id)
      expect(request.items.map((item) => item.type)).toEqual([
        'user_message',
        'tool_use',
        'tool_result',
        'assistant_text',
        'user_message',
      ])
      assertNoOrphanToolItems(request.items)
      return [events.toolCall('tool-2', 'write_once', { path: 'second.txt' }), events.response()]
    },
    (request) => {
      expect(request.items.filter((item) => item.type === 'tool_result')).toHaveLength(2)
      assertNoOrphanToolItems(request.items)
      return [events.text('restored write complete'), events.response()]
    },
  ])
  const restored = AgentSession.fromSnapshot(
    {
      provider: restoredProvider,
      definition,
      snapshot,
    },
    {
      idFactory: (() => {
        let id = 0
        return () => `restored-${++id}`
      })(),
      now: () => '2026-06-17T00:00:00.000Z',
    },
  )

  snapshot.state.toolCalls = 99
  snapshot.transcript.blocks = []

  expect(restored.state()).toEqual({ toolCalls: 1 })
  expect(restored.phase()).toBe('idle')
  expect(restored.queuedMessages()).toEqual([])

  await restored.send(text('continue after restore'))

  expect(restored.state()).toEqual({ toolCalls: 2 })
  const restoredUser = restored.transcript().blocks.find((block) => {
    if (block.type !== 'user') return false
    return block.content.some((item) => item.type === 'text' && item.text === 'continue after restore')
  })
  expect(restoredUser).toMatchObject({ type: 'user', id: 'restored-2' })
  expect(restored.transcript().blocks.at(-2)).toMatchObject({ type: 'text', text: 'restored write complete' })
  expect(restoredProvider.requests).toHaveLength(2)
})

test('AgentSession.fromSnapshot rejects mismatched definitions', async () => {
  const store = new MemorySessionStore()
  const provider = new RecordingProvider([[events.text('answer'), events.response()]])
  const definition = createDefinition()
  const session = createSession(provider, definition, undefined, undefined, { store })

  await session.send(text('save snapshot'))

  const snapshot = store.snapshots.at(-1)
  if (!snapshot) throw new Error('missing snapshot')

  expect(() => {
    AgentSession.fromSnapshot({
      provider: new RecordingProvider([]),
      definition: createDefinition({ name: 'other-agent' }),
      snapshot,
    })
  }).toThrow('snapshot definition "test-agent" does not match "other-agent"')
})

test('restored session after provider error does not duplicate completed tool results', async () => {
  const store = new MemorySessionStore()
  const firstProvider = new RecordingProvider([
    [events.toolCall('tool-1', 'write_once', { path: 'file.txt' }), events.response()],
    [events.text('wrote file'), events.response()],
    [events.error('provider failed after write', 'rate_limit')],
  ])
  const definition = createDefinition({
    tools: (ctx) => [
      {
        name: 'write_once',
        description: 'Represents a non-idempotent write.',
        inputSchema: { type: 'object' },
        invoke: () => {
          ctx.state.toolCalls += 1
          return { output: [{ type: 'text', text: 'created file.txt' }] }
        },
      },
    ],
  })
  const first = createSession(firstProvider, definition, undefined, undefined, { store })

  await first.send(text('write file'))
  await expect(first.send(text('after write'))).rejects.toThrow('provider failed after write')
  expect(first.state().toolCalls).toBe(1)

  const snapshot = store.snapshots.at(-1)
  expect(snapshot?.transcript.blocks.at(-1)).toMatchObject({ type: 'error', code: 'rate_limit' })
  const restoredProvider = new RecordingProvider([
    (request) => {
      const toolResults = request.items.filter((item) => item.type === 'tool_result')
      expect(toolResults).toHaveLength(1)
      assertNoOrphanToolItems(request.items)
      return [events.text('recovered after restore'), events.response()]
    },
  ])
  const restored = createSession(restoredProvider, definition, snapshot ? new Transcript(snapshot.transcript.blocks) : undefined)

  await restored.send(text('recover after error'))

  expect(restored.state().toolCalls).toBe(0)
})

function expectRequestMatchesTranscript(request: InferenceRequest, session: ReturnType<typeof createSession>): void {
  expect(request.items).toEqual(session.transcript().collectInferenceItems())
  assertNoOrphanToolItems(request.items)
}

function latestUserText(request: InferenceRequest): string {
  const latest = [...request.items].reverse().find((item) => item.type === 'user_message')
  if (latest?.type !== 'user_message') throw new Error('missing user message')
  const textBlock = [...latest.content].reverse().find((block) => block.type === 'text')
  if (textBlock?.type !== 'text') throw new Error('missing text content')
  return textBlock.text
}

function deferred<T>(): { promise: Promise<T>; resolve: (value: T) => void } {
  let resolve!: (value: T) => void
  const promise = new Promise<T>((innerResolve) => {
    resolve = innerResolve
  })
  return { promise, resolve }
}

import { expect, test } from 'bun:test'
import { AgentClient, type ServerFrame } from '@demicodes/agent/client'
import { deferred } from '@demicodes/utils'
import { ConversationCache } from '../conversation-cache'
import { ConversationRuntime, type RuntimeState } from '../conversation-runtime'

function fixture(id: string) {
  const model = {
    providerId: 'stub', thinking: null,
    model: {
      id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null,
      inputLimit: null, thinking: [], acceptedExtensions: [],
    },
  }
  const state: RuntimeState = {
    id, cwd: '/', blocks: [], phase: 'idle', queue: [], pendingSteers: [],
    model: { providerId: 'stub', modelId: 'stub', thinkingEffort: null, serviceTierId: null },
    lastError: null, load: 'loading', pendingAction: null,
  }
  let receive: (frame: ServerFrame) => void = () => {}
  let connections = 0
  let closes = 0
  const runtime = new ConversationRuntime({
    state,
    prepareModel: async () => ({ providerId: 'stub', model }),
    connect: async () => {
      connections += 1
      return new AgentClient({
        send(frame) {
          if (frame.type === 'open') {
            receive({ type: 'opened' })
          }
        },
        close() {
          closes += 1
        },
        onFrame(handler) {
          receive = handler
          return () => {
            receive = () => {}
          }
        },
      })
    },
  })
  return {
    runtime, state,
    receive: (frame: ServerFrame) => receive(frame),
    connections: () => connections,
    closes: () => closes,
  }
}

test('cached runtimes stay live across switches and all detach on cleanup', async () => {
  const cache = new ConversationCache()
  const first = fixture('first')
  const second = fixture('second')
  try {
    for (const item of [first, second, first, second]) {
      await cache.open(item.state.id, async (entry) => {
        entry.runtime = item.runtime
        await item.runtime.connect()
      })
    }
    first.receive({ type: 'phase', phase: 'running' })
    expect(first.state.phase).toBe('running')
    expect(first.state.load).toBe('ready')
    expect(first.connections()).toBe(1)
    expect(second.connections()).toBe(1)
    expect(first.closes()).toBe(0)
    cache.retain(new Set(['second']))
    expect(first.closes()).toBe(1)
    expect(second.closes()).toBe(0)
  } finally {
    cache.clear()
  }
  expect(second.closes()).toBe(1)
  expect(first.closes()).toBe(1)
})

test('invalidating a pending load cannot discard its replacement', async () => {
  const cache = new ConversationCache()
  const gate = deferred<void>()
  const started = deferred<void>()
  const opening = cache.open('first', async (entry) => {
    started.resolve()
    await gate.promise
    entry.controller.signal.throwIfAborted()
  })
  const failure = opening.catch((error: unknown) => error)
  await started.promise
  cache.delete('first')
  await cache.open('first', async () => {})
  const replacement = cache.get('first')
  gate.resolve()
  expect(await failure).toBeInstanceOf(DOMException)
  expect(cache.get('first')).toBe(replacement)
  cache.clear()
})

test('failed initialization releases its runtime and can be retried', async () => {
  const cache = new ConversationCache()
  const first = fixture('first')
  await expect(cache.open('first', async (entry) => {
    entry.runtime = first.runtime
    await first.runtime.connect()
    throw new Error('Failed to restore')
  })).rejects.toThrow('Failed to restore')
  expect(first.closes()).toBe(1)
  expect(cache.get('first')).toBeUndefined()
  await cache.open('first', async () => {})
  expect(cache.get('first')).toBeDefined()
  cache.clear()
})

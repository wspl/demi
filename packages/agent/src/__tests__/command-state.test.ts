import { afterEach, expect, test } from 'bun:test'
import { z } from 'zod'
import { deferred } from '@demicodes/utils'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { AgentProvider } from '@demicodes/provider'
import { AgentSession } from '../session/session'
import { CommandStateHistory, commandStateSchema } from '../store/command-state'
import { createRuntime, MemorySessionStore, model, text } from './helpers'
import type { AgentSessionStore } from '../types'

const sessions: AgentSession<{ toolCalls: number }>[] = []
afterEach(async () => {
  await Promise.all(sessions.splice(0).map((session) => session.dispose()))
})

function fixture(options: {
  store?: AgentSessionStore<{ toolCalls: number }>
  failRestore?: () => boolean
  provider?: AgentProvider
} = {}) {
  const store = options.store ?? new MemorySessionStore<{ toolCalls: number }>()
  const session = new AgentSession({
    provider: options.provider ?? new StubProvider(Array.from({ length: 8 }, () => [events.text('answer'), events.response()])),
    model,
    cwd: '/workspace',
    runtime: createRuntime({
      restoreState: () => {
        if (options.failRestore?.()) {
          throw new Error('restore failed')
        }
        return { toolCalls: 0 }
      },
    }),
  }, { store, retry: { maxAttempts: 1 } })
  sessions.push(session)
  return { session, store, storage: session.commandStorage() }
}

test('atomic concurrent mutations preserve all values, return detached data and skip unchanged maps', async () => {
  const { session, storage, store } = fixture()
  await Promise.all(Array.from({ length: 20 }, (_, index) =>
    storage.updateJson('items', (current) => [
      ...z.array(z.number()).parse(current === undefined ? [] : current), index,
    ]),
  ))
  const items = z.array(z.number()).parse(await storage.readJson('items'))
  expect(items).toEqual(Array.from({ length: 20 }, (_, index) => index))
  items.push(99)
  expect(await storage.readJson('items')).toHaveLength(20)
  await storage.writeJson('settings', { a: 1, b: 2 })
  const revision = session.commandState().revision
  await storage.writeJson('settings', { b: 2, a: 1 })
  expect(session.commandState().revision).toBe(revision)
  await storage.delete('missing')
  expect(session.commandState().revision).toBe(revision)
  await storage.delete('settings')
  expect(await storage.list('')).toEqual(['items'])
  expect((await store.load())!.commandState.revision).toBe(revision + 1)
})

test('assistant cutoffs select committed state and editing restores the target user boundary', async () => {
  const { session, storage, store } = fixture()
  await storage.writeJson('todos.json', ['pending'])
  await session.send(text('first'))
  const firstAnswer = session.transcript().blocks.find((block) => block.type === 'text')!
  await storage.writeJson('todos.json', ['done'])
  await session.send(text('second'))
  const secondUser = session.transcript().blocks.filter((block) => block.type === 'user')[1]!
  await storage.writeJson('todos.json', ['later'])
  const history = new CommandStateHistory(session.commandState())
  const prefix = session.transcript().throughAssistantMessage(firstAnswer.id)
  const fork = new CommandStateHistory(history.select(
    prefix.blocks, history.boundary(firstAnswer.id, 'after_assistant'), true,
  ))
  expect(fork.values()['todos.json']).toEqual(['pending'])
  await session.editAndSend({
    operationId: 'edit-second', targetBlockId: secondUser.id,
    version: session.transcript().version(), content: text('replacement'),
  })
  await session.waitUntilDone()
  await expect(storage.writeJson('todos.json', ['stale job'])).rejects.toThrow()
  await expect(storage.withSignal(new AbortController().signal).writeJson('todos.json', ['stale RPC'])).rejects.toThrow()
  expect(await session.commandStorage().readJson('todos.json')).toEqual(['done'])
  await session.commandStorage().writeJson('todos.json', ['new history'])
  expect(session.commandState().revision).toBe(4)
  expect((await store.load())!.commandState).toEqual(session.commandState())
  expect(fork.values()['todos.json']).toEqual(['pending'])
})

test('rejected edit preserves current command state and existing invocation handles', async () => {
  let failRestore = false
  const { session, storage } = fixture({ failRestore: () => failRestore })
  await session.send(text('first'))
  const target = session.transcript().blocks[0]!
  await storage.writeJson('value', 1)
  failRestore = true
  await expect(session.editAndSend({
    operationId: 'rejected', targetBlockId: target.id,
    version: session.transcript().version(), content: text('replacement'),
  })).rejects.toThrow('restore failed')
  await storage.updateJson('value', (value) => z.number().parse(value) + 1)
  expect(await storage.readJson('value')).toBe(2)
})

test('assistant completion is ordered after an already admitted state commit', async () => {
  const streamed = deferred<void>()
  const finishText = deferred<void>()
  const writing = deferred<void>()
  const commit = deferred<void>()
  const persisted = new MemorySessionStore<{ toolCalls: number }>()
  const provider: AgentProvider = {
    clone: () => provider,
    async *run() {
      yield events.text('streaming')
      streamed.resolve()
      await finishText.promise
      yield events.response()
    },
  }
  const { session, storage } = fixture({
    provider,
    store: {
      load: () => persisted.load(),
      async save(update, options) {
        if (update.commandState?.revision === 2) {
          writing.resolve()
          await commit.promise
        }
        options?.signal?.throwIfAborted()
        persisted.save(update)
      },
    },
  })
  await storage.writeJson('value', 1)
  const sending = session.send(text('first'))
  await streamed.promise
  const update = storage.writeJson('value', 2)
  await writing.promise
  finishText.resolve()
  expect(session.transcript().blocks.find((block) => block.type === 'text')!.forkable).toBeUndefined()
  commit.resolve()
  await Promise.all([sending, update])
  const answer = session.transcript().blocks.find((block) => block.type === 'text')!
  const history = new CommandStateHistory(session.commandState())
  expect(history.boundary(answer.id, 'after_assistant')).toBe(2)
  await storage.writeJson('value', 3)
  expect(new CommandStateHistory(session.commandState()).boundary(answer.id, 'after_assistant')).toBe(2)
})

test('a failed or cancelled commit leaves the old head and releases subsequent mutations', async () => {
  const persisted = new MemorySessionStore<{ toolCalls: number }>()
  const entered = deferred<void>()
  const release = deferred<void>()
  let pause = false
  let fail = false
  const { session } = fixture({ store: {
    load: () => persisted.load(),
    async save(update, options) {
      if (pause) {
        entered.resolve()
        await release.promise
      }
      options?.signal?.throwIfAborted()
      if (fail) {
        throw new Error('disk failed')
      }
      persisted.save(update)
    },
  } })
  const storage = session.commandStorage()
  await storage.writeJson('value', 1)
  fail = true
  await expect(storage.writeJson('value', 2)).rejects.toThrow('disk failed')
  expect(await storage.readJson('value')).toBe(1)
  fail = false
  pause = true
  const controller = new AbortController()
  const cancelled = storage.withSignal(controller.signal).writeJson('value', 3).catch((error) => error)
  await entered.promise
  controller.abort()
  release.resolve()
  expect(await cancelled).toBeInstanceOf(Error)
  expect(persisted.snapshots.at(-1)!.commandState.revision).toBe(1)
  pause = false
  await storage.writeJson('value', 4)
  expect(session.commandState().revision).toBe(2)
  expect(await storage.readJson('value')).toBe(4)
})

test('disposal invalidates pending and future operations from a job', async () => {
  const { session, storage } = fixture()
  await storage.writeJson('value', 1)
  await session.dispose()
  await expect(storage.writeJson('value', 2)).rejects.toThrow()
  await expect(session.commandStorage().readJson('value')).rejects.toThrow()
  expect(session.commandState().revision).toBe(1)
})

test('storage validates values and references without repairing missing history', async () => {
  const { session, storage } = fixture()
  await expect(storage.writeJson('../escape', true)).rejects.toThrow()
  await expect(storage.writeJson('invalid', undefined)).rejects.toThrow()
  expect(session.commandState().revision).toBe(0)
  expect(() => commandStateSchema.parse({
    ...session.commandState(), revision: 100,
  })).toThrow()
  expect(() => commandStateSchema.parse({
    ...session.commandState(),
    boundaries: [{ blockId: 'missing', edge: 'after_assistant', commandRevision: 100 }],
  })).toThrow()
})

test('command reads distinguish an absent key from stored null without trusting a caller type', async () => {
  const { storage, session } = fixture()
  expect(await storage.readJson('todos')).toBeUndefined()
  expect(await storage.readJson('toString')).toBeUndefined()
  await storage.writeJson('todos', null)
  expect(await storage.readJson('todos')).toBeNull()
  const revision = session.commandState().revision
  await expect(storage.updateJson('todos', (current) =>
    z.array(z.string()).parse(current),
  )).rejects.toThrow()
  expect(await storage.readJson('todos')).toBeNull()
  expect(session.commandState().revision).toBe(revision)
  await storage.delete('todos')
  expect(await storage.readJson('todos')).toBeUndefined()
})

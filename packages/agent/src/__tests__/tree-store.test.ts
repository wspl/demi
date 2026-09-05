import { expect, test } from 'bun:test'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { Block } from '@demicodes/core'
import { AgentSession, completedChildrenCarriedBy, completionMessageId, type AgentNodeRecord, type AgentSessionPersistUpdate } from '../index'
import { MemoryAgentStore } from '../testing'
import { MemorySessionStore, createRuntime as baseRuntime, model, text } from './helpers'

// The journal contract a node's store realizes, and the tree contract over
// the in-memory realization: the three commits and completion delivery
// (`docs/subagent.md` § Persistence). The backend's SQLite realization is
// checked against the same contract in its own storage tests.

const createRuntime = () => baseRuntime({ harnessName: 'store-test' })

test('a later turn saves only its own rows — earlier rows are never rewritten', async () => {
  const store = new MemorySessionStore<{ toolCalls: number }>()
  const provider = new StubProvider([
    [events.text('one'), events.response()],
    [events.text('two'), events.response()],
  ])
  const session = new AgentSession(
    { provider, model, cwd: '/workspace', runtime: createRuntime() },
    { store, persistIntervalMs: 60_000 },
  )

  await session.send(text('first'))
  const afterFirst = store.saves.length
  const firstCount = store.saves.at(-1)?.blockCount ?? 0
  expect(firstCount).toBe(3) // user, text, response

  await session.send(text('second'))
  const laterSaves = store.saves.slice(afterFirst)
  expect(laterSaves.length).toBeGreaterThan(0)
  for (const save of laterSaves) {
    for (const { index } of save.changedBlocks) expect(index).toBeGreaterThanOrEqual(firstCount)
  }
  expect(store.saves.at(-1)?.blockCount).toBe(6)
  expect(store.snapshots.at(-1)?.transcript.blocks.map((block) => block.type)).toEqual([
    'user',
    'text',
    'response',
    'user',
    'text',
    'response',
  ])
})

function record(id: string, parentId: string | null): AgentNodeRecord {
  return { id, parentId, description: '', profileName: null, metadata: null, spawnedAt: 1, canSpawnSubagents: true, closedPhase: null, closedAt: null, result: null, failure: null, delivered: false }
}

function checkpoint(queue: AgentSessionPersistUpdate<unknown>['queue'] = []): AgentSessionPersistUpdate<unknown> {
  return { changedBlocks: [], blockCount: 0, state: {}, phase: 'idle', queue, cwd: '/w', model, harnessName: 'store-test' }
}

function userBlock(turnId: string): Block {
  return { type: 'user', id: `u-${turnId}`, turnId, createdAt: '2026-01-01T00:00:00.000Z', model, content: text('hello'), preamble: null }
}

test('create queues the first message with the node; the journal replaces it once the turn is saved', async () => {
  const store = new MemoryAgentStore()
  await store.createNode(record('root', null), checkpoint())
  const brief = { id: 'm1', text: 'brief', content: text('brief') }
  await store.createNode(record('child', 'root'), checkpoint([brief]))

  expect((await store.children('root')).map((node) => node.id)).toEqual(['child'])
  expect((await store.sessionStore('child').load())?.queue).toEqual([brief])
  await store.sessionStore('child').save({ ...checkpoint(), changedBlocks: [{ index: 0, block: userBlock('m1') }], blockCount: 1 })
  const loaded = await store.sessionStore('child').load()
  expect(loaded?.queue).toEqual([])
  expect(loaded?.transcript.blocks.map((block) => block.type)).toEqual(['user'])
  await expect(store.createNode(record('child', 'root'), checkpoint())).rejects.toThrow('already exists')
})

test('a parent save marks delivered the completions it carries, queued or as its turn; the rest stay undelivered', async () => {
  const store = new MemoryAgentStore()
  await store.createNode(record('root', null), checkpoint())
  for (const id of ['a', 'b', 'c']) {
    await store.createNode(record(id, 'root'), checkpoint())
    await store.closeNode(id, { phase: 'completed', closedAt: 2, result: `${id} done`, failure: null })
  }
  expect((await store.children('root')).map((node) => [node.closedPhase, node.result, node.delivered])).toEqual([
    ['completed', 'a done', false],
    ['completed', 'b done', false],
    ['completed', 'c done', false],
  ])

  const save: AgentSessionPersistUpdate<unknown> = {
    ...checkpoint([{ id: completionMessageId('a'), text: 'a done', content: text('a done') }]),
    changedBlocks: [{ index: 0, block: userBlock(completionMessageId('b')) }],
    blockCount: 1,
  }
  expect(completedChildrenCarriedBy(save).sort()).toEqual(['a', 'b'])
  await store.sessionStore('root').save(save)
  expect((await store.node('a'))?.delivered).toBe(true)
  expect((await store.node('b'))?.delivered).toBe(true)
  expect((await store.node('c'))?.delivered).toBe(false)
  await store.markDelivered('c')
  expect((await store.node('c'))?.delivered).toBe(true)
})

test('reopen makes an archived node live with the reviving message queued; delete takes the subtree', async () => {
  const store = new MemoryAgentStore()
  await store.createNode(record('root', null), checkpoint())
  await store.createNode(record('child', 'root'), checkpoint())
  await store.createNode(record('grandchild', 'child'), checkpoint())
  await store.closeNode('child', { phase: 'error', closedAt: 3, result: null, failure: 'boom' })
  expect(await store.node('child')).toMatchObject({ closedPhase: 'error', failure: 'boom' })

  const reviving = { id: 'm2', text: 'again', content: text('again') }
  await store.reopenNode('child', { metadata: { round: 1 }, spawnedAt: 9 }, reviving)
  expect(await store.node('child')).toMatchObject({ closedPhase: null, closedAt: null, result: null, failure: null, delivered: false, metadata: { round: 1 }, spawnedAt: 9 })
  expect((await store.sessionStore('child').load())?.queue).toEqual([reviving])

  await store.deleteNode('child')
  expect(await store.node('child')).toBeNull()
  expect(await store.node('grandchild')).toBeNull()
  expect(await store.node('root')).not.toBeNull()
})

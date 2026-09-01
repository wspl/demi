import { expect, test } from 'bun:test'
import { memoryHostStore } from '@demicodes/shell/testing'
import { StubProvider, events } from '@demicodes/provider/testing'
import type { Block } from '@demicodes/core'
import { AgentSession, hostAgentSessionStore, type BlobStore } from '../index'
import { MemorySessionStore, createRuntime as baseRuntime, model, text } from './helpers'

function memoryBlobStore(): BlobStore & { blobs: Map<string, Uint8Array> } {
  const blobs = new Map<string, Uint8Array>()
  return {
    blobs,
    async put(data) {
      const key = `blob-${blobs.size}-${data.byteLength}`
      for (const [existing, bytes] of blobs) {
        if (bytes.length === data.length && bytes.every((b, i) => b === data[i])) return existing
      }
      blobs.set(key, data)
      return key
    },
    async get(sha256) {
      return blobs.get(sha256) ?? null
    },
  }
}

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

test('hostAgentSessionStore round-trips a session through block rows', async () => {
  const host = memoryHostStore()
  const store = hostAgentSessionStore<{ toolCalls: number }>(host, 'agent-sessions/s1')
  const provider = new StubProvider([[events.text('persisted'), events.response()]])
  const session = new AgentSession(
    { provider, model, cwd: '/workspace', runtime: createRuntime() },
    { store, persistIntervalMs: 0 },
  )
  await session.send(text('hello'))

  const keys = await host.list('agent-sessions/s1/')
  expect(keys.filter((key) => key.includes('/blocks/'))).toHaveLength(3)
  expect(keys).toContain('agent-sessions/s1/state.json')

  const checkpoint = await store.load()
  expect(checkpoint?.transcript.blocks.map((block) => block.type)).toEqual(['user', 'text', 'response'])
  expect(checkpoint?.cwd).toBe('/workspace')
  expect(checkpoint?.harnessName).toBe('store-test')
})

test('shrinking the transcript deletes the stale rows', async () => {
  const host = memoryHostStore()
  const store = hostAgentSessionStore<{ toolCalls: number }>(host, 'agent-sessions/s2')
  await store.save({
    changedBlocks: [0, 1, 2, 3].map((index) => ({ index, block: fakeText(`b${index}`) })),
    blockCount: 4,
    state: { toolCalls: 0 },
    phase: 'idle',
    queue: [],
    cwd: '/w',
    model,
    harnessName: 'store-test',
  })
  await store.save({
    changedBlocks: [],
    blockCount: 2,
    state: { toolCalls: 0 },
    phase: 'idle',
    queue: [],
    cwd: '/w',
    model,
    harnessName: 'store-test',
  })
  const checkpoint = await store.load()
  expect(checkpoint?.transcript.blocks.map((block) => block.id)).toEqual(['b0', 'b1'])
  expect((await host.list('agent-sessions/s2/blocks/')).length).toBe(2)
})

test('media bytes externalize to the blob store and rehydrate on load', async () => {
  const host = memoryHostStore()
  const blobs = memoryBlobStore()
  const store = hostAgentSessionStore<{ toolCalls: number }>(host, 'agent-sessions/s3', { blobs })
  const imageBytes = new Uint8Array([1, 2, 3, 4, 5])
  const userBlock: Block = {
    type: 'user',
    id: 'u1',
    turnId: 't1',
    createdAt: '2026-01-01T00:00:00.000Z',
    model,
    content: [
      { type: 'text', text: 'look at this' },
      { type: 'image', source: { type: 'binary', data: imageBytes, mediaType: 'image/png' } },
    ],
    preamble: null,
  }
  await store.save({
    changedBlocks: [{ index: 0, block: userBlock }],
    blockCount: 1,
    state: { toolCalls: 0 },
    phase: 'idle',
    queue: [],
    cwd: '/w',
    model,
    harnessName: 'store-test',
  })

  // The persisted row carries a ref, never bytes.
  const row = await host.readJson<{ content: Array<{ source?: { type?: string; ref?: string } }> }>(
    'agent-sessions/s3/blocks/00000000.json',
  )
  expect(row?.content[1]?.source?.type).toBe('ref')
  expect(blobs.blobs.size).toBe(1)

  // Load rehydrates the original bytes.
  const checkpoint = await store.load()
  const loaded = checkpoint?.transcript.blocks[0]
  if (loaded?.type !== 'user') throw new Error('expected user block')
  const image = loaded.content[1]
  if (image?.type !== 'image' || image.source.type !== 'binary') throw new Error('expected binary image source')
  expect([...image.source.data]).toEqual([...imageBytes])

  // A missing blob degrades to a text placeholder instead of failing the load.
  blobs.blobs.clear()
  const degraded = await store.load()
  const degradedBlock = degraded?.transcript.blocks[0]
  if (degradedBlock?.type !== 'user') throw new Error('expected user block')
  expect(degradedBlock.content[1]).toMatchObject({ type: 'text' })
})

function fakeText(id: string): Block {
  return { type: 'text', id, createdAt: '2026-01-01T00:00:00.000Z', model, text: id }
}

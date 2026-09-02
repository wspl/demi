import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { AgentSessionCommandStorage, fileHostStore } from '../index'
import { LocalHost } from '../node'

const storeAt = (root: string) => fileHostStore(new LocalHost(root, { storeRoot: root }).fs, root)

test('fileHostStore reads, writes, lists, and deletes JSON files', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = storeAt(root)

  await store.writeJson('nested/todos.json', [{ text: 'a' }])

  expect(await store.readJson<Array<{ text: string }>>('nested/todos.json')).toEqual([{ text: 'a' }])
  expect(await store.list('')).toEqual(['nested/todos.json'])
  expect(await store.list('nested')).toEqual(['nested/todos.json'])

  await store.delete('nested/todos.json')

  expect(await store.readJson('nested/todos.json')).toBeNull()
  expect(await store.list('')).toEqual([])
})

test('fileHostStore works with agent-session-scoped command storage', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = storeAt(root)
  const first = new AgentSessionCommandStorage(store, 'session-a')
  const second = new AgentSessionCommandStorage(store, 'session-b')

  await first.writeJson('todos.json', [{ text: 'a' }])
  await second.writeJson('todos.json', [{ text: 'b' }])

  expect(await first.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'a' }])
  expect(await second.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'b' }])
  expect(await first.list('')).toEqual(['todos.json'])
  expect(await store.list('')).toEqual(['agent-sessions/session-a/todos.json', 'agent-sessions/session-b/todos.json'])
})

test('fileHostStore round-trips Uint8Array values inside stored JSON', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = storeAt(root)

  await store.writeJson('session/checkpoint.json', {
    content: [{ type: 'image', source: { type: 'binary', data: new Uint8Array([137, 80, 78, 71]), mediaType: 'image/png' } }],
  })

  const restored = await store.readJson<{ content: Array<{ source: { data: Uint8Array } }> }>('session/checkpoint.json')
  expect(restored?.content[0].source.data).toBeInstanceOf(Uint8Array)
  expect([...(restored?.content[0].source.data ?? [])]).toEqual([137, 80, 78, 71])
})

test('fileHostStore keeps a concurrently overwritten key complete and parseable', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = storeAt(root)
  // Payloads large enough that a torn or interleaved write could not parse back.
  const payloads = Array.from({ length: 6 }, (_, writer) => ({ writer, filler: `${writer}`.repeat(2_000_000) }))

  for (let round = 0; round < 3; round++) {
    await Promise.all(payloads.map((payload) => store.writeJson('session/checkpoint.json', payload)))

    const restored = await store.readJson<{ writer: number; filler: string }>('session/checkpoint.json')
    if (!restored) throw new Error('checkpoint missing after concurrent writes')
    expect(restored.filler).toBe(`${restored.writer}`.repeat(2_000_000))
    // No temp files may survive a completed write.
    expect(await store.list('')).toEqual(['session/checkpoint.json'])
  }
})

test('fileHostStore rejects keys that are not relative store paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = storeAt(root)

  await expect(store.writeJson('../outside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson('nested/../inside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson(join(root, 'absolute-inside-root.json'), {})).rejects.toThrow('HostStore keys must be relative')
  await expect(store.writeJson('bad\0key.json', {})).rejects.toThrow('Invalid HostStore key')
})

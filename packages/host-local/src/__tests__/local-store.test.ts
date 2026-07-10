import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { AgentSessionCommandStorage } from '@demicodes/shell'
import { LocalHostStore } from '../local-store'

test('LocalHostStore reads, writes, lists, and deletes JSON files', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalHostStore(root)

  await store.writeJson('nested/todos.json', [{ text: 'a' }])

  expect(await store.readJson<Array<{ text: string }>>('nested/todos.json')).toEqual([{ text: 'a' }])
  expect(await store.list('')).toEqual(['nested/todos.json'])
  expect(await store.list('nested')).toEqual(['nested/todos.json'])

  await store.delete('nested/todos.json')

  expect(await store.readJson('nested/todos.json')).toBeNull()
  expect(await store.list('')).toEqual([])
})

test('LocalHostStore works with agent-session-scoped command storage', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalHostStore(root)
  const first = new AgentSessionCommandStorage(store, 'session-a')
  const second = new AgentSessionCommandStorage(store, 'session-b')

  await first.writeJson('todos.json', [{ text: 'a' }])
  await second.writeJson('todos.json', [{ text: 'b' }])

  expect(await first.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'a' }])
  expect(await second.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'b' }])
  expect(await first.list('')).toEqual(['todos.json'])
  expect(await store.list('')).toEqual(['agent-sessions/session-a/todos.json', 'agent-sessions/session-b/todos.json'])
})

test('LocalHostStore round-trips Uint8Array values inside stored JSON', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalHostStore(root)

  await store.writeJson('session/checkpoint.json', {
    content: [{ type: 'image', source: { type: 'binary', data: new Uint8Array([137, 80, 78, 71]), mediaType: 'image/png' } }],
  })

  const restored = await store.readJson<{ content: Array<{ source: { data: Uint8Array } }> }>('session/checkpoint.json')
  expect(restored?.content[0].source.data).toBeInstanceOf(Uint8Array)
  expect([...(restored?.content[0].source.data ?? [])]).toEqual([137, 80, 78, 71])
})

test('LocalHostStore rejects keys that are not relative store paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalHostStore(root)

  await expect(store.writeJson('../outside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson('nested/../inside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson(join(root, 'absolute-inside-root.json'), {})).rejects.toThrow('HostStore keys must be relative')
  await expect(store.writeJson('bad\0key.json', {})).rejects.toThrow('Invalid HostStore key')
})

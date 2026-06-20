import { mkdtemp } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { AgentSessionCommandStorage } from '@demi/shell'
import { LocalDemiStore } from '../local-store'

test('LocalDemiStore reads, writes, lists, and deletes JSON files', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalDemiStore(root)

  await store.writeJson('nested/todos.json', [{ text: 'a' }])

  expect(await store.readJson<Array<{ text: string }>>('nested/todos.json')).toEqual([{ text: 'a' }])
  expect(await store.list('')).toEqual(['nested/todos.json'])
  expect(await store.list('nested')).toEqual(['nested/todos.json'])

  await store.delete('nested/todos.json')

  expect(await store.readJson('nested/todos.json')).toBeNull()
  expect(await store.list('')).toEqual([])
})

test('LocalDemiStore works with agent-session-scoped command storage', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalDemiStore(root)
  const first = new AgentSessionCommandStorage(store, 'session-a')
  const second = new AgentSessionCommandStorage(store, 'session-b')

  await first.writeJson('todos.json', [{ text: 'a' }])
  await second.writeJson('todos.json', [{ text: 'b' }])

  expect(await first.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'a' }])
  expect(await second.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'b' }])
  expect(await first.list('')).toEqual(['todos.json'])
  expect(await store.list('')).toEqual(['session-a/todos.json', 'session-b/todos.json'])
})

test('LocalDemiStore rejects keys that are not relative store paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalDemiStore(root)

  await expect(store.writeJson('../outside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson('nested/../inside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson(join(root, 'absolute-inside-root.json'), {})).rejects.toThrow('DemiStore keys must be relative')
  await expect(store.writeJson('bad\0key.json', {})).rejects.toThrow('Invalid DemiStore key')
})

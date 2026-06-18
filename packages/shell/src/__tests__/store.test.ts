import { mkdtemp } from 'node:fs/promises'
import { join } from 'node:path'
import { tmpdir } from 'node:os'
import { expect, test } from 'bun:test'
import { LocalDemiStore, SessionCommandStorage } from '../store'

test('SessionCommandStorage prefixes keys by session id and exposes session-local keys', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalDemiStore(root)
  const first = new SessionCommandStorage(store, 'session-a')
  const second = new SessionCommandStorage(store, 'session-b')

  await first.writeJson('todos.json', [{ text: 'a' }])
  await second.writeJson('todos.json', [{ text: 'b' }])

  expect(await first.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'a' }])
  expect(await second.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'b' }])
  expect(await first.list('')).toEqual(['todos.json'])
  expect(await store.list('')).toEqual(['session-a/todos.json', 'session-b/todos.json'])
})

test('SessionCommandStorage rejects keys and session ids that escape the session prefix', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalDemiStore(root)
  const first = new SessionCommandStorage(store, 'session-a')
  const second = new SessionCommandStorage(store, 'session-b')

  await second.writeJson('todos.json', [{ text: 'b' }])

  await expect(Promise.resolve().then(() => first.writeJson('../session-b/todos.json', [{ text: 'hijack' }]))).rejects.toThrow(
    'path traversal',
  )
  await expect(Promise.resolve().then(() => first.writeJson('nested\\..\\todos.json', [{ text: 'hijack' }]))).rejects.toThrow(
    'path traversal',
  )
  await expect(Promise.resolve().then(() => first.writeJson('/absolute.json', {}))).rejects.toThrow('must be relative')
  expect(() => new SessionCommandStorage(store, '../session-b')).toThrow('Invalid command storage session id')

  expect(await second.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'b' }])
})

test('LocalDemiStore rejects keys that are not relative store paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-store-'))
  const store = new LocalDemiStore(root)

  await expect(store.writeJson('../outside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson('nested/../inside.json', {})).rejects.toThrow('path traversal')
  await expect(store.writeJson(join(root, 'absolute-inside-root.json'), {})).rejects.toThrow('DemiStore keys must be relative')
  await expect(store.writeJson('bad\0key.json', {})).rejects.toThrow('Invalid DemiStore key')
})

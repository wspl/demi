import { expect, test } from 'bun:test'
import { AgentSessionCommandStorage, type HostStore } from '../index'

test('AgentSessionCommandStorage prefixes keys by agent session id and exposes session-local keys', async () => {
  const store = new MemoryHostStore()
  const first = new AgentSessionCommandStorage(store, 'session-a')
  const second = new AgentSessionCommandStorage(store, 'session-b')

  await first.writeJson('todos.json', [{ text: 'a' }])
  await second.writeJson('todos.json', [{ text: 'b' }])

  expect(await first.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'a' }])
  expect(await second.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'b' }])
  expect(await first.list('')).toEqual(['todos.json'])
  expect(await store.list('')).toEqual(['session-a/todos.json', 'session-b/todos.json'])
})

test('AgentSessionCommandStorage rejects keys and agent session ids that escape the session prefix', async () => {
  const store = new MemoryHostStore()
  const first = new AgentSessionCommandStorage(store, 'session-a')
  const second = new AgentSessionCommandStorage(store, 'session-b')

  await second.writeJson('todos.json', [{ text: 'b' }])

  await expect(Promise.resolve().then(() => first.writeJson('../session-b/todos.json', [{ text: 'hijack' }]))).rejects.toThrow(
    'path traversal',
  )
  await expect(Promise.resolve().then(() => first.writeJson('nested\\..\\todos.json', [{ text: 'hijack' }]))).rejects.toThrow(
    'path traversal',
  )
  await expect(Promise.resolve().then(() => first.writeJson('/absolute.json', {}))).rejects.toThrow('must be relative')
  expect(() => new AgentSessionCommandStorage(store, '../session-b')).toThrow('Invalid command storage agent session id')

  expect(await second.readJson<Array<{ text: string }>>('todos.json')).toEqual([{ text: 'b' }])
})

class MemoryHostStore implements HostStore {
  private readonly values = new Map<string, unknown>()

  async readJson<T>(key: string): Promise<T | null> {
    if (!this.values.has(key)) return null
    return structuredClone(this.values.get(key)) as T
  }

  async writeJson<T>(key: string, value: T): Promise<void> {
    this.values.set(key, structuredClone(value))
  }

  async delete(key: string): Promise<void> {
    this.values.delete(key)
  }

  async list(prefix: string): Promise<string[]> {
    return [...this.values.keys()].filter((key) => key.startsWith(prefix)).sort()
  }
}

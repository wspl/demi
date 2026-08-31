import { expect, test } from 'bun:test'
import { openSqliteDatabase } from '../storage/database'
import { migrate } from '../storage/migrations'
import { DbHostStore } from '../storage/host-store'
import { ConversationIndex } from '../storage/conversations'

function openTestDb() {
  const db = openSqliteDatabase(':memory:')
  migrate(db)
  return db
}

test('migrations apply once and are idempotent', () => {
  const db = openTestDb()
  migrate(db)
  const tables = db
    .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
    .map((row) => row.name)
  for (const table of [
    'users',
    'web_sessions',
    'devices',
    'workspaces',
    'conversations',
    'connections',
    'usage_ledger',
    'attachments',
    'host_store',
    'settings',
  ]) {
    expect(tables).toContain(table)
  }
  db.close()
})

test('DbHostStore round-trips portable JSON and lists by literal prefix', async () => {
  const db = openTestDb()
  const store = new DbHostStore(db, 'agent')

  expect(await store.readJson('missing')).toBeNull()
  await store.writeJson('agent-sessions/s1/checkpoint.json', {
    bytes: new Uint8Array([1, 2, 3]),
    when: new Date('2026-08-31T00:00:00Z'),
    count: 42n,
  })
  const value = await store.readJson<{ bytes: Uint8Array; when: Date; count: bigint }>(
    'agent-sessions/s1/checkpoint.json',
  )
  expect(value?.bytes).toBeInstanceOf(Uint8Array)
  expect([...(value?.bytes ?? [])]).toEqual([1, 2, 3])
  expect(value?.when).toBeInstanceOf(Date)
  expect(value?.count).toBe(42n)

  // Overwrite is a single upsert.
  await store.writeJson('agent-sessions/s1/checkpoint.json', { v: 2 })
  expect(await store.readJson<{ v: number }>('agent-sessions/s1/checkpoint.json')).toEqual({ v: 2 })

  await store.writeJson('agent-sessions/s2/checkpoint.json', { v: 1 })
  await store.writeJson('other/x.json', { v: 1 })
  expect(await store.list('agent-sessions/')).toEqual([
    'agent-sessions/s1/checkpoint.json',
    'agent-sessions/s2/checkpoint.json',
  ])
  // LIKE metacharacters in prefixes are literal.
  await store.writeJson('pre%fix/a', { v: 1 })
  await store.writeJson('prefix/b', { v: 1 })
  expect(await store.list('pre%fix/')).toEqual(['pre%fix/a'])

  // Scopes are isolated.
  expect(await new DbHostStore(db, 'other-scope').list('')).toEqual([])

  await store.delete('other/x.json')
  expect(await store.readJson('other/x.json')).toBeNull()
  db.close()
})

test('conversation index CRUD and ordering', () => {
  const db = openTestDb()
  db.run("INSERT INTO users (id, username, password_hash, role, created_at) VALUES ('u1', 'local', '!', 'master', '2026-01-01T00:00:00Z')")
  const index = new ConversationIndex(db)

  const first = index.create('u1')
  const second = index.create('u1')
  expect(first.title).toBe('New conversation')

  index.defaultTitle(first.id, 'hello world')
  index.defaultTitle(first.id, 'should not overwrite')
  expect(index.get(first.id)?.title).toBe('hello world')

  index.rename(second.id, 'renamed')
  expect(index.get(second.id)?.title).toBe('renamed')

  // Most recently updated first.
  const list = index.listForUser('u1')
  expect(list.map((record) => record.id)).toEqual([second.id, first.id])

  index.setArchived(first.id, true)
  expect(index.listForUser('u1').map((record) => record.id)).toEqual([second.id])
  expect(index.listForUser('u1', { archived: true }).map((record) => record.id)).toEqual([first.id])
  index.setArchived(first.id, false)
  expect(index.listForUser('u1')).toHaveLength(2)

  index.setModel(second.id, 'conn-1', 'model-x')
  const updated = index.get(second.id)
  expect(updated?.connectionId).toBe('conn-1')
  expect(updated?.modelId).toBe('model-x')
  db.close()
})

import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import type { ModelSelection } from '@demicodes/core'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, CONVERSATION_MIGRATIONS, migrate } from '../storage/migrations'
import { DbHostStore } from '../storage/host-store'
import { LocalControlService } from '../storage/control'
import { ConversationStores } from '../storage/conversation-store'
import { DirBlobStore } from '../storage/blob-store'
import { filesTreeBackend } from '../storage/files-tree'
import { completionMessageId } from '@demicodes/agent'
import { WebSessions } from '../auth/sessions'

const model: ModelSelection = {
  providerId: 'stub',
  model: { id: 'm', name: 'M', contextWindow: 1000, inputLimit: null, thinking: [], acceptedExtensions: [] },
  thinking: null,
}

function openControlDb() {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  return db
}

test('control migrations apply once and are idempotent', () => {
  const db = openControlDb()
  migrate(db, CONTROL_MIGRATIONS)
  const tables = db
    .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
    .map((row) => row.name)
  for (const table of [
    'users',
    'web_sessions',
    'devices',
    'workspaces',
    'conversations',
    'providers',
    'usage_ledger',
    'attachments',
  ]) {
    expect(tables).toContain(table)
  }
  // The control plane holds no conversation data.
  expect(tables).not.toContain('host_store')
  expect(tables).not.toContain('blocks')
  db.close()
})

test('conversation migrations create the data-plane tables', () => {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONVERSATION_MIGRATIONS)
  const tables = db
    .all<{ name: string }>("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")
    .map((row) => row.name)
  for (const table of ['nodes', 'blocks', 'host_store', 'files']) expect(tables).toContain(table)
  db.close()
})

test('DbHostStore round-trips portable JSON and lists by literal prefix', async () => {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONVERSATION_MIGRATIONS)
  const store = new DbHostStore(db, 'host')

  expect(await store.readJson('missing')).toBeNull()
  await store.writeJson('agent-sessions/s1/state.json', {
    bytes: new Uint8Array([1, 2, 3]),
    when: new Date('2026-08-31T00:00:00Z'),
    count: 42n,
  })
  const value = await store.readJson<{ bytes: Uint8Array; when: Date; count: bigint }>('agent-sessions/s1/state.json')
  expect(value?.bytes).toBeInstanceOf(Uint8Array)
  expect([...(value?.bytes ?? [])]).toEqual([1, 2, 3])
  expect(value?.when).toBeInstanceOf(Date)
  expect(value?.count).toBe(42n)

  // Overwrite is a single upsert.
  await store.writeJson('agent-sessions/s1/state.json', { v: 2 })
  expect(await store.readJson<{ v: number }>('agent-sessions/s1/state.json')).toEqual({ v: 2 })

  await store.writeJson('agent-sessions/s2/state.json', { v: 1 })
  await store.writeJson('other/x.json', { v: 1 })
  expect(await store.list('agent-sessions/')).toEqual([
    'agent-sessions/s1/state.json',
    'agent-sessions/s2/state.json',
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

test('ControlService conversation CRUD and ordering', async () => {
  const db = openControlDb()
  const control = new LocalControlService(db)
  const user = (await control.createMaster({ username: 'local', passwordHash: '!' }))!
  expect(await control.createMaster({ username: 'again', passwordHash: '!' })).toBeNull()

  const first = await control.createConversation(user.id)
  const second = await control.createConversation(user.id)
  expect(first.title).toBe('New conversation')

  await control.defaultConversationTitle(first.id, 'hello world')
  await control.defaultConversationTitle(first.id, 'should not overwrite')
  expect((await control.getConversation(first.id))?.title).toBe('hello world')

  await control.renameConversation(second.id, 'renamed')
  expect((await control.getConversation(second.id))?.title).toBe('renamed')

  // Most recently updated first.
  const list = await control.listConversations(user.id)
  expect(list.map((record) => record.id)).toEqual([second.id, first.id])

  await control.setConversationArchived(first.id, true)
  expect((await control.listConversations(user.id)).map((record) => record.id)).toEqual([second.id])
  expect((await control.listConversations(user.id, { archived: true })).map((record) => record.id)).toEqual([first.id])
  await control.setConversationArchived(first.id, false)
  expect(await control.listConversations(user.id)).toHaveLength(2)

  await control.setConversationModel(second.id, 'conn-1', 'model-x')
  const updated = await control.getConversation(second.id)
  expect(updated?.providerId).toBe('conn-1')
  expect(updated?.modelId).toBe('model-x')
  db.close()
})

test('ControlService device and workspace records', async () => {
  const db = openControlDb()
  const control = new LocalControlService(db)
  const user = (await control.createMaster({ username: 'local', passwordHash: '!' }))!

  const device = await control.createDevice({ userId: user.id, name: 'laptop', platform: 'darwin', tokenHash: 'hash-1' })
  expect(device.lastSeenAt).toBeNull()
  expect(await control.getDevice(device.id)).toEqual(device)
  expect(await control.getDeviceByTokenHash('hash-1')).toEqual(device)
  expect(await control.getDeviceByTokenHash('hash-unknown')).toBeNull()

  await control.touchDeviceSeen(device.id)
  expect((await control.getDevice(device.id))?.lastSeenAt).not.toBeNull()

  const other = await control.createDevice({ userId: user.id, name: 'desktop', platform: 'linux', tokenHash: 'hash-2' })
  expect((await control.listDevices(user.id)).map((row) => row.id)).toEqual([device.id, other.id])

  const workspace = await control.createWorkspace({ userId: user.id, deviceId: device.id, path: '/proj', name: 'proj' })
  expect(await control.getWorkspace(workspace.id)).toEqual(workspace)

  const conversation = await control.createConversation(user.id)
  await control.setConversationWorkspace(conversation.id, workspace.id)
  expect((await control.getConversation(conversation.id))?.workspaceId).toBe(workspace.id)
  await control.setConversationWorkspace(conversation.id, null)
  expect((await control.getConversation(conversation.id))?.workspaceId).toBeNull()

  await control.deleteDevice(other.id)
  expect((await control.listDevices(user.id)).map((row) => row.id)).toEqual([device.id])
  db.close()
})

test('DirBlobStore content-addresses bytes and is idempotent', async () => {
  const root = mkdtempSync(join(tmpdir(), 'demi-blobs-'))
  const blobs = new DirBlobStore(root)
  const data = new Uint8Array([10, 20, 30])
  const sha = await blobs.put(data)
  expect(sha).toMatch(/^[0-9a-f]{64}$/)
  expect(await blobs.put(new Uint8Array([10, 20, 30]))).toBe(sha)
  expect([...((await blobs.get(sha)) ?? [])]).toEqual([10, 20, 30])
  expect(await blobs.get('0'.repeat(64))).toBeNull()
  expect(await blobs.get('../escape')).toBeNull()
  rmSync(root, { recursive: true, force: true })
})

const rootRecord = (id: string, parentId: string | null = null) => ({
  id,
  parentId,
  description: '',
  profileName: null,
  metadata: null,
  spawnedAt: 1,
  canSpawnSubagents: true,
  closedPhase: null,
  closedAt: null,
  result: null,
  failure: null,
  delivered: false,
})
const emptyCheckpoint = () => ({ changedBlocks: [], blockCount: 0, state: {}, phase: 'idle' as const, queue: [], cwd: '/home/demi', model, harnessName: 'h' })

test('the tree store writes node and block rows, media leaves the database, the cold read is the root', async () => {
  const root = mkdtempSync(join(tmpdir(), 'demi-conv-'))
  const blobs = new DirBlobStore(join(root, 'blobs'))
  const stores = new ConversationStores(join(root, 'conversations'), () => blobs)
  const tree = stores.treeStore('conv-1')
  await tree.createNode(rootRecord('conv-1'), emptyCheckpoint())
  const store = tree.sessionStore('conv-1')
  const imageBytes = new Uint8Array([9, 9, 9, 9])

  await store.save({
    ...emptyCheckpoint(),
    changedBlocks: [
      {
        index: 0,
        block: {
          type: 'user',
          id: 'u1',
          turnId: 't1',
          createdAt: '2026-01-01T00:00:00.000Z',
          model,
          content: [
            { type: 'text', text: 'hi' },
            { type: 'image', source: { type: 'binary', data: imageBytes, mediaType: 'image/png' } },
          ],
          preamble: null,
        },
      },
      { index: 1, block: { type: 'text', id: 'a1', createdAt: '2026-01-01T00:00:01.000Z', model, text: 'hello' } },
    ],
    blockCount: 2,
  })

  // Rows are per node and block; no byte payload appears in the database.
  const db = stores.db('conv-1')
  const rows = db.all<{ idx: number; block_json: string }>('SELECT idx, block_json FROM blocks WHERE node_id = ? ORDER BY idx', ['conv-1'])
  expect(rows.map((row) => row.idx)).toEqual([0, 1])
  expect(rows[0].block_json).toContain('"ref"')
  expect(rows[0].block_json).not.toContain('"data"')

  // Load rehydrates the bytes from the blob store.
  const checkpoint = await store.load()
  const user = checkpoint?.transcript.blocks[0]
  if (user?.type !== 'user') throw new Error('expected user block')
  const image = user.content[1]
  if (image?.type !== 'image' || image.source.type !== 'binary') throw new Error('expected binary image')
  expect([...image.source.data]).toEqual([...imageBytes])

  // Shrinking deletes stale rows; the cold transcript read is the root node's rows.
  await store.save({ ...emptyCheckpoint(), blockCount: 1 })
  expect(stores.transcriptBlocks('conv-1').map((block) => block.id)).toEqual(['u1'])
  expect(stores.transcriptBlocks('conv-none')).toEqual([])

  stores.close()
  rmSync(root, { recursive: true, force: true })
})

test('the tree store keeps the tree contract: the brief in the create, delivery by the parent save, reopen, cascade', async () => {
  const root = mkdtempSync(join(tmpdir(), 'demi-conv-tree-'))
  const stores = new ConversationStores(join(root, 'conversations'), () => new DirBlobStore(join(root, 'blobs')))
  const tree = stores.treeStore('c')
  await tree.createNode(rootRecord('c'), emptyCheckpoint())
  const brief = { id: 'm1', text: 'brief', content: [{ type: 'text' as const, text: 'brief' }] }
  await tree.createNode({ ...rootRecord('a', 'c'), description: 'first', metadata: { round: 1 } }, { ...emptyCheckpoint(), queue: [brief] })
  await tree.createNode({ ...rootRecord('b', 'c'), spawnedAt: 2 }, emptyCheckpoint())
  await tree.createNode(rootRecord('a1', 'a'), emptyCheckpoint())
  expect((await tree.children('c')).map((node) => [node.id, node.description, node.metadata])).toEqual([['a', 'first', { round: 1 }], ['b', '', null]])
  expect((await tree.sessionStore('a').load())?.queue).toEqual([brief])

  await tree.closeNode('a', { phase: 'completed', closedAt: 3, result: 'a done', failure: null })
  await tree.closeNode('b', { phase: 'error', closedAt: 4, result: null, failure: 'boom' })
  expect(await tree.node('a')).toMatchObject({ closedPhase: 'completed', result: 'a done', delivered: false })
  expect(await tree.node('b')).toMatchObject({ closedPhase: 'error', failure: 'boom', delivered: false })

  // The root's save carrying a's wakeup as a user turn marks a delivered; b waits.
  await tree.sessionStore('c').save({
    ...emptyCheckpoint(),
    changedBlocks: [{ index: 0, block: { type: 'user', id: 'u1', turnId: completionMessageId('a'), createdAt: '2026-01-01T00:00:00.000Z', model, content: [{ type: 'text', text: 'x' }], preamble: null } }],
    blockCount: 1,
  })
  expect((await tree.node('a'))?.delivered).toBe(true)
  expect((await tree.node('b'))?.delivered).toBe(false)
  await tree.markDelivered('b')
  expect((await tree.node('b'))?.delivered).toBe(true)

  const reviving = { id: 'm2', text: 'again', content: [{ type: 'text' as const, text: 'again' }] }
  await tree.reopenNode('a', { metadata: { round: 2 }, spawnedAt: 9 }, reviving)
  expect(await tree.node('a')).toMatchObject({ closedPhase: null, result: null, delivered: false, metadata: { round: 2 }, spawnedAt: 9 })
  expect((await tree.sessionStore('a').load())?.queue).toEqual([reviving])

  await tree.deleteNode('a')
  expect(await tree.node('a')).toBeNull()
  expect(await tree.node('a1')).toBeNull()
  expect((await tree.children('c')).map((node) => node.id)).toEqual(['b'])

  stores.close()
  rmSync(root, { recursive: true, force: true })
})

test('host_store scopes are isolated per conversation database', async () => {
  const root = mkdtempSync(join(tmpdir(), 'demi-conv-iso-'))
  const stores = new ConversationStores(join(root, 'conversations'), () => new DirBlobStore(join(root, 'blobs')))
  await stores.hostStore('conv-a').writeJson('k', { from: 'a' })
  await stores.hostStore('conv-b').writeJson('k', { from: 'b' })
  expect(await stores.hostStore('conv-a').readJson<{ from: string }>('k')).toEqual({ from: 'a' })
  expect(await stores.hostStore('conv-b').readJson<{ from: string }>('k')).toEqual({ from: 'b' })
  expect(await stores.hostStore('conv-c').readJson('k')).toBeNull()
  expect(() => stores.db('../escape')).toThrow()
  stores.close()
  rmSync(root, { recursive: true, force: true })
})

test('conversation handles are an LRU: a cold read holds one only until others are touched, data survives eviction', async () => {
  const root = mkdtempSync(join(tmpdir(), 'demi-conv-lru-'))
  const stores = new ConversationStores(join(root, 'conversations'), () => new DirBlobStore(join(root, 'blobs')), { maxOpen: 2 })
  const a = stores.hostStore('conv-a')
  await a.writeJson('k', 'a')
  await stores.hostStore('conv-b').writeJson('k', 'b')
  expect(stores.openHandles).toBe(2)
  expect(stores.transcriptBlocks('conv-c')).toEqual([])
  expect(stores.openHandles).toBe(2)
  // conv-a's handle was the oldest and is closed; the store handed out earlier still reads and writes.
  expect(await a.readJson<string>('k')).toBe('a')
  await a.writeJson('k', 'a2')
  expect(await stores.hostStore('conv-a').readJson<string>('k')).toBe('a2')
  expect(stores.openHandles).toBe(2)
  stores.close()
  expect(stores.openHandles).toBe(0)
  rmSync(root, { recursive: true, force: true })
})

test('expired web sessions are swept when a session opens', async () => {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  const user = await control.createUser({ username: 'u', passwordHash: 'x', role: 'user' })
  let now = Date.parse('2026-01-01T00:00:00.000Z')
  const sessions = new WebSessions(control, { ttlMs: 1000, now: () => now })
  await sessions.open(user!.id)
  await sessions.open(user!.id)
  now += 2000
  const live = await sessions.open(user!.id)
  expect(db.get<{ n: number }>('SELECT COUNT(*) AS n FROM web_sessions')?.n).toBe(1)
  expect((await sessions.resolve(live.token))?.user.id).toBe(user!.id)
  db.close()
})

test('concurrent file appends preserve every write and recover after a failed append', async () => {
  const root = mkdtempSync(join(tmpdir(), 'demi-file-appends-'))
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONVERSATION_MIGRATIONS)
  const blobs = new DirBlobStore(root)
  let failNext = false
  const fs = filesTreeBackend(db, {
    get: (hash) => blobs.get(hash),
    put: async (bytes) => {
      if (failNext) { failNext = false; throw new Error('temporary blob write failure') }
      return blobs.put(bytes)
    },
  })
  const encode = (text: string) => new TextEncoder().encode(text)
  try {
    await fs.writeFile('/shared', encode('start\n'))
    const lines = Array.from({ length: 20 }, (_, index) => `${index}\n`)
    await Promise.all(lines.map((line) => fs.appendFile('/shared', encode(line))))
    expect(new TextDecoder().decode(await fs.readFile('/shared'))).toBe(`start\n${lines.join('')}`)
    failNext = true
    const writes = await Promise.allSettled([
      fs.appendFile('/shared', encode('failed\n')),
      fs.appendFile('/shared', encode('recovered\n')),
    ])
    expect(writes.map((write) => write.status)).toEqual(['rejected', 'fulfilled'])
    expect(new TextDecoder().decode(await fs.readFile('/shared'))).toBe(`start\n${lines.join('')}recovered\n`)
    await Promise.all([fs.appendFile('/created', encode('A')), fs.appendFile('/created', encode('B'))])
    expect(new TextDecoder().decode(await fs.readFile('/created'))).toBe('AB')
    // The quota's measure is one sum over the rows.
    expect(await fs.usage()).toBe(`start\n${lines.join('')}recovered\n`.length + 2)
  } finally {
    db.close()
    rmSync(root, { recursive: true, force: true })
  }
})

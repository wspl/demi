import { afterEach, expect, test } from 'bun:test'
import { mkdtemp, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { stringifyPortableJson } from '@demicodes/utils'
import { sqliteAgentTreeStore, readNode } from '../storage/tree-store'
import { openSqliteDatabase } from '../storage/database'
import { CONVERSATION_MIGRATIONS, migrate } from '../storage/migrations'
import { DirBlobStore } from '../storage/blob-store'
import type { AgentNodeRecord, AgentSessionPersistUpdate } from '@demicodes/agent'

const cleanup: (() => void | Promise<void>)[] = []
afterEach(async () => {
  for (const close of cleanup.splice(0)) await close()
})

async function fixture() {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONVERSATION_MIGRATIONS)
  cleanup.push(() => db.close())
  const tree = sqliteAgentTreeStore(db, { put: async () => '0'.repeat(64), get: async () => null })
  const record: AgentNodeRecord = {
    id: 'node',
    parentId: null,
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
  }
  const checkpoint: AgentSessionPersistUpdate<unknown> = {
    state: {},
    phase: 'idle',
    queue: [],
    cwd: '/workspace',
    harnessName: 'test',
    model: {
      providerId: 'p',
      model: {
        id: 'm',
        name: 'M',
        contextWindow: 100,
        outputLimit: null,
        inputLimit: null,
        thinking: [],
        acceptedExtensions: [],
      },
      thinking: null,
    },
    blockCount: 0,
    changedBlocks: [],
  }
  await tree.createNode(record, checkpoint)
  return { db, tree, checkpoint }
}

test('session state corruption and metadata/flag corruption fail without deleting a node', async () => {
  const { db, tree, checkpoint } = await fixture()
  for (const bad of [
    { ...checkpoint, phase: 'bad' },
    { ...checkpoint, queue: [{ id: 'q', text: 7, content: [] }] },
    { ...checkpoint, model: { providerId: 'p' } },
    { ...checkpoint, harnessName: null },
    { ...checkpoint, edits: [{ operationId: 'e' }] },
    { ...checkpoint, state: undefined },
  ]) {
    db.run('UPDATE nodes SET state_json = ? WHERE id = ?', [stringifyPortableJson(bad), 'node'])
    await expect(tree.sessionStore('node').load()).rejects.toThrow()
    await expect(
      tree.reopenNode(
        'node',
        { metadata: null, spawnedAt: 1 },
        { id: 'q', text: 'q', content: [] },
      ),
    ).rejects.toThrow()
    expect(await tree.node('node')).not.toBeNull()
  }
  db.run('UPDATE nodes SET can_spawn = 7 WHERE id = ?', ['node'])
  await expect(tree.node('node')).rejects.toThrow()
  db.run('UPDATE nodes SET can_spawn = 1, metadata_json = ? WHERE id = ?', ['[]', 'node'])
  await expect(tree.node('node')).rejects.toThrow()
})

test('missing, sparse, excess and malformed block rows cannot become partial histories', async () => {
  const { db, checkpoint } = await fixture()
  const block = {
    type: 'text' as const,
    id: 'text',
    createdAt: '2026-09-01T00:00:00.000Z',
    model: checkpoint.model,
    text: 'kept',
  }
  db.run('UPDATE nodes SET block_count = 1 WHERE id = ?', ['node'])
  expect(() => readNode(db, 'node')).toThrow('sequence')
  db.run('INSERT INTO blocks (node_id, idx, block_json) VALUES (?, ?, ?)', [
    'node',
    1,
    stringifyPortableJson(block),
  ])
  expect(() => readNode(db, 'node')).toThrow('sequence')
  db.run('UPDATE blocks SET idx = 0 WHERE node_id = ?', ['node'])
  expect(readNode(db, 'node')?.blocks).toEqual([block])
  db.run('UPDATE blocks SET block_json = ? WHERE node_id = ?', ['{"type":"user"}', 'node'])
  expect(() => readNode(db, 'node')).toThrow()
  db.run('UPDATE nodes SET block_count = 0 WHERE id = ?', ['node'])
  expect(() => readNode(db, 'node')).toThrow('sequence')
  expect(readNode(db, 'missing')).toBeNull()
})

test('blob reads distinguish absent content, malformed keys and corrupt bytes', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-blob-contract-'))
  cleanup.push(() => rm(root, { recursive: true, force: true }))
  const blobs = new DirBlobStore(root)
  const key = await blobs.put(new Uint8Array([1, 2, 3]))
  expect(await blobs.get(key)).toEqual(new Uint8Array([1, 2, 3]))
  await expect(blobs.get('invalid')).rejects.toThrow()
  expect(await blobs.get('0'.repeat(64))).toBeNull()
  await writeFile(join(root, key), new Uint8Array([9]))
  await expect(blobs.get(key)).rejects.toThrow('digest')
})

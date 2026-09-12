import { editRequestSchema } from '@demicodes/agent'
import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { AgentSession, type EditRequest } from '@demicodes/agent'
import { parsePortableJson } from '@demicodes/utils'
import { model, text } from '../../../agent/src/__tests__/helpers'
import { editStorageFixture } from './fixtures/edit-crash'
import { openSqliteDatabase } from '../storage/database'
import { sqliteAgentTreeStore } from '../storage/tree-store'
import { DirBlobStore } from '../storage/blob-store'

test('SQL failure after replacement rows and suffix deletion rolls back blocks, state, media and receipt', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-edit-sql-'))
  let armed = false
  let deleted = false
  const fixture = await editStorageFixture(root, (db) => ({
    ...db,
    run(sql, params) {
      db.run(sql, params)
      if (armed && sql.startsWith('DELETE FROM blocks')) deleted = true
      if (armed && sql.startsWith('UPDATE nodes SET state_json')) throw new Error('injected SQL failure')
    },
  }))
  try {
    armed = true
    const replacement = { ...fixture.before.transcript.blocks[3]!, content: text('B-edited') }
    await expect(fixture.store.save({
      changedBlocks: [{ index: 3, block: replacement }], blockCount: 4,
      state: { value: 'new-state' }, phase: 'running', queue: [], model, cwd: root,
      harnessName: 'crash-test', edits: [{ operationId: 'edit', digest: '0'.repeat(64), turnId: 'new-turn' }],
    })).rejects.toThrow('injected SQL failure')
    expect(deleted).toBe(true)
    expect(await fixture.store.load()).toEqual(fixture.before)
    fixture.raw.close()
    const reopened = openSqliteDatabase(join(root, 'conversation.sqlite'))
    try {
      expect(await sqliteAgentTreeStore(reopened, fixture.blobs).sessionStore('root').load()).toEqual(fixture.before)
    } finally {
      reopened.close()
    }
  } finally {
    fixture.raw.close()
    await rm(root, { recursive: true, force: true })
  }
})

test('blob failure leaves the existing checkpoint and retained attachment bytes readable', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-edit-blob-'))
  const fixture = await editStorageFixture(root)
  try {
    const store = sqliteAgentTreeStore(fixture.db, {
      get: (id) => fixture.blobs.get(id),
      put: async () => { throw new Error('blob write failed') },
    }).sessionStore('root')
    await expect(store.save({
      changedBlocks: [{ index: 0, block: fixture.before.transcript.blocks[0]! }],
      blockCount: 1, state: {}, phase: 'running', queue: [], model, cwd: root, harnessName: 'crash-test',
    })).rejects.toThrow('blob write failed')
    expect(await fixture.store.load()).toEqual(fixture.before)
  } finally {
    fixture.raw.close()
    await rm(root, { recursive: true, force: true })
  }
})

for (const boundary of ['inside-transaction', 'after-commit', 'after-output']) {
  test(`SIGKILL ${boundary} recovers one complete checkpoint without graceful disposal`, async () => {
    const root = await mkdtemp(join(tmpdir(), 'demi-edit-crash-'))
    const child = Bun.spawn([
      process.execPath, '--conditions', 'development',
      join(import.meta.dir, 'fixtures/edit-crash.ts'), root, boundary,
    ], { stdout: 'pipe', stderr: 'pipe' })
    try {
      await child.exited
      expect(child.signalCode).toBe('SIGKILL')
      const db = openSqliteDatabase(join(root, 'conversation.sqlite'))
      try {
        const store = sqliteAgentTreeStore(db, new DirBlobStore(join(root, 'blobs'))).sessionStore('root')
        const checkpoint = (await store.load())!
        const texts = checkpoint.transcript.blocks.filter((block) => block.type === 'user').map((block) => block.content[0])
        expect(texts).toEqual((boundary === 'inside-transaction' ? ['A', 'B', 'C'] : ['A', 'B-edited']).flatMap(text))
        expect(checkpoint.state).toEqual({ value: boundary === 'inside-transaction' ? 'old-state' : 'new-state' })
        expect(checkpoint.edits?.length ?? 0).toBe(boundary === 'inside-transaction' ? 0 : 1)
        expect(checkpoint.transcript.blocks.some((block) => block.type === 'text' && block.text === 'checkpointed-output')).toBe(boundary === 'after-output')
        if (boundary !== 'inside-transaction') {
          let calls = 0
          const session = AgentSession.fromCheckpoint({
            checkpoint,
            provider: { clone() { throw new Error('unexpected clone') }, async *run() { calls += 1 } },
            runtime: { harnessName: 'crash-test', initialState: () => ({}), systemPrompt: () => '', tools: () => [] },
          }, { store })
          try {
            expect(session.transcript().blocks).toEqual(checkpoint.transcript.blocks)
            expect(calls).toBe(0)
            const request = editRequestSchema.parse(parsePortableJson(await Bun.file(join(root, 'request.json')).text()))
            await expect(session.editAndSend(request)).resolves.toEqual(checkpoint.edits![0])
            expect(calls).toBe(0)
            if (boundary === 'after-commit') {
              await session.resume()
              expect(calls).toBe(1)
            }
          } finally {
            await session.dispose()
          }
        }
      } finally {
        db.close()
      }
    } finally {
      child.kill()
      await child.exited
      await rm(root, { recursive: true, force: true })
    }
  }, 20_000)
}

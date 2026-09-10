import { mkdtemp, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { expect, test } from 'bun:test'
import { deferred } from '@demicodes/utils'
import { editStorageFixture } from './fixtures/edit-crash'
import { sqliteAgentTreeStore } from '../storage/tree-store'

test('command snapshot, head and transcript boundary roll back together on SQL failure', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-command-sql-'))
  let armed = false
  const f = await editStorageFixture(root, (db) => ({
    ...db,
    run(sql, params) {
      db.run(sql, params)
      if (armed && sql.startsWith('INSERT INTO session_boundaries')) {
        throw new Error('boundary failed')
      }
    },
  }))
  try {
    const before = f.before
    const state = structuredClone(before.commandState)
    state.versions.push({ revision: 1, values: { 'todos.json': ['pending'] } })
    state.revision = 1
    const update = {
      ...before, changedBlocks: [], blockCount: before.transcript.blocks.length,
      commandState: state,
    }
    armed = true
    await expect(f.store.save(update)).rejects.toThrow('boundary failed')
    expect(await f.store.load()).toEqual(before)
    armed = false
    await f.store.save(update)
    expect((await f.store.load())!.commandState.revision).toBe(1)
    state.versions[1]!.values = { changed: true }
    await expect(f.store.save(update)).rejects.toThrow('immutable')
    expect((await f.store.load())!.commandState.versions[1]!.values).toEqual({ 'todos.json': ['pending'] })
  } finally {
    f.raw.close()
    await rm(root, { recursive: true, force: true })
  }
})

test('cancellation during media IO prevents the command-state transaction', async () => {
  const root = await mkdtemp(join(tmpdir(), 'demi-command-cancel-'))
  const f = await editStorageFixture(root)
  const entered = deferred<void>()
  const release = deferred<void>()
  const controller = new AbortController()
  try {
    const store = sqliteAgentTreeStore(f.db, {
      get: (id) => f.blobs.get(id),
      async put(bytes) {
        entered.resolve()
        await release.promise
        return f.blobs.put(bytes)
      },
    }).sessionStore('root')
    const state = structuredClone(f.before.commandState)
    state.versions.push({ revision: 1, values: { value: 1 } })
    state.revision = 1
    const saved = Promise.resolve(store.save({
      ...f.before,
      changedBlocks: [{ index: 0, block: f.before.transcript.blocks[0]! }],
      blockCount: f.before.transcript.blocks.length,
      commandState: state,
    }, { signal: controller.signal })).catch((error) => error)
    await entered.promise
    controller.abort()
    release.resolve()
    expect(await saved).toBeInstanceOf(Error)
    expect(await f.store.load()).toEqual(f.before)
  } finally {
    release.resolve()
    f.raw.close()
    await rm(root, { recursive: true, force: true })
  }
})

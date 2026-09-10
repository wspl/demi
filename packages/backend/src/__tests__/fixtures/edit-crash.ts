import { join } from 'node:path'
import { AgentSession, TranscriptLog } from '@demicodes/agent'
import { model, text, commandStateFor } from '../../../../agent/src/__tests__/helpers'
import type { AgentProvider } from '@demicodes/provider'
import { openSqliteDatabase, type SqlDatabase } from '../../storage/database'
import { CONVERSATION_MIGRATIONS, migrate } from '../../storage/migrations'
import { sqliteAgentTreeStore } from '../../storage/tree-store'
import { DirBlobStore } from '../../storage/blob-store'
import { stringifyPortableJson } from '@demicodes/utils'

export async function editStorageFixture(root: string, wrap: (db: SqlDatabase) => SqlDatabase = (db) => db) {
  const raw = openSqliteDatabase(join(root, 'conversation.sqlite'))
  migrate(raw, CONVERSATION_MIGRATIONS)
  const db = wrap(raw)
  const blobs = new DirBlobStore(join(root, 'blobs'))
  const tree = sqliteAgentTreeStore(db, blobs)
  const transcript = new TranscriptLog()
  for (const letter of ['A', 'B', 'C']) {
    transcript.pushUserTurn(`turn-${letter}`, model, [
      ...text(letter),
      { type: 'image', source: { type: 'binary', data: new Uint8Array([letter.charCodeAt(0)]), mediaType: 'image/png' } },
    ])
    transcript.applyProviderEvent(model, { type: 'text_delta', text: `answer-${letter}` })
    transcript.applyProviderEvent(model, {
      type: 'response', usage: { inputTokens: 3, outputTokens: 2, cacheReadTokens: 0, cacheWriteTokens: 0 },
    })
  }
  await tree.createNode({
    id: 'root', parentId: null, description: '', profileName: null, metadata: null,
    spawnedAt: 1, canSpawnSubagents: true, closedPhase: null, closedAt: null,
    result: null, failure: null, delivered: false,
  }, {
    commandState: commandStateFor(transcript.blocks),
    changedBlocks: transcript.blocks.map((block, index) => ({ index, block })),
    blockCount: transcript.blocks.length, state: { value: 'old-state' }, phase: 'idle',
    queue: [], cwd: root, model, harnessName: 'crash-test',
  })
  const store = tree.sessionStore('root')
  return { raw, db, blobs, tree, store, before: (await store.load())! }
}

function crash(): never {
  process.kill(process.pid, 'SIGKILL')
  throw new Error('SIGKILL did not terminate the fixture')
}

if (import.meta.main) {
  const [root, boundary] = process.argv.slice(2)
  let armed = false
  const fixture = await editStorageFixture(root!, (db) => ({
    ...db,
    run(sql, params) {
      db.run(sql, params)
      if (armed && boundary === 'inside-transaction' && sql.startsWith('DELETE FROM blocks')) {
        crash()
      }
    },
  }))
  const provider = (): AgentProvider => ({
    clone: provider,
    async *run() {
      yield { type: 'text_delta', text: 'checkpointed-output' }
      yield { type: 'response', usage: { inputTokens: 3, outputTokens: 2, cacheReadTokens: 0, cacheWriteTokens: 0 } }
    },
  })
  const session = AgentSession.fromCheckpoint({
    provider: provider(), checkpoint: fixture.before,
    runtime: {
      harnessName: 'crash-test', initialState: () => ({}),
      restoreState: () => ({ value: 'new-state' }),
      systemPrompt: () => 'test', tools: () => [],
    },
  }, {
    store: {
      load: () => fixture.store.load(),
      async save(update) {
        await fixture.store.save(update)
        if (update.edits?.length && boundary === 'after-commit') crash()
        if (boundary === 'after-output' && update.changedBlocks.some(({ block }) =>
          block.type === 'text' && block.text === 'checkpointed-output',
        )) crash()
      },
    },
    compaction: { preflightThresholdRatio: Number.POSITIVE_INFINITY },
  })
  armed = true
  const request = {
    operationId: 'crash-edit', targetBlockId: fixture.before.transcript.blocks[3]!.id,
    version: session.transcript().version(), content: text('B-edited'),
  }
  await Bun.write(join(root!, 'request.json'), stringifyPortableJson(request))
  await session.editAndSend(request)
  await session.waitUntilDone()
  throw new Error('Fixture did not reach its crash boundary')
}

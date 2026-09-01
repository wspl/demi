import { join } from 'node:path'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { Block } from '@demicodes/core'
import type { HostStore } from '@demicodes/shell'
import {
  externalizeBlockMedia,
  rehydrateBlockMedia,
  type AgentSessionCheckpoint,
  type AgentSessionPersistUpdate,
  type AgentSessionStore,
  type BlobStore,
  type PersistedSessionState,
} from '@demicodes/agent'
import { openSqliteDatabase, type SqlDatabase } from './database'
import { DbHostStore } from './host-store'
import { CONVERSATION_MIGRATIONS, migrate } from './migrations'

type PersistedState = Omit<PersistedSessionState<unknown>, 'blockCount'>

/**
 * The per-conversation databases: `conversations/<id>.sqlite`, one file per
 * conversation, holding the block-row transcript, the session state row, and
 * that conversation's host_store scope. Each file has exactly one writer
 * (this process); handles are opened on demand and live until `close`.
 */
export class ConversationStores {
  private readonly open = new Map<string, SqlDatabase>()

  constructor(
    private readonly root: string,
    private readonly blobs: BlobStore,
  ) {}

  db(conversationId: string): SqlDatabase {
    const existing = this.open.get(conversationId)
    if (existing) return existing
    if (!/^[A-Za-z0-9_-]+$/.test(conversationId)) {
      throw new Error(`ConversationStores: invalid conversation id "${conversationId}"`)
    }
    const db = openSqliteDatabase(join(this.root, `${conversationId}.sqlite`))
    migrate(db, CONVERSATION_MIGRATIONS)
    this.open.set(conversationId, db)
    return db
  }

  sessionStore(conversationId: string): AgentSessionStore<unknown> {
    const db = this.db(conversationId)
    const blobs = this.blobs
    return {
      async save(update: AgentSessionPersistUpdate<unknown>): Promise<void> {
        const rows = await Promise.all(
          update.changedBlocks.map(async ({ index, block }) => ({
            index,
            json: stringifyPortableJson(await externalizeBlockMedia(block, blobs)),
          })),
        )
        const state: PersistedState = {
          state: update.state,
          phase: update.phase,
          queue: update.queue,
          cwd: update.cwd,
          model: update.model,
          harnessName: update.harnessName,
        }
        db.transaction(() => {
          for (const { index, json } of rows) {
            db.run(
              'INSERT INTO blocks (idx, block_json) VALUES (?, ?) ON CONFLICT (idx) DO UPDATE SET block_json = excluded.block_json',
              [index, json],
            )
          }
          db.run('DELETE FROM blocks WHERE idx >= ?', [update.blockCount])
          db.run(
            'INSERT INTO session (id, state_json, block_count) VALUES (1, ?, ?) ON CONFLICT (id) DO UPDATE SET state_json = excluded.state_json, block_count = excluded.block_count',
            [stringifyPortableJson(state), update.blockCount],
          )
        })
      },

      load: async (): Promise<AgentSessionCheckpoint<unknown> | null> => {
        const loaded = this.readSession(conversationId)
        if (!loaded) return null
        const blocks = await Promise.all(loaded.blocks.map((block) => rehydrateBlockMedia(block, blobs)))
        return { ...loaded.state, transcript: { blocks } }
      },
    }
  }

  hostStore(conversationId: string): HostStore {
    return new DbHostStore(this.db(conversationId), 'host')
  }

  /** Cold transcript read: the raw rows, media left as refs. */
  transcriptBlocks(conversationId: string): Block[] {
    return this.readSession(conversationId)?.blocks ?? []
  }

  close(): void {
    for (const db of this.open.values()) db.close()
    this.open.clear()
  }

  private readSession(conversationId: string): { state: PersistedState; blocks: Block[] } | null {
    const db = this.db(conversationId)
    const session = db.get<{ state_json: string; block_count: number }>(
      'SELECT state_json, block_count FROM session WHERE id = 1',
    )
    if (!session) return null
    const rows = db.all<{ block_json: string }>('SELECT block_json FROM blocks WHERE idx < ? ORDER BY idx', [
      session.block_count,
    ])
    return {
      state: parsePortableJson<PersistedState>(session.state_json),
      blocks: rows.map((row) => parsePortableJson<Block>(row.block_json)),
    }
  }
}

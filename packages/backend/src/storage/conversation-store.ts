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
import type { VirtualFsBackend } from '@demicodes/host-virtual'
import { openSqliteDatabase, type SqlDatabase, type SqlParams } from './database'
import { clearFilesTree, filesTreeBackend, materializeFilesTree, type TreePlacement } from './files-tree'
import { DbHostStore } from './host-store'
import { CONVERSATION_MIGRATIONS, migrate } from './migrations'

type PersistedState = Omit<PersistedSessionState<unknown>, 'blockCount'>

/**
 * The per-conversation databases: `conversations/<id>.sqlite`, one file per
 * conversation, holding the block-row transcript, the session state row, and
 * that conversation's host_store scope. Each file has exactly one writer
 * (this process). The database object handed out for a conversation is
 * stable; the SQLite handle behind it opens on first use and is one of at
 * most `maxOpen` kept open, the least recently used closed first — so a
 * cold transcript read holds a handle only until other conversations are
 * touched, and a conversation in use is always the most recent.
 */
export class ConversationStores {
  private readonly databases = new Map<string, SqlDatabase>()
  /** Insertion order is recency: a use re-inserts. */
  private readonly handles = new Map<string, SqlDatabase>()
  private readonly maxOpen: number

  constructor(
    private readonly root: string,
    private readonly blobsFor: (conversationId: string) => BlobStore,
    options: { maxOpen?: number } = {},
  ) {
    this.maxOpen = options.maxOpen ?? 64
  }

  /** The conversation's database: a stable object whose handle opens on demand. */
  db(conversationId: string): SqlDatabase {
    const existing = this.databases.get(conversationId)
    if (existing) return existing
    if (!/^[A-Za-z0-9_-]+$/.test(conversationId)) {
      throw new Error(`ConversationStores: invalid conversation id "${conversationId}"`)
    }
    const handle = () => this.handle(conversationId)
    const db: SqlDatabase = {
      run: (sql, params) => handle().run(sql, params),
      all: <T>(sql: string, params?: SqlParams) => handle().all<T>(sql, params),
      get: <T>(sql: string, params?: SqlParams) => handle().get<T>(sql, params),
      transaction: <T>(fn: () => T) => handle().transaction(fn),
      close: () => this.release(conversationId),
    }
    this.databases.set(conversationId, db)
    return db
  }

  /** Handles open right now (diagnostics and tests). */
  get openHandles(): number {
    return this.handles.size
  }

  sessionStore(conversationId: string): AgentSessionStore<unknown> {
    const db = this.db(conversationId)
    const blobs = this.blobsFor(conversationId)
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

  /** The hostless filesystem: the conversation's `files` tree over the blob store. */
  filesBackend(conversationId: string): VirtualFsBackend {
    return filesTreeBackend(this.db(conversationId), this.blobsFor(conversationId))
  }

  /** The tree written into real directories for the home image, then emptied (`storage.md` § The upgrade). */
  async materializeFiles(conversationId: string, placements: readonly TreePlacement[]): Promise<void> {
    await materializeFilesTree(this.db(conversationId), this.blobsFor(conversationId), placements)
  }

  clearFiles(conversationId: string): void {
    clearFilesTree(this.db(conversationId))
  }

  /** Cold transcript read: the raw rows, media left as refs. */
  transcriptBlocks(conversationId: string): Block[] {
    return this.readSession(conversationId)?.blocks ?? []
  }

  close(): void {
    for (const handle of this.handles.values()) handle.close()
    this.handles.clear()
  }

  private handle(conversationId: string): SqlDatabase {
    const open = this.handles.get(conversationId)
    if (open) {
      this.handles.delete(conversationId)
      this.handles.set(conversationId, open)
      return open
    }
    const opened = openSqliteDatabase(join(this.root, `${conversationId}.sqlite`))
    migrate(opened, CONVERSATION_MIGRATIONS)
    this.handles.set(conversationId, opened)
    while (this.handles.size > this.maxOpen) {
      const oldest = this.handles.keys().next().value
      if (oldest === undefined) break
      this.release(oldest)
    }
    return opened
  }

  private release(conversationId: string): void {
    const open = this.handles.get(conversationId)
    if (!open) return
    this.handles.delete(conversationId)
    open.close()
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

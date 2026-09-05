import { join } from 'node:path'
import type { Block } from '@demicodes/core'
import type { HostStore } from '@demicodes/shell'
import type { AgentTreeStore, BlobStore } from '@demicodes/agent'
import type { VirtualFsBackend } from '@demicodes/host-virtual'
import { openSqliteDatabase, type SqlDatabase, type SqlParams } from './database'
import { clearFilesTree, filesTreeBackend, materializeFilesTree, type TreePlacement } from './files-tree'
import { DbHostStore } from './host-store'
import { CONVERSATION_MIGRATIONS, migrate } from './migrations'
import { readNode, sqliteAgentTreeStore } from './tree-store'

/**
 * The per-conversation databases: `conversations/<id>.sqlite`, one file per
 * conversation, holding the session tree — its node rows and their block-row
 * transcripts — and that conversation's host_store scope. Each file has exactly one writer
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

  /** The conversation's session tree (`subagent.md` § Persistence); the root node's id is the conversation's. */
  treeStore(conversationId: string): AgentTreeStore<unknown> {
    return sqliteAgentTreeStore(this.db(conversationId), this.blobsFor(conversationId))
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

  /** Cold transcript read of the root node: the raw rows, media left as refs. */
  transcriptBlocks(conversationId: string): Block[] {
    return readNode(this.db(conversationId), conversationId)?.blocks ?? []
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
}

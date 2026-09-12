import { sessionPhaseSchema } from '@demicodes/agent'
import { join } from 'node:path'
import { z } from 'zod'
import { parsePortableJson } from '@demicodes/utils'
import type { StoredBlock } from '@demicodes/agent'
import type { HostStore } from '@demicodes/shell'
import type { AgentTreeStore, BlobStore } from '@demicodes/agent'
import { openSqliteDatabase, type SqlDatabase, type SqlParams } from './database'
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

  /**
   * The conversation's database: a stable object whose handle opens on demand.
   */
  db(conversationId: string): SqlDatabase {
    const existing = this.databases.get(conversationId)
    if (existing) {
      return existing
    }
    if (!/^[A-Za-z0-9_-]+$/.test(conversationId)) {
      throw new Error(
        `ConversationStores: invalid conversation id "${conversationId}"`,
      )
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

  /**
   * The conversation's session tree (`subagent.md` § Persistence); the root
   * node's id is the conversation's.
   */
  treeStore(conversationId: string): AgentTreeStore<unknown> {
    return sqliteAgentTreeStore(
      this.db(conversationId),
      this.blobsFor(conversationId),
    )
  }

  hostStore(conversationId: string): HostStore {
    return new DbHostStore(this.db(conversationId), 'host')
  }

  /**
   * Cold transcript read of the root node: the raw rows, media left as refs.
   */
  transcriptBlocks(conversationId: string): StoredBlock[] {
    return readNode(this.db(conversationId), conversationId)?.blocks ?? []
  }

  /** All descendant histories, retaining blob references for the browser. */
  async subagentHistory(conversationId: string) {
    const store = this.treeStore(conversationId)
    const parents = [conversationId]
    const agents = []
    for (const parent of parents) {
      for (const node of await store.children(parent)) {
        parents.push(node.id)
        agents.push({
          id: node.id,
          name: node.description,
          phase: node.closedPhase ?? ('running' as const),
          startedAt: new Date(node.spawnedAt).toISOString(),
          endedAt:
            node.closedAt === null ? null : new Date(node.closedAt).toISOString(),
          blocks: readNode(this.db(conversationId), node.id)?.blocks ?? [],
        })
      }
    }
    return agents
  }

  summary(conversationId: string) {
    const db = this.db(conversationId)
    const node = db.get<{
      state_json: string
      output_revision: number
    }>('SELECT state_json, output_revision FROM nodes WHERE id = ?', [
      conversationId,
    ])
    const phase = node
      ? z
          .object({ phase: sessionPhaseSchema })
          .parse(parsePortableJson(node.state_json)).phase
      : 'idle'
    const terminal = db.get<{ block_json: string }>(
      "SELECT block_json FROM blocks WHERE node_id = ? AND json_extract(block_json, '$.type') IN ('response', 'error', 'abort') ORDER BY idx DESC LIMIT 1",
      [conversationId],
    )
    const last = terminal
      ? z
          .object({ type: z.enum(['response', 'error', 'abort']) })
          .parse(parsePortableJson(terminal.block_json)).type
      : null
    return {
      phase,
      revision: node ? z.number().int().nonnegative().parse(node.output_revision) : 0,
      last,
    }
  }

  close(): void {
    for (const handle of this.handles.values()) {
      handle.close()
    }
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
      if (oldest === undefined) {
        break
      }
      this.release(oldest)
    }
    return opened
  }

  private release(conversationId: string): void {
    const open = this.handles.get(conversationId)
    if (!open) {
      return
    }
    this.handles.delete(conversationId)
    open.close()
  }
}

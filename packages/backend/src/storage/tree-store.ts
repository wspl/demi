import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { Block, QueuedMessage } from '@demicodes/core'
import {
  completedChildrenCarriedBy,
  externalizeBlockMedia,
  rehydrateBlockMedia,
  type AgentMetadata,
  type AgentNodeClose,
  type AgentNodeClosePhase,
  type AgentNodeRecord,
  type AgentSessionCheckpoint,
  type AgentSessionPersistUpdate,
  type AgentSessionStore,
  type AgentTreeStore,
  type BlobStore,
} from '@demicodes/agent'
import type { SqlDatabase } from './database'

/** The checkpoint fields other than the transcript, as `nodes.state_json` holds them. */
type NodeState = Omit<AgentSessionCheckpoint<unknown>, 'transcript'>

interface NodeRow {
  id: string
  parent_id: string | null
  description: string
  profile_name: string | null
  metadata_json: string | null
  spawned_at: number
  can_spawn: number
  closed_phase: AgentNodeClosePhase | null
  closed_at: number | null
  result: string | null
  failure: string | null
  delivered: number
}

const NODE_SELECT = 'SELECT id, parent_id, description, profile_name, metadata_json, spawned_at, can_spawn, closed_phase, closed_at, result, failure, delivered FROM nodes'

/**
 * The `AgentTreeStore` over a conversation's database (`storage.md`): node
 * rows in `nodes`, each node's transcript in `blocks`, media externalized to
 * the blob store on the way in and rehydrated on the way out. Create, save
 * and close are each one transaction (`subagent.md` § Persistence).
 */
export function sqliteAgentTreeStore(db: SqlDatabase, blobs: BlobStore): AgentTreeStore<unknown> {
  const record = (row: NodeRow): AgentNodeRecord => ({
    id: row.id,
    parentId: row.parent_id,
    description: row.description,
    profileName: row.profile_name,
    metadata: row.metadata_json === null ? null : parsePortableJson<AgentMetadata>(row.metadata_json),
    spawnedAt: row.spawned_at,
    canSpawnSubagents: row.can_spawn === 1,
    closedPhase: row.closed_phase,
    closedAt: row.closed_at,
    result: row.result,
    failure: row.failure,
    delivered: row.delivered === 1,
  })
  const stateOf = (update: AgentSessionPersistUpdate<unknown>): NodeState => ({
    state: update.state,
    phase: update.phase,
    queue: update.queue,
    cwd: update.cwd,
    model: update.model,
    harnessName: update.harnessName,
  })
  // The journal write: the changed rows, the rows past the end gone, the state row, the completions this state carries.
  const writeCheckpoint = (id: string, update: AgentSessionPersistUpdate<unknown>, rows: Array<{ index: number; json: string }>): void => {
    for (const { index, json } of rows) {
      db.run(
        'INSERT INTO blocks (node_id, idx, block_json) VALUES (?, ?, ?) ON CONFLICT (node_id, idx) DO UPDATE SET block_json = excluded.block_json',
        [id, index, json],
      )
    }
    db.run('DELETE FROM blocks WHERE node_id = ? AND idx >= ?', [id, update.blockCount])
    db.run('UPDATE nodes SET state_json = ?, block_count = ? WHERE id = ?', [stringifyPortableJson(stateOf(update)), update.blockCount, id])
    for (const childId of completedChildrenCarriedBy(update)) {
      db.run('UPDATE nodes SET delivered = 1 WHERE id = ? AND parent_id = ?', [childId, id])
    }
  }
  const externalize = (update: AgentSessionPersistUpdate<unknown>) =>
    Promise.all(
      update.changedBlocks.map(async ({ index, block }) => ({ index, json: stringifyPortableJson(await externalizeBlockMedia(block, blobs)) })),
    )

  return {
    async node(id) {
      const row = db.get<NodeRow>(`${NODE_SELECT} WHERE id = ?`, [id])
      return row ? record(row) : null
    },

    async children(parentId) {
      return db.all<NodeRow>(`${NODE_SELECT} WHERE parent_id = ? ORDER BY spawned_at, id`, [parentId]).map(record)
    },

    async createNode(node, checkpoint) {
      const rows = await externalize(checkpoint)
      db.transaction(() => {
        db.run(
          'INSERT INTO nodes (id, parent_id, description, profile_name, metadata_json, spawned_at, can_spawn, closed_phase, closed_at, result, failure, delivered, state_json, block_count) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
          [
            node.id,
            node.parentId,
            node.description,
            node.profileName,
            node.metadata === null ? null : stringifyPortableJson(node.metadata),
            node.spawnedAt,
            node.canSpawnSubagents ? 1 : 0,
            node.closedPhase,
            node.closedAt,
            node.result,
            node.failure,
            node.delivered ? 1 : 0,
            stringifyPortableJson(stateOf(checkpoint)),
            0,
          ],
        )
        writeCheckpoint(node.id, checkpoint, rows)
      })
    },

    sessionStore(id): AgentSessionStore<unknown> {
      return {
        save: async (update) => {
          const rows = await externalize(update)
          db.transaction(() => writeCheckpoint(id, update, rows))
        },
        load: async () => {
          const loaded = readNode(db, id)
          if (!loaded) return null
          const blocks = await Promise.all(loaded.blocks.map((block) => rehydrateBlockMedia(block, blobs)))
          return { ...loaded.state, transcript: { blocks } }
        },
      }
    },

    async closeNode(id, close: AgentNodeClose) {
      db.run('UPDATE nodes SET closed_phase = ?, closed_at = ?, result = ?, failure = ?, delivered = 0 WHERE id = ?', [
        close.phase,
        close.closedAt,
        close.result,
        close.failure,
        id,
      ])
    },

    async reopenNode(id, fields, message: QueuedMessage) {
      db.transaction(() => {
        const row = db.get<{ state_json: string }>('SELECT state_json FROM nodes WHERE id = ?', [id])
        if (!row) throw new Error(`no node "${id}" to reopen`)
        const state = parsePortableJson<NodeState>(row.state_json)
        db.run(
          'UPDATE nodes SET metadata_json = ?, spawned_at = ?, closed_phase = NULL, closed_at = NULL, result = NULL, failure = NULL, delivered = 0, state_json = ? WHERE id = ?',
          [fields.metadata === null ? null : stringifyPortableJson(fields.metadata), fields.spawnedAt, stringifyPortableJson({ ...state, queue: [message] }), id],
        )
      })
    },

    async markDelivered(id) {
      db.run('UPDATE nodes SET delivered = 1 WHERE id = ?', [id])
    },

    async deleteNode(id) {
      // Descendants and their blocks go through the cascades.
      db.run('DELETE FROM nodes WHERE id = ?', [id])
    },
  }
}

/** A node's rows as stored: the state and the raw blocks, media left as refs (cold reads need none). */
export function readNode(db: SqlDatabase, id: string): { state: NodeState; blocks: Block[] } | null {
  const row = db.get<{ state_json: string; block_count: number }>('SELECT state_json, block_count FROM nodes WHERE id = ?', [id])
  if (!row) return null
  const rows = db.all<{ block_json: string }>('SELECT block_json FROM blocks WHERE node_id = ? AND idx < ? ORDER BY idx', [id, row.block_count])
  return { state: parsePortableJson<NodeState>(row.state_json), blocks: rows.map((block) => parsePortableJson<Block>(block.block_json)) }
}

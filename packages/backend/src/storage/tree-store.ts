import { z } from 'zod'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { QueuedMessage } from '@demicodes/core'
import {
  completedChildrenCarriedBy,
  metadataSchema,
  sessionStateSchema,
  storedBlockSchema,
  type StoredBlock,
  emptyCommandState,
  externalizeBlockMedia,
  rehydrateBlockMedia,
  type AgentNodeClose,
  type AgentNodeRecord,
  type AgentSessionPersistUpdate,
  type AgentSessionStore,
  type AgentTreeStore,
  type BlobStore,
} from '@demicodes/agent'
import type { SqlDatabase } from './database'
import { readCommandState, writeCommandState } from './command-state'

/**
 * The checkpoint fields other than the transcript, as `nodes.state_json` holds
 * them.
 */
type NodeState = z.infer<typeof sessionStateSchema>
const nodeRowSchema = z.object({
  id: z.string().min(1),
  parent_id: z.string().min(1).nullable(),
  description: z.string(),
  profile_name: z.string().nullable(),
  metadata_json: z.string().nullable(),
  spawned_at: z.number().int().nonnegative().max(8.64e15),
  can_spawn: z.union([z.literal(0), z.literal(1)]),
  closed_phase: z.enum(['completed', 'aborted', 'error']).nullable(),
  closed_at: z.number().int().nonnegative().max(8.64e15).nullable(),
  result: z.string().nullable(),
  failure: z.string().nullable(),
  delivered: z.union([z.literal(0), z.literal(1)]),
})
type NodeRow = z.infer<typeof nodeRowSchema>

const NODE_SELECT = 'SELECT id, parent_id, description, profile_name, metadata_json, spawned_at, can_spawn, closed_phase, closed_at, result, failure, delivered FROM nodes'

/**
 * The `AgentTreeStore` over a conversation's database (`storage.md`): node
 * rows in `nodes`, each node's transcript in `blocks`, media externalized to
 * the blob store on the way in and rehydrated on the way out. Create, save
 * and close are each one transaction (`subagent.md` § Persistence).
 */
export function sqliteAgentTreeStore(
  db: SqlDatabase,
  blobs: BlobStore
): AgentTreeStore<unknown> {
  const record = (input: NodeRow): AgentNodeRecord => {
    const row = nodeRowSchema.parse(input)
    return {
      id: row.id,
      parentId: row.parent_id,
      description: row.description,
      profileName: row.profile_name,
      metadata: row.metadata_json === null
        ? null
        : metadataSchema.parse(parsePortableJson(row.metadata_json)),
      spawnedAt: row.spawned_at,
      canSpawnSubagents: row.can_spawn === 1,
      closedPhase: row.closed_phase,
      closedAt: row.closed_at,
      result: row.result,
      failure: row.failure,
      delivered: row.delivered === 1,
    }
  }
  const stateOf = (update: AgentSessionPersistUpdate<unknown>): NodeState => ({
    state: update.state,
    phase: update.phase,
    queue: update.queue,
    cwd: update.cwd,
    model: update.model,
    harnessName: update.harnessName,
    ...(update.edits ? { edits: update.edits } : {}),
  })
  // The journal write: the changed rows, the rows past the end gone, the state row, the completions this state carries.
  const writeCheckpoint = (
    id: string,
    update: AgentSessionPersistUpdate<unknown>,
    rows: Array<{
      index: number;
      json: string
    }>
  ): void => {
    if (update.commandState) {
      writeCommandState(db, id, update.commandState)
    }
    for (const { index, json } of rows) {
      db.run(
        'INSERT INTO blocks (node_id, idx, block_json) VALUES (?, ?, ?) ON CONFLICT (node_id, idx) DO UPDATE SET block_json = excluded.block_json',
        [id, index, json],
      )
    }
    db.run(
      'DELETE FROM blocks WHERE node_id = ? AND idx >= ?',
      [id, update.blockCount]
    )
    const outputChanged = update.changedBlocks.some(({ block }) => ![
      'user',
      'steer',
      'resume',
      'extension_state_snapshot'
    ].includes(block.type))
    db.run(
      'UPDATE nodes SET state_json = ?, block_count = ?, output_revision = output_revision + CASE WHEN block_count > ? OR ? THEN 1 ELSE 0 END WHERE id = ?',
      [
        stringifyPortableJson(stateOf(update)),
        update.blockCount,
        update.blockCount,
        outputChanged ? 1 : 0,
        id
      ]
    )
    for (const childId of completedChildrenCarriedBy(update)) {
      db.run(
        'UPDATE nodes SET delivered = 1 WHERE id = ? AND parent_id = ?',
        [childId, id]
      )
    }
  }
  const externalize = (update: AgentSessionPersistUpdate<unknown>) =>
  Promise.all(
    update.changedBlocks.map(async ({ index, block }) => ({
      index,
      json: stringifyPortableJson(await externalizeBlockMedia(block, blobs))
    })),
  )

  return {
    async node(id) {
      const row = db.get<NodeRow>(`${NODE_SELECT} WHERE id = ?`, [id])
      return row ? record(row) : null
    },

    async children(parentId) {
      return db.all<NodeRow>(
        `${NODE_SELECT} WHERE parent_id = ? ORDER BY spawned_at, id`,
        [parentId]
      )
        .map(record)
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
            node.metadata === null
              ? null
              : stringifyPortableJson(node.metadata),
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
        writeCheckpoint(node.id, {
          ...checkpoint,
          commandState: checkpoint.commandState ?? emptyCommandState(),
        }, rows)
      })
    },

    sessionStore(id): AgentSessionStore<unknown> {
      return {
        save: async (update, options) => {
          const rows = await externalize(update)
          options?.signal?.throwIfAborted()
          db.transaction(() => writeCheckpoint(id, update, rows))
        },
        load: async () => {
          const loaded = readNode(db, id)
          if (!loaded)
            return null
          const commandState = readCommandState(db, id)
          const blocks = await Promise.all(
            loaded.blocks.map((block) => rehydrateBlockMedia(block, blobs))
          )
          return { ...loaded.state, commandState, transcript: { blocks } }
        },
      }
    },

    async closeNode(id, close: AgentNodeClose) {
      db.run(
        'UPDATE nodes SET closed_phase = ?, closed_at = ?, result = ?, failure = ?, delivered = 0 WHERE id = ?',
        [
          close.phase,
          close.closedAt,
          close.result,
          close.failure,
          id,
        ]
      )
    },

    async reopenNode(id, fields, message: QueuedMessage) {
      db.transaction(() => {
        const row = db.get<{ state_json: string }>(
          'SELECT state_json FROM nodes WHERE id = ?',
          [id]
        )
        if (!row)
          throw new Error(`no node "${id}" to reopen`)
        const state = sessionStateSchema.parse(parsePortableJson(row.state_json))
        db.run(
          'UPDATE nodes SET metadata_json = ?, spawned_at = ?, closed_phase = NULL, closed_at = NULL, result = NULL, failure = NULL, delivered = 0, state_json = ? WHERE id = ?',
          [
            fields.metadata === null
              ? null
              : stringifyPortableJson(fields.metadata),
            fields.spawnedAt,
            stringifyPortableJson({ ...state, queue: [message] }),
            id
          ],
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

/**
 * A node's rows as stored: the state and the raw blocks, media left as refs
 * (cold reads need none).
 */
export function readNode(
  db: SqlDatabase,
  id: string
): {
  state: NodeState;
  blocks: StoredBlock[]
} | null {
  const row = db.get<{
    state_json: string;
    block_count: number
  }>('SELECT state_json, block_count FROM nodes WHERE id = ?', [id])
  if (!row)
    return null
  const count = z.number().int().nonnegative().parse(row.block_count)
  const rows = db.all<{ idx: number; block_json: string }>(
    'SELECT idx, block_json FROM blocks WHERE node_id = ? ORDER BY idx',
    [id]
  )
  if (rows.length !== count || rows.some((block, index) => block.idx !== index))
    throw new Error('Corrupt transcript block sequence')
  return {
    state: sessionStateSchema.parse(parsePortableJson(row.state_json)),
    blocks: rows.map((block) => storedBlockSchema.parse(parsePortableJson(block.block_json)))
  }
}

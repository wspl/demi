import {
  commandStateSchema,
  type CommandStateSnapshot,
  type SessionBoundary,
} from '@demicodes/agent'
import { parsePortableJson, stringifyPortableJson } from '@demicodes/utils'
import type { SqlDatabase } from './database'

/** Runs inside the node's checkpoint transaction. Existing versions are immutable. */
export function writeCommandState(db: SqlDatabase, nodeId: string, input: CommandStateSnapshot): void {
  const snapshot = commandStateSchema.parse(input)
  for (const version of snapshot.versions) {
    const valuesJson = stringifyPortableJson(version.values)
    const existing = db.get<{ values_json: string }>(
      'SELECT values_json FROM command_snapshots WHERE node_id = ? AND revision = ?',
      [nodeId, version.revision],
    )
    if (existing) {
      if (existing.values_json !== valuesJson) {
        throw new Error(`Command-state version ${version.revision} is immutable`)
      }
    } else {
      db.run(
        'INSERT INTO command_snapshots (node_id, revision, values_json) VALUES (?, ?, ?)',
        [nodeId, version.revision, valuesJson],
      )
    }
  }
  db.run('UPDATE nodes SET command_revision = ? WHERE id = ?', [snapshot.revision, nodeId])
  db.run('DELETE FROM session_boundaries WHERE node_id = ?', [nodeId])
  for (const boundary of snapshot.boundaries) {
    db.run(
      'INSERT INTO session_boundaries (node_id, block_id, edge, command_revision) VALUES (?, ?, ?, ?)',
      [nodeId, boundary.blockId, boundary.edge, boundary.commandRevision],
    )
  }
}

/** Read synchronously with the block/state rows before rehydrating media. */
export function readCommandState(db: SqlDatabase, nodeId: string): CommandStateSnapshot {
  const node = db.get<{ command_revision: number }>(
    'SELECT command_revision FROM nodes WHERE id = ?', [nodeId],
  )
  if (!node) {
    throw new Error(`No node ${nodeId} for command storage`)
  }
  const versions = db.all<{ revision: number; values_json: string }>(
    'SELECT revision, values_json FROM command_snapshots WHERE node_id = ? ORDER BY revision',
    [nodeId],
  )
  const boundaries = db.all<{
    block_id: string;
    edge: SessionBoundary['edge'];
    command_revision: number;
  }>('SELECT block_id, edge, command_revision FROM session_boundaries WHERE node_id = ? ORDER BY block_id, edge', [nodeId])
  return commandStateSchema.parse({
    revision: node.command_revision,
    versions: versions.map((version) => ({
      revision: version.revision,
      values: parsePortableJson(version.values_json),
    })),
    boundaries: boundaries.map((boundary) => ({
      blockId: boundary.block_id,
      edge: boundary.edge,
      commandRevision: boundary.command_revision,
    })),
  })
}

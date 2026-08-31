import { createId } from '@demicodes/utils'
import type { SqlDatabase } from './database'

export interface ConversationRecord {
  id: string
  userId: string
  title: string
  archived: boolean
  workspaceId: string | null
  connectionId: string | null
  modelId: string | null
  createdAt: string
  updatedAt: string
}

interface ConversationRow {
  id: string
  user_id: string
  title: string
  archived: number
  workspace_id: string | null
  connection_id: string | null
  model_id: string | null
  created_at: string
  updated_at: string
}

const SELECT = 'SELECT id, user_id, title, archived, workspace_id, connection_id, model_id, created_at, updated_at FROM conversations'

export class ConversationIndex {
  constructor(private readonly db: SqlDatabase) {}

  create(userId: string, options: { title?: string } = {}): ConversationRecord {
    const now = new Date().toISOString()
    const record: ConversationRecord = {
      id: createId(),
      userId,
      title: options.title ?? 'New conversation',
      archived: false,
      workspaceId: null,
      connectionId: null,
      modelId: null,
      createdAt: now,
      updatedAt: now,
    }
    this.db.run(
      'INSERT INTO conversations (id, user_id, title, archived, workspace_id, connection_id, model_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)',
      [record.id, userId, record.title, 0, null, null, null, now, now],
    )
    return record
  }

  get(id: string): ConversationRecord | null {
    const row = this.db.get<ConversationRow>(`${SELECT} WHERE id = ?`, [id])
    return row ? fromRow(row) : null
  }

  listForUser(userId: string, options: { archived?: boolean } = {}): ConversationRecord[] {
    const archived = options.archived ?? false
    const rows = this.db.all<ConversationRow>(
      `${SELECT} WHERE user_id = ? AND archived = ? ORDER BY updated_at DESC`,
      [userId, archived ? 1 : 0],
    )
    return rows.map(fromRow)
  }

  rename(id: string, title: string): void {
    this.db.run('UPDATE conversations SET title = ?, updated_at = ? WHERE id = ?', [title, new Date().toISOString(), id])
  }

  setArchived(id: string, archived: boolean): void {
    this.db.run('UPDATE conversations SET archived = ?, updated_at = ? WHERE id = ?', [
      archived ? 1 : 0,
      new Date().toISOString(),
      id,
    ])
  }

  setModel(id: string, connectionId: string | null, modelId: string | null): void {
    this.db.run('UPDATE conversations SET connection_id = ?, model_id = ?, updated_at = ? WHERE id = ?', [
      connectionId,
      modelId,
      new Date().toISOString(),
      id,
    ])
  }

  /** Sets the title only when it is still the creation default (first user message becomes the title). */
  defaultTitle(id: string, title: string): void {
    this.db.run('UPDATE conversations SET title = ?, updated_at = ? WHERE id = ? AND title = ?', [
      title,
      new Date().toISOString(),
      id,
      'New conversation',
    ])
  }

  touch(id: string): void {
    this.db.run('UPDATE conversations SET updated_at = ? WHERE id = ?', [new Date().toISOString(), id])
  }
}

function fromRow(row: ConversationRow): ConversationRecord {
  return {
    id: row.id,
    userId: row.user_id,
    title: row.title,
    archived: row.archived !== 0,
    workspaceId: row.workspace_id,
    connectionId: row.connection_id,
    modelId: row.model_id,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  }
}

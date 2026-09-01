import { createId } from '@demicodes/utils'
import type { SqlDatabase } from './database'

/**
 * The control-plane service contract: every read/write of `control.sqlite`
 * goes through these domain methods. One call is one atomic operation —
 * transactions never leak out of the implementation, so the same interface
 * is realizable in-process (here) and as the `demi-controld` RPC client at
 * N>1 without changing a caller.
 */
export interface ControlService {
  ensureUser(user: { id: string; username: string; role: 'master' | 'admin' | 'user' }): Promise<void>
  createConversation(userId: string, options?: { title?: string }): Promise<ConversationRecord>
  getConversation(id: string): Promise<ConversationRecord | null>
  listConversations(userId: string, options?: { archived?: boolean }): Promise<ConversationRecord[]>
  renameConversation(id: string, title: string): Promise<void>
  setConversationArchived(id: string, archived: boolean): Promise<void>
  setConversationModel(id: string, connectionId: string | null, modelId: string | null): Promise<void>
  /** Sets the title only when it is still the creation default (first user message becomes the title). */
  defaultConversationTitle(id: string, title: string): Promise<void>
  touchConversation(id: string): Promise<void>
}

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

const SELECT =
  'SELECT id, user_id, title, archived, workspace_id, connection_id, model_id, created_at, updated_at FROM conversations'

/** In-process `ControlService` over the control database. */
export class LocalControlService implements ControlService {
  constructor(private readonly db: SqlDatabase) {}

  async ensureUser(user: { id: string; username: string; role: 'master' | 'admin' | 'user' }): Promise<void> {
    this.db.run(
      'INSERT INTO users (id, username, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (id) DO NOTHING',
      [user.id, user.username, '!', user.role, new Date().toISOString()],
    )
  }

  async createConversation(userId: string, options: { title?: string } = {}): Promise<ConversationRecord> {
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

  async getConversation(id: string): Promise<ConversationRecord | null> {
    const row = this.db.get<ConversationRow>(`${SELECT} WHERE id = ?`, [id])
    return row ? fromRow(row) : null
  }

  async listConversations(userId: string, options: { archived?: boolean } = {}): Promise<ConversationRecord[]> {
    const archived = options.archived ?? false
    const rows = this.db.all<ConversationRow>(`${SELECT} WHERE user_id = ? AND archived = ? ORDER BY updated_at DESC`, [
      userId,
      archived ? 1 : 0,
    ])
    return rows.map(fromRow)
  }

  async renameConversation(id: string, title: string): Promise<void> {
    this.db.run('UPDATE conversations SET title = ?, updated_at = ? WHERE id = ?', [title, new Date().toISOString(), id])
  }

  async setConversationArchived(id: string, archived: boolean): Promise<void> {
    this.db.run('UPDATE conversations SET archived = ?, updated_at = ? WHERE id = ?', [
      archived ? 1 : 0,
      new Date().toISOString(),
      id,
    ])
  }

  async setConversationModel(id: string, connectionId: string | null, modelId: string | null): Promise<void> {
    this.db.run('UPDATE conversations SET connection_id = ?, model_id = ?, updated_at = ? WHERE id = ?', [
      connectionId,
      modelId,
      new Date().toISOString(),
      id,
    ])
  }

  async defaultConversationTitle(id: string, title: string): Promise<void> {
    this.db.run('UPDATE conversations SET title = ?, updated_at = ? WHERE id = ? AND title = ?', [
      title,
      new Date().toISOString(),
      id,
      'New conversation',
    ])
  }

  async touchConversation(id: string): Promise<void> {
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

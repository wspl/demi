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
  /** A user device by default; a managed host names its owner (`managed-hosts.md` § What a managed host is). */
  createDevice(device: {
    userId: string
    name: string
    platform: string
    tokenHash: string
    kind?: DeviceKind
    ownerConversationId?: string
    ownerWorkspaceId?: string
  }): Promise<DeviceRecord>
  getDevice(id: string): Promise<DeviceRecord | null>
  getDeviceByTokenHash(tokenHash: string): Promise<DeviceRecord | null>
  /** The managed host bound to an owner, if one was ever provisioned. */
  getManagedDevice(owner: ManagedHostOwner): Promise<DeviceRecord | null>
  countManagedDevices(userId: string): Promise<number>
  /** A managed host's token is minted fresh at every provision and wake; the row keeps only the current hash. */
  rotateDeviceToken(id: string, tokenHash: string): Promise<void>
  /** The user's paired devices — managed hosts never appear in a device list. */
  listDevices(userId: string): Promise<DeviceRecord[]>
  deleteDevice(id: string): Promise<void>
  touchDeviceSeen(id: string): Promise<void>
  /** `config` is opaque here — the vault encrypts/decrypts; storage never sees plaintext. */
  createConnection(connection: {
    ownerUserId: string | null
    type: string
    label: string
    config: string
  }): Promise<ConnectionRecord>
  getConnection(id: string): Promise<ConnectionRecord | null>
  listConnections(): Promise<ConnectionRecord[]>
  deleteConnection(id: string): Promise<void>
  appendUsage(row: Omit<UsageRow, 'id' | 'createdAt'>): Promise<void>
  listUsage(userId: string): Promise<UsageRow[]>
  /** Metadata only — the bytes live in the blob store under `sha256`. */
  createAttachment(attachment: { userId: string; mediaType: string; sizeBytes: number; sha256: string }): Promise<AttachmentRecord>
  getAttachment(id: string): Promise<AttachmentRecord | null>
  createWorkspace(workspace: { userId: string; deviceId: string; path: string; name: string }): Promise<WorkspaceRecord>
  getWorkspace(id: string): Promise<WorkspaceRecord | null>
  listWorkspaces(userId: string): Promise<WorkspaceRecord[]>
  renameWorkspace(id: string, name: string): Promise<void>
  deleteWorkspace(id: string): Promise<void>
  countConversationsInWorkspace(workspaceId: string): Promise<number>
  listConversationIdsInWorkspace(workspaceId: string): Promise<string[]>
  setConversationWorkspace(conversationId: string, workspaceId: string | null): Promise<void>
  /**
   * The target-switch write (`sessions-and-targets.md` § Switching): moves
   * the target pointers, records the switch for the next turn's announcement
   * and grants the departed device, in one compare-and-set — returns false
   * (and writes nothing) when the pointers no longer equal `from`, so
   * concurrent switches have exactly one winner.
   */
  switchConversationTarget(
    conversationId: string,
    from: ConversationTargetPointer,
    to: ConversationTargetPointer,
    pending: PendingSwitch,
    grantDeviceId: string | null,
  ): Promise<boolean>
  /** The announcement was injected; nothing is pending until the next switch. */
  clearPendingSwitch(conversationId: string): Promise<void>
  /** The grant set (`sessions-and-targets.md` § Host grants): idempotent add and remove. */
  grantHost(conversationId: string, deviceId: string): Promise<void>
  revokeHost(conversationId: string, deviceId: string): Promise<void>
  listHostGrants(conversationId: string): Promise<HostGrantRecord[]>
  isHostGranted(conversationId: string, deviceId: string): Promise<boolean>
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

export type DeviceKind = 'user' | 'managed'

export interface DeviceRecord {
  id: string
  userId: string
  /** `user`: paired through the claim flow; `managed`: a VM the backend provisioned, bound to one owner. */
  kind: DeviceKind
  name: string
  platform: string
  /** Managed hosts only: exactly one of the two owners is set. */
  ownerConversationId: string | null
  ownerWorkspaceId: string | null
  claimedAt: string
  lastSeenAt: string | null
}

/** What a managed host is bound to (`managed-hosts.md` § What a managed host is): exactly one of the two. */
export type ManagedHostOwner = { kind: 'conversation'; id: string } | { kind: 'workspace'; id: string }

export interface HostGrantRecord {
  conversationId: string
  deviceId: string
  grantedAt: string
}

export interface WorkspaceRecord {
  id: string
  userId: string
  deviceId: string
  path: string
  name: string
  createdAt: string
}

export interface ConnectionRecord {
  id: string
  ownerUserId: string | null
  type: string
  label: string
  /** Encrypted at rest (vault crypto); opaque to storage. */
  config: string
  createdAt: string
}

export interface AttachmentRecord {
  id: string
  userId: string
  mediaType: string
  sizeBytes: number
  sha256: string
  createdAt: string
}

export interface UsageRow {
  id: string
  userId: string
  conversationId: string
  connectionId: string
  modelId: string
  inputTokens: number
  outputTokens: number
  cacheReadTokens: number
  cacheWriteTokens: number
  createdAt: string
}

/**
 * Where a conversation's commands run (`sessions-and-targets.md` § The three
 * states), resolved from its record: a workspace, a session-bound managed
 * host, or nothing.
 */
export type ExecutionTarget =
  | { kind: 'hostless' }
  | { kind: 'workspace'; workspaceId: string; deviceId: string; path: string }
  | { kind: 'host'; deviceId: string }

/** The two mutually exclusive pointers on the conversation row; both null is hostless. */
export interface ConversationTargetPointer {
  workspaceId: string | null
  hostDeviceId: string | null
}

/** A switch the model has not been told about yet: consumed by the next turn's announcement. */
export interface PendingSwitch {
  from: ExecutionTarget
  to: ExecutionTarget
}

export interface ConversationRecord {
  id: string
  userId: string
  title: string
  archived: boolean
  workspaceId: string | null
  hostDeviceId: string | null
  pendingSwitch: PendingSwitch | null
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
  host_device_id: string | null
  pending_switch_json: string | null
  connection_id: string | null
  model_id: string | null
  created_at: string
  updated_at: string
}

const SELECT =
  'SELECT id, user_id, title, archived, workspace_id, host_device_id, pending_switch_json, connection_id, model_id, created_at, updated_at FROM conversations'

/** In-process `ControlService` over the control database. */
export class LocalControlService implements ControlService {
  constructor(private readonly db: SqlDatabase) {}

  async ensureUser(user: { id: string; username: string; role: 'master' | 'admin' | 'user' }): Promise<void> {
    this.db.run(
      'INSERT INTO users (id, username, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (id) DO NOTHING',
      [user.id, user.username, '!', user.role, new Date().toISOString()],
    )
  }

  async createDevice(device: {
    userId: string
    name: string
    platform: string
    tokenHash: string
    kind?: DeviceKind
    ownerConversationId?: string
    ownerWorkspaceId?: string
  }): Promise<DeviceRecord> {
    const record: DeviceRecord = {
      id: createId(),
      userId: device.userId,
      kind: device.kind ?? 'user',
      name: device.name,
      platform: device.platform,
      ownerConversationId: device.ownerConversationId ?? null,
      ownerWorkspaceId: device.ownerWorkspaceId ?? null,
      claimedAt: new Date().toISOString(),
      lastSeenAt: null,
    }
    this.db.run(
      'INSERT INTO devices (id, user_id, kind, name, platform, token_hash, owner_conversation_id, owner_workspace_id, claimed_at, last_seen_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
      [
        record.id,
        record.userId,
        record.kind,
        record.name,
        record.platform,
        device.tokenHash,
        record.ownerConversationId,
        record.ownerWorkspaceId,
        record.claimedAt,
        null,
      ],
    )
    return record
  }

  async getDevice(id: string): Promise<DeviceRecord | null> {
    const row = this.db.get<DeviceRow>(`${DEVICE_SELECT} WHERE id = ?`, [id])
    return row ? deviceFromRow(row) : null
  }

  async getDeviceByTokenHash(tokenHash: string): Promise<DeviceRecord | null> {
    const row = this.db.get<DeviceRow>(`${DEVICE_SELECT} WHERE token_hash = ?`, [tokenHash])
    return row ? deviceFromRow(row) : null
  }

  async getManagedDevice(owner: ManagedHostOwner): Promise<DeviceRecord | null> {
    const column = owner.kind === 'conversation' ? 'owner_conversation_id' : 'owner_workspace_id'
    const row = this.db.get<DeviceRow>(`${DEVICE_SELECT} WHERE kind = 'managed' AND ${column} = ?`, [owner.id])
    return row ? deviceFromRow(row) : null
  }

  async countManagedDevices(userId: string): Promise<number> {
    return this.db.get<{ n: number }>("SELECT COUNT(*) AS n FROM devices WHERE user_id = ? AND kind = 'managed'", [userId])?.n ?? 0
  }

  async rotateDeviceToken(id: string, tokenHash: string): Promise<void> {
    this.db.run('UPDATE devices SET token_hash = ? WHERE id = ?', [tokenHash, id])
  }

  async listDevices(userId: string): Promise<DeviceRecord[]> {
    const rows = this.db.all<DeviceRow>(`${DEVICE_SELECT} WHERE user_id = ? AND kind = 'user' ORDER BY claimed_at`, [userId])
    return rows.map(deviceFromRow)
  }

  async deleteDevice(id: string): Promise<void> {
    this.db.run('DELETE FROM devices WHERE id = ?', [id])
  }

  async touchDeviceSeen(id: string): Promise<void> {
    this.db.run('UPDATE devices SET last_seen_at = ? WHERE id = ?', [new Date().toISOString(), id])
  }

  async createConnection(connection: {
    ownerUserId: string | null
    type: string
    label: string
    config: string
  }): Promise<ConnectionRecord> {
    const record: ConnectionRecord = {
      id: createId(),
      ownerUserId: connection.ownerUserId,
      type: connection.type,
      label: connection.label,
      config: connection.config,
      createdAt: new Date().toISOString(),
    }
    this.db.run('INSERT INTO connections (id, owner_user_id, type, label, config, created_at) VALUES (?, ?, ?, ?, ?, ?)', [
      record.id,
      record.ownerUserId,
      record.type,
      record.label,
      record.config,
      record.createdAt,
    ])
    return record
  }

  async getConnection(id: string): Promise<ConnectionRecord | null> {
    const row = this.db.get<ConnectionRow>(`${CONNECTION_SELECT} WHERE id = ?`, [id])
    return row ? connectionFromRow(row) : null
  }

  async listConnections(): Promise<ConnectionRecord[]> {
    return this.db.all<ConnectionRow>(`${CONNECTION_SELECT} ORDER BY created_at`).map(connectionFromRow)
  }

  async deleteConnection(id: string): Promise<void> {
    this.db.run('DELETE FROM connections WHERE id = ?', [id])
  }

  async appendUsage(row: Omit<UsageRow, 'id' | 'createdAt'>): Promise<void> {
    this.db.run(
      'INSERT INTO usage_ledger (id, user_id, conversation_id, connection_id, model_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
      [
        createId(),
        row.userId,
        row.conversationId,
        row.connectionId,
        row.modelId,
        row.inputTokens,
        row.outputTokens,
        row.cacheReadTokens,
        row.cacheWriteTokens,
        new Date().toISOString(),
      ],
    )
  }

  async createAttachment(attachment: {
    userId: string
    mediaType: string
    sizeBytes: number
    sha256: string
  }): Promise<AttachmentRecord> {
    const record: AttachmentRecord = {
      id: createId(),
      userId: attachment.userId,
      mediaType: attachment.mediaType,
      sizeBytes: attachment.sizeBytes,
      sha256: attachment.sha256,
      createdAt: new Date().toISOString(),
    }
    this.db.run('INSERT INTO attachments (id, user_id, media_type, size_bytes, sha256, created_at) VALUES (?, ?, ?, ?, ?, ?)', [
      record.id,
      record.userId,
      record.mediaType,
      record.sizeBytes,
      record.sha256,
      record.createdAt,
    ])
    return record
  }

  async getAttachment(id: string): Promise<AttachmentRecord | null> {
    const row = this.db.get<AttachmentRow>(
      'SELECT id, user_id, media_type, size_bytes, sha256, created_at FROM attachments WHERE id = ?',
      [id],
    )
    return row
      ? {
          id: row.id,
          userId: row.user_id,
          mediaType: row.media_type,
          sizeBytes: row.size_bytes,
          sha256: row.sha256,
          createdAt: row.created_at,
        }
      : null
  }

  async listUsage(userId: string): Promise<UsageRow[]> {
    return this.db
      .all<UsageLedgerRow>(
        'SELECT id, user_id, conversation_id, connection_id, model_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at FROM usage_ledger WHERE user_id = ? ORDER BY created_at',
        [userId],
      )
      .map((row) => ({
        id: row.id,
        userId: row.user_id,
        conversationId: row.conversation_id,
        connectionId: row.connection_id,
        modelId: row.model_id,
        inputTokens: row.input_tokens,
        outputTokens: row.output_tokens,
        cacheReadTokens: row.cache_read_tokens,
        cacheWriteTokens: row.cache_write_tokens,
        createdAt: row.created_at,
      }))
  }

  async createWorkspace(workspace: {
    userId: string
    deviceId: string
    path: string
    name: string
  }): Promise<WorkspaceRecord> {
    const record: WorkspaceRecord = {
      id: createId(),
      userId: workspace.userId,
      deviceId: workspace.deviceId,
      path: workspace.path,
      name: workspace.name,
      createdAt: new Date().toISOString(),
    }
    this.db.run('INSERT INTO workspaces (id, user_id, device_id, path, name, created_at) VALUES (?, ?, ?, ?, ?, ?)', [
      record.id,
      record.userId,
      record.deviceId,
      record.path,
      record.name,
      record.createdAt,
    ])
    return record
  }

  async getWorkspace(id: string): Promise<WorkspaceRecord | null> {
    const row = this.db.get<WorkspaceRow>('SELECT id, user_id, device_id, path, name, created_at FROM workspaces WHERE id = ?', [id])
    return row
      ? {
          id: row.id,
          userId: row.user_id,
          deviceId: row.device_id,
          path: row.path,
          name: row.name,
          createdAt: row.created_at,
        }
      : null
  }

  async listWorkspaces(userId: string): Promise<WorkspaceRecord[]> {
    const rows = this.db.all<WorkspaceRow>(
      'SELECT id, user_id, device_id, path, name, created_at FROM workspaces WHERE user_id = ? ORDER BY created_at',
      [userId],
    )
    return rows.map((row) => ({
      id: row.id,
      userId: row.user_id,
      deviceId: row.device_id,
      path: row.path,
      name: row.name,
      createdAt: row.created_at,
    }))
  }

  async renameWorkspace(id: string, name: string): Promise<void> {
    this.db.run('UPDATE workspaces SET name = ? WHERE id = ?', [name, id])
  }

  async deleteWorkspace(id: string): Promise<void> {
    this.db.run('DELETE FROM workspaces WHERE id = ?', [id])
  }

  async countConversationsInWorkspace(workspaceId: string): Promise<number> {
    return this.db.get<{ n: number }>('SELECT COUNT(*) AS n FROM conversations WHERE workspace_id = ?', [workspaceId])?.n ?? 0
  }

  async listConversationIdsInWorkspace(workspaceId: string): Promise<string[]> {
    return this.db.all<{ id: string }>('SELECT id FROM conversations WHERE workspace_id = ?', [workspaceId]).map((row) => row.id)
  }

  async setConversationWorkspace(conversationId: string, workspaceId: string | null): Promise<void> {
    this.db.run('UPDATE conversations SET workspace_id = ?, updated_at = ? WHERE id = ?', [
      workspaceId,
      new Date().toISOString(),
      conversationId,
    ])
  }

  async switchConversationTarget(
    conversationId: string,
    from: ConversationTargetPointer,
    to: ConversationTargetPointer,
    pending: PendingSwitch,
    grantDeviceId: string | null,
  ): Promise<boolean> {
    return this.db.transaction(() => {
      const now = new Date().toISOString()
      this.db.run(
        'UPDATE conversations SET workspace_id = ?, host_device_id = ?, pending_switch_json = ?, updated_at = ? WHERE id = ? AND workspace_id IS ? AND host_device_id IS ?',
        [to.workspaceId, to.hostDeviceId, JSON.stringify(pending), now, conversationId, from.workspaceId, from.hostDeviceId],
      )
      const won = (this.db.get<{ n: number }>('SELECT changes() AS n')?.n ?? 0) > 0
      if (won && grantDeviceId !== null) this.insertGrant(conversationId, grantDeviceId, now)
      return won
    })
  }

  async clearPendingSwitch(conversationId: string): Promise<void> {
    this.db.run('UPDATE conversations SET pending_switch_json = NULL WHERE id = ?', [conversationId])
  }

  async grantHost(conversationId: string, deviceId: string): Promise<void> {
    this.insertGrant(conversationId, deviceId, new Date().toISOString())
  }

  async revokeHost(conversationId: string, deviceId: string): Promise<void> {
    this.db.run('DELETE FROM conversation_host_grants WHERE conversation_id = ? AND device_id = ?', [conversationId, deviceId])
  }

  async listHostGrants(conversationId: string): Promise<HostGrantRecord[]> {
    return this.db
      .all<HostGrantRow>(
        'SELECT conversation_id, device_id, granted_at FROM conversation_host_grants WHERE conversation_id = ? ORDER BY granted_at',
        [conversationId],
      )
      .map((row) => ({ conversationId: row.conversation_id, deviceId: row.device_id, grantedAt: row.granted_at }))
  }

  async isHostGranted(conversationId: string, deviceId: string): Promise<boolean> {
    return (
      this.db.get<{ n: number }>('SELECT COUNT(*) AS n FROM conversation_host_grants WHERE conversation_id = ? AND device_id = ?', [
        conversationId,
        deviceId,
      ])?.n ?? 0
    ) > 0
  }

  private insertGrant(conversationId: string, deviceId: string, grantedAt: string): void {
    this.db.run(
      'INSERT INTO conversation_host_grants (conversation_id, device_id, granted_at) VALUES (?, ?, ?) ON CONFLICT (conversation_id, device_id) DO NOTHING',
      [conversationId, deviceId, grantedAt],
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
      hostDeviceId: null,
      pendingSwitch: null,
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

interface DeviceRow {
  id: string
  user_id: string
  kind: DeviceKind
  name: string
  platform: string
  owner_conversation_id: string | null
  owner_workspace_id: string | null
  claimed_at: string
  last_seen_at: string | null
}

interface HostGrantRow {
  conversation_id: string
  device_id: string
  granted_at: string
}

interface WorkspaceRow {
  id: string
  user_id: string
  device_id: string
  path: string
  name: string
  created_at: string
}

const DEVICE_SELECT = 'SELECT id, user_id, kind, name, platform, owner_conversation_id, owner_workspace_id, claimed_at, last_seen_at FROM devices'

interface ConnectionRow {
  id: string
  owner_user_id: string | null
  type: string
  label: string
  config: string
  created_at: string
}

interface AttachmentRow {
  id: string
  user_id: string
  media_type: string
  size_bytes: number
  sha256: string
  created_at: string
}

interface UsageLedgerRow {
  id: string
  user_id: string
  conversation_id: string
  connection_id: string
  model_id: string
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_write_tokens: number
  created_at: string
}

const CONNECTION_SELECT = 'SELECT id, owner_user_id, type, label, config, created_at FROM connections'

function connectionFromRow(row: ConnectionRow): ConnectionRecord {
  return {
    id: row.id,
    ownerUserId: row.owner_user_id,
    type: row.type,
    label: row.label,
    config: row.config,
    createdAt: row.created_at,
  }
}

function deviceFromRow(row: DeviceRow): DeviceRecord {
  return {
    id: row.id,
    userId: row.user_id,
    kind: row.kind,
    name: row.name,
    platform: row.platform,
    ownerConversationId: row.owner_conversation_id,
    ownerWorkspaceId: row.owner_workspace_id,
    claimedAt: row.claimed_at,
    lastSeenAt: row.last_seen_at,
  }
}

function fromRow(row: ConversationRow): ConversationRecord {
  return {
    id: row.id,
    userId: row.user_id,
    title: row.title,
    archived: row.archived !== 0,
    workspaceId: row.workspace_id,
    hostDeviceId: row.host_device_id,
    pendingSwitch: row.pending_switch_json ? (JSON.parse(row.pending_switch_json) as PendingSwitch) : null,
    connectionId: row.connection_id,
    modelId: row.model_id,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  }
}

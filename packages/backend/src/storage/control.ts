import { createId } from '@demicodes/utils'
import type { Role, User } from '../auth/identity'
import type { SqlDatabase } from './database'

/**
 * The control-plane service contract: every read/write of `control.sqlite`
 * goes through these domain methods. One call is one atomic operation —
 * transactions never leak out of the implementation, so the same interface
 * is realizable in-process (here) and as the `demi-controld` RPC client at
 * N>1 without changing a caller.
 */
export interface ControlService {
  /** The instance's first account: inserted only while `users` is empty, so two concurrent setups yield one master. */
  createMaster(user: { username: string; passwordHash: string }): Promise<User | null>
  /** Null when the username is taken. */
  createUser(user: { username: string; passwordHash: string; role: Role }): Promise<User | null>
  getUser(id: string): Promise<User | null>
  /** The login lookup: the row with its hash. */
  findUserByUsername(username: string): Promise<(User & { passwordHash: string }) | null>
  listUsers(): Promise<User[]>
  countUsers(): Promise<number>
  setUserPassword(id: string, passwordHash: string): Promise<void>
  createWebSession(session: { tokenHash: string; userId: string; expiresAt: string }): Promise<void>
  getWebSession(tokenHash: string): Promise<{ userId: string; expiresAt: string } | null>
  extendWebSession(tokenHash: string, expiresAt: string): Promise<void>
  deleteWebSession(tokenHash: string): Promise<void>
  /** Drops every session whose expiry is at or before `before` (an ISO instant). */
  deleteExpiredWebSessions(before: string): Promise<void>
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
  /** Workspaces pointing at the device: a revoke is refused while any exist. */
  countWorkspacesOnDevice(deviceId: string): Promise<number>
  /** Drops the device with its grants — a grant is a permission, gone with the machine. */
  deleteDevice(id: string): Promise<void>
  touchDeviceSeen(id: string): Promise<void>
  /** `config` is opaque here — the vault encrypts/decrypts; storage never sees plaintext. */
  createProvider(provider: {
    id?: string
    ownerUserId: string | null
    providerType: string
    credentialKind: ProviderCredentialKind
    label: string
    config: string
  }): Promise<ProviderRecord>
  getProvider(id: string): Promise<ProviderRecord | null>
  /** The providers of one scope — the instance's (owner null) or a user's — or every row for the startup check. */
  listProviders(scope: ProviderScope): Promise<ProviderRecord[]>
  /** Rewrites label and/or config (already encrypted); null when the row is gone. */
  updateProvider(id: string, patch: { label?: string; config?: string }): Promise<ProviderRecord | null>
  deleteProvider(id: string): Promise<void>
  appendUsage(row: Omit<UsageRow, 'id' | 'createdAt'>): Promise<void>
  listUsage(userId: string): Promise<UsageRow[]>
  /** The whole ledger — the shared-mode admin view. */
  listAllUsage(): Promise<UsageRow[]>
  /** Metadata only — the bytes live in the blob store under `sha256`. */
  createAttachment(attachment: { userId: string; mediaType: string; sizeBytes: number; sha256: string }): Promise<AttachmentRecord>
  getAttachment(id: string): Promise<AttachmentRecord | null>
  /** `id` pre-chosen by the caller when something must exist under it before the row does (a Cloud workspace's host). */
  createWorkspace(workspace: { id?: string; userId: string; deviceId: string; path: string; name: string }): Promise<WorkspaceRecord>
  getWorkspace(id: string): Promise<WorkspaceRecord | null>
  listWorkspaces(userId: string): Promise<WorkspaceRecord[]>
  renameWorkspace(id: string, name: string): Promise<void>
  deleteWorkspace(id: string): Promise<void>
  countConversationsInWorkspace(workspaceId: string): Promise<number>
  listConversationIdsInWorkspace(workspaceId: string): Promise<string[]>
  setConversationWorkspace(conversationId: string, workspaceId: string | null): Promise<void>
  /**
   * The target-switch write (`sessions-and-targets.md` § Switching): moves
   * the target pointers, records the switch for the next turn's
   * announcement, attaches the departed device with the directory it was
   * left at, and detaches the device the target moves to — a host is main
   * or attached, never both — in one compare-and-set. Returns false (and
   * writes nothing) when the pointers no longer equal `from`, so concurrent
   * switches have exactly one winner.
   */
  switchConversationTarget(
    conversationId: string,
    from: ConversationTargetPointer,
    to: ConversationTargetPointer,
    pending: PendingSwitch,
    ends: { departed: { deviceId: string; cwd: string | null } | null; arrivingDeviceId: string | null },
  ): Promise<boolean>
  /**
   * The session upgrade's write (`sessions-and-targets.md` § Hostless
   * execution): a hostless conversation bound to the machine provisioned for
   * it, silently — no pending switch. False when it is no longer hostless.
   */
  bindConversationHost(conversationId: string, deviceId: string): Promise<boolean>
  /** The announcements were injected; nothing is pending until the next switch or change to the attached hosts. */
  clearPendingAnnouncements(conversationId: string): Promise<void>
  /**
   * The attached hosts (`sessions-and-targets.md` § Attached hosts). `attachHost`
   * is idempotent: an attached device keeps its row; a new one is named from
   * `name`, suffixed while the name is taken in the conversation, and starts
   * where `cwd` says (`null`: its home). `announce` marks the change for the
   * next turn's context block.
   */
  attachHost(conversationId: string, deviceId: string, name: string, cwd: string | null, announce: boolean): Promise<AttachedHostRecord>
  detachHost(conversationId: string, deviceId: string): Promise<boolean>
  listAttachedHosts(conversationId: string): Promise<AttachedHostRecord[]>
  getAttachedHost(conversationId: string, deviceId: string): Promise<AttachedHostRecord | null>
  renameAttachedHost(conversationId: string, deviceId: string, name: string): Promise<'renamed' | 'name_taken' | 'not_attached'>
  /** Where work on the attached host last stood: written back from the job's exit. */
  setAttachedHostCwd(conversationId: string, deviceId: string, cwd: string): Promise<void>
  createConversation(userId: string, options?: { title?: string }): Promise<ConversationRecord>
  getConversation(id: string): Promise<ConversationRecord | null>
  listConversations(userId: string, options?: { archived?: boolean }): Promise<ConversationRecord[]>
  renameConversation(id: string, title: string): Promise<void>
  setConversationArchived(id: string, archived: boolean): Promise<void>
  setConversationModel(id: string, providerId: string | null, modelId: string | null): Promise<void>
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

export interface AttachedHostRecord {
  conversationId: string
  deviceId: string
  /** What the model and the user call the host; unique within the conversation. */
  name: string
  /** The directory the last `demi host shell --host` there ended in; `null` until one ran — its home. */
  cwd: string | null
  attachedAt: string
}

export interface WorkspaceRecord {
  id: string
  userId: string
  deviceId: string
  path: string
  name: string
  createdAt: string
}

export interface ProviderRecord {
  id: string
  ownerUserId: string | null
  providerType: string
  credentialKind: ProviderCredentialKind
  label: string
  /** Encrypted at rest (vault crypto); opaque to storage. */
  config: string
  createdAt: string
}

export type ProviderCredentialKind = 'api_key' | 'subscription'

export class ProviderExistsError extends Error {
  readonly code = 'provider_exists'

  constructor(providerType: string) {
    super(`This scope already has a ${providerType} subscription`)
    this.name = 'ProviderExistsError'
  }
}

export interface AttachmentRecord {
  id: string
  userId: string
  mediaType: string
  sizeBytes: number
  sha256: string
  createdAt: string
}

/** A provider listing's scope: one owner (null = the instance's, shared mode) or every row. */
export type ProviderScope = { ownerUserId: string | null } | 'all'

export interface UsageRow {
  id: string
  userId: string
  conversationId: string
  providerId: string
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
  /** The attached hosts changed since the model last heard of them: announced at the next turn boundary. */
  hostsChanged: boolean
  providerId: string | null
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
  hosts_changed: number
  provider_id: string | null
  model_id: string | null
  created_at: string
  updated_at: string
}

const SELECT =
  'SELECT id, user_id, title, archived, workspace_id, host_device_id, pending_switch_json, hosts_changed, provider_id, model_id, created_at, updated_at FROM conversations'

interface UserRow {
  id: string
  username: string
  role: Role
  created_at: string
}

const USER_SELECT = 'SELECT id, username, role, created_at FROM users'
const USER_INSERT = 'INSERT INTO users (id, username, password_hash, role, created_at) VALUES (?, ?, ?, ?, ?)'

function userFromRow(row: UserRow): User {
  return { id: row.id, username: row.username, role: row.role, createdAt: row.created_at }
}

/** In-process `ControlService` over the control database. */
export class LocalControlService implements ControlService {
  constructor(private readonly db: SqlDatabase) {}

  async createMaster(user: { username: string; passwordHash: string }): Promise<User | null> {
    const record: User = { id: createId(), username: user.username, role: 'master', createdAt: new Date().toISOString() }
    return this.db.transaction(() => {
      if (this.db.get<{ n: number }>('SELECT COUNT(*) AS n FROM users')?.n) return null
      this.db.run(USER_INSERT, [record.id, record.username, user.passwordHash, record.role, record.createdAt])
      return record
    })
  }

  async createUser(user: { username: string; passwordHash: string; role: Role }): Promise<User | null> {
    const record: User = { id: createId(), username: user.username, role: user.role, createdAt: new Date().toISOString() }
    return this.db.transaction(() => {
      if (this.db.get(`${USER_SELECT} WHERE username = ?`, [user.username])) return null
      this.db.run(USER_INSERT, [record.id, record.username, user.passwordHash, record.role, record.createdAt])
      return record
    })
  }

  async getUser(id: string): Promise<User | null> {
    const row = this.db.get<UserRow>(`${USER_SELECT} WHERE id = ?`, [id])
    return row ? userFromRow(row) : null
  }

  async findUserByUsername(username: string): Promise<(User & { passwordHash: string }) | null> {
    const row = this.db.get<UserRow & { password_hash: string }>('SELECT id, username, role, created_at, password_hash FROM users WHERE username = ?', [username])
    return row ? { ...userFromRow(row), passwordHash: row.password_hash } : null
  }

  async listUsers(): Promise<User[]> {
    return this.db.all<UserRow>(`${USER_SELECT} ORDER BY created_at`).map(userFromRow)
  }

  async countUsers(): Promise<number> {
    return this.db.get<{ n: number }>('SELECT COUNT(*) AS n FROM users')?.n ?? 0
  }

  async setUserPassword(id: string, passwordHash: string): Promise<void> {
    this.db.run('UPDATE users SET password_hash = ? WHERE id = ?', [passwordHash, id])
  }

  async createWebSession(session: { tokenHash: string; userId: string; expiresAt: string }): Promise<void> {
    this.db.run('INSERT INTO web_sessions (token_hash, user_id, expires_at) VALUES (?, ?, ?)', [session.tokenHash, session.userId, session.expiresAt])
  }

  async getWebSession(tokenHash: string): Promise<{ userId: string; expiresAt: string } | null> {
    const row = this.db.get<{ user_id: string; expires_at: string }>('SELECT user_id, expires_at FROM web_sessions WHERE token_hash = ?', [tokenHash])
    return row ? { userId: row.user_id, expiresAt: row.expires_at } : null
  }

  async extendWebSession(tokenHash: string, expiresAt: string): Promise<void> {
    this.db.run('UPDATE web_sessions SET expires_at = ? WHERE token_hash = ?', [expiresAt, tokenHash])
  }

  async deleteWebSession(tokenHash: string): Promise<void> {
    this.db.run('DELETE FROM web_sessions WHERE token_hash = ?', [tokenHash])
  }

  async deleteExpiredWebSessions(before: string): Promise<void> {
    this.db.run('DELETE FROM web_sessions WHERE expires_at <= ?', [before])
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

  async countWorkspacesOnDevice(deviceId: string): Promise<number> {
    return this.db.get<{ n: number }>('SELECT COUNT(*) AS n FROM workspaces WHERE device_id = ?', [deviceId])?.n ?? 0
  }

  async deleteDevice(id: string): Promise<void> {
    this.db.transaction(() => {
      this.db.run('DELETE FROM conversation_hosts WHERE device_id = ?', [id])
      this.db.run('DELETE FROM devices WHERE id = ?', [id])
    })
  }

  async touchDeviceSeen(id: string): Promise<void> {
    this.db.run('UPDATE devices SET last_seen_at = ? WHERE id = ?', [new Date().toISOString(), id])
  }

  async createProvider(provider: {
    id?: string
    ownerUserId: string | null
    providerType: string
    credentialKind: ProviderCredentialKind
    label: string
    config: string
  }): Promise<ProviderRecord> {
    const record: ProviderRecord = {
      id: provider.id ?? createId(),
      ownerUserId: provider.ownerUserId,
      providerType: provider.providerType,
      credentialKind: provider.credentialKind,
      label: provider.label,
      config: provider.config,
      createdAt: new Date().toISOString(),
    }
    const inserted = this.db.get<{ id: string }>("INSERT INTO providers (id, owner_user_id, provider_type, credential_kind, label, config, created_at) VALUES (?, ?, ?, ?, ?, ?, ?) ON CONFLICT(COALESCE(owner_user_id, ''), provider_type) WHERE credential_kind = 'subscription' DO NOTHING RETURNING id", [
      record.id,
      record.ownerUserId,
      record.providerType,
      record.credentialKind,
      record.label,
      record.config,
      record.createdAt,
    ])
    if (!inserted) throw new ProviderExistsError(provider.providerType)
    return record
  }

  async getProvider(id: string): Promise<ProviderRecord | null> {
    const row = this.db.get<ProviderRow>(`${PROVIDER_SELECT} WHERE id = ?`, [id])
    return row ? providerFromRow(row) : null
  }

  async listProviders(scope: ProviderScope): Promise<ProviderRecord[]> {
    const rows =
      scope === 'all'
        ? this.db.all<ProviderRow>(`${PROVIDER_SELECT} ORDER BY created_at`)
        : scope.ownerUserId === null
          ? this.db.all<ProviderRow>(`${PROVIDER_SELECT} WHERE owner_user_id IS NULL ORDER BY created_at`)
          : this.db.all<ProviderRow>(`${PROVIDER_SELECT} WHERE owner_user_id = ? ORDER BY created_at`, [scope.ownerUserId])
    return rows.map(providerFromRow)
  }

  async updateProvider(id: string, patch: { label?: string; config?: string }): Promise<ProviderRecord | null> {
    if (patch.label !== undefined) this.db.run('UPDATE providers SET label = ? WHERE id = ?', [patch.label, id])
    if (patch.config !== undefined) this.db.run('UPDATE providers SET config = ? WHERE id = ?', [patch.config, id])
    return this.getProvider(id)
  }

  async deleteProvider(id: string): Promise<void> {
    this.db.run('DELETE FROM providers WHERE id = ?', [id])
  }

  async appendUsage(row: Omit<UsageRow, 'id' | 'createdAt'>): Promise<void> {
    this.db.run(
      'INSERT INTO usage_ledger (id, user_id, conversation_id, provider_id, model_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)',
      [
        createId(),
        row.userId,
        row.conversationId,
        row.providerId,
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
    return this.db.all<UsageLedgerRow>(`${USAGE_SELECT} WHERE user_id = ? ORDER BY created_at`, [userId]).map(usageFromRow)
  }

  async listAllUsage(): Promise<UsageRow[]> {
    return this.db.all<UsageLedgerRow>(`${USAGE_SELECT} ORDER BY created_at`).map(usageFromRow)
  }


  async createWorkspace(workspace: {
    id?: string
    userId: string
    deviceId: string
    path: string
    name: string
  }): Promise<WorkspaceRecord> {
    const record: WorkspaceRecord = {
      id: workspace.id ?? createId(),
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
    ends: { departed: { deviceId: string; cwd: string | null } | null; arrivingDeviceId: string | null },
  ): Promise<boolean> {
    return this.db.transaction(() => {
      const now = new Date().toISOString()
      this.db.run(
        'UPDATE conversations SET workspace_id = ?, host_device_id = ?, pending_switch_json = ?, updated_at = ? WHERE id = ? AND workspace_id IS ? AND host_device_id IS ?',
        [to.workspaceId, to.hostDeviceId, JSON.stringify(pending), now, conversationId, from.workspaceId, from.hostDeviceId],
      )
      const won = (this.db.get<{ n: number }>('SELECT changes() AS n')?.n ?? 0) > 0
      if (!won) return false
      if (ends.arrivingDeviceId !== null) this.db.run('DELETE FROM conversation_hosts WHERE conversation_id = ? AND device_id = ?', [conversationId, ends.arrivingDeviceId])
      if (ends.departed !== null && ends.departed.deviceId !== ends.arrivingDeviceId) {
        const name = this.db.get<{ name: string }>('SELECT name FROM devices WHERE id = ?', [ends.departed.deviceId])?.name ?? ends.departed.deviceId
        this.insertAttachedHost(conversationId, ends.departed.deviceId, name, ends.departed.cwd, now)
      }
      return true
    })
  }

  async bindConversationHost(conversationId: string, deviceId: string): Promise<boolean> {
    return this.db.transaction(() => {
      this.db.run('UPDATE conversations SET host_device_id = ?, updated_at = ? WHERE id = ? AND workspace_id IS NULL AND host_device_id IS NULL', [
        deviceId,
        new Date().toISOString(),
        conversationId,
      ])
      return (this.db.get<{ n: number }>('SELECT changes() AS n')?.n ?? 0) > 0
    })
  }

  async clearPendingAnnouncements(conversationId: string): Promise<void> {
    this.db.run('UPDATE conversations SET pending_switch_json = NULL, hosts_changed = 0 WHERE id = ?', [conversationId])
  }

  async attachHost(conversationId: string, deviceId: string, name: string, cwd: string | null, announce: boolean): Promise<AttachedHostRecord> {
    return this.db.transaction(() => {
      const existing = this.attachedHost(conversationId, deviceId)
      if (existing) return existing
      this.insertAttachedHost(conversationId, deviceId, name, cwd, new Date().toISOString())
      if (announce) this.db.run('UPDATE conversations SET hosts_changed = 1 WHERE id = ?', [conversationId])
      return this.attachedHost(conversationId, deviceId)!
    })
  }

  async detachHost(conversationId: string, deviceId: string): Promise<boolean> {
    return this.db.transaction(() => {
      this.db.run('DELETE FROM conversation_hosts WHERE conversation_id = ? AND device_id = ?', [conversationId, deviceId])
      const removed = (this.db.get<{ n: number }>('SELECT changes() AS n')?.n ?? 0) > 0
      if (removed) this.db.run('UPDATE conversations SET hosts_changed = 1 WHERE id = ?', [conversationId])
      return removed
    })
  }

  async listAttachedHosts(conversationId: string): Promise<AttachedHostRecord[]> {
    return this.db
      .all<AttachedHostRow>(`${ATTACHED_HOST_SELECT} WHERE conversation_id = ? ORDER BY attached_at, name`, [conversationId])
      .map(attachedHostFromRow)
  }

  async getAttachedHost(conversationId: string, deviceId: string): Promise<AttachedHostRecord | null> {
    return this.attachedHost(conversationId, deviceId)
  }

  async renameAttachedHost(conversationId: string, deviceId: string, name: string): Promise<'renamed' | 'name_taken' | 'not_attached'> {
    return this.db.transaction(() => {
      if (!this.attachedHost(conversationId, deviceId)) return 'not_attached'
      const holder = this.db.get<{ device_id: string }>('SELECT device_id FROM conversation_hosts WHERE conversation_id = ? AND name = ?', [conversationId, name])
      if (holder && holder.device_id !== deviceId) return 'name_taken'
      this.db.run('UPDATE conversation_hosts SET name = ? WHERE conversation_id = ? AND device_id = ?', [name, conversationId, deviceId])
      this.db.run('UPDATE conversations SET hosts_changed = 1 WHERE id = ?', [conversationId])
      return 'renamed'
    })
  }

  async setAttachedHostCwd(conversationId: string, deviceId: string, cwd: string): Promise<void> {
    this.db.run('UPDATE conversation_hosts SET cwd = ? WHERE conversation_id = ? AND device_id = ?', [cwd, conversationId, deviceId])
  }

  private attachedHost(conversationId: string, deviceId: string): AttachedHostRecord | null {
    const row = this.db.get<AttachedHostRow>(`${ATTACHED_HOST_SELECT} WHERE conversation_id = ? AND device_id = ?`, [conversationId, deviceId])
    return row ? attachedHostFromRow(row) : null
  }

  /** Inserts the row under the first free name: `name`, then `name-2`, `name-3`, … within the conversation. */
  private insertAttachedHost(conversationId: string, deviceId: string, name: string, cwd: string | null, attachedAt: string): void {
    const base = name.trim() || deviceId
    const taken = new Set(this.db.all<{ name: string }>('SELECT name FROM conversation_hosts WHERE conversation_id = ?', [conversationId]).map((row) => row.name))
    let candidate = base
    for (let n = 2; taken.has(candidate); n += 1) candidate = `${base}-${n}`
    this.db.run(
      'INSERT INTO conversation_hosts (conversation_id, device_id, name, cwd, attached_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT (conversation_id, device_id) DO NOTHING',
      [conversationId, deviceId, candidate, cwd, attachedAt],
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
      hostsChanged: false,
      providerId: null,
      modelId: null,
      createdAt: now,
      updatedAt: now,
    }
    this.db.run(
      'INSERT INTO conversations (id, user_id, title, archived, workspace_id, provider_id, model_id, created_at, updated_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)',
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

  async setConversationModel(id: string, providerId: string | null, modelId: string | null): Promise<void> {
    this.db.run('UPDATE conversations SET provider_id = ?, model_id = ?, updated_at = ? WHERE id = ?', [
      providerId,
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

interface AttachedHostRow {
  conversation_id: string
  device_id: string
  name: string
  cwd: string | null
  attached_at: string
}

const ATTACHED_HOST_SELECT = 'SELECT conversation_id, device_id, name, cwd, attached_at FROM conversation_hosts'

function attachedHostFromRow(row: AttachedHostRow): AttachedHostRecord {
  return { conversationId: row.conversation_id, deviceId: row.device_id, name: row.name, cwd: row.cwd, attachedAt: row.attached_at }
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

interface ProviderRow {
  id: string
  owner_user_id: string | null
  provider_type: string
  credential_kind: ProviderCredentialKind
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
  provider_id: string
  model_id: string
  input_tokens: number
  output_tokens: number
  cache_read_tokens: number
  cache_write_tokens: number
  created_at: string
}

const PROVIDER_SELECT = 'SELECT id, owner_user_id, provider_type, credential_kind, label, config, created_at FROM providers'

const USAGE_SELECT =
  'SELECT id, user_id, conversation_id, provider_id, model_id, input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at FROM usage_ledger'

function usageFromRow(row: UsageLedgerRow): UsageRow {
  return {
    id: row.id,
    userId: row.user_id,
    conversationId: row.conversation_id,
    providerId: row.provider_id,
    modelId: row.model_id,
    inputTokens: row.input_tokens,
    outputTokens: row.output_tokens,
    cacheReadTokens: row.cache_read_tokens,
    cacheWriteTokens: row.cache_write_tokens,
    createdAt: row.created_at,
  }
}

function providerFromRow(row: ProviderRow): ProviderRecord {
  return {
    id: row.id,
    ownerUserId: row.owner_user_id,
    providerType: row.provider_type,
    credentialKind: row.credential_kind,
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
    hostsChanged: row.hosts_changed !== 0,
    providerId: row.provider_id,
    modelId: row.model_id,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  }
}

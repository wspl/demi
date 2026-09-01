import type { ConnectionRecord, ControlService } from '../storage/control'
import { decryptJson, encryptJson } from './crypto'

/**
 * A provider connection's credential payload. `provider` names a registered
 * provider type in the LLM module's factory table; `modelIds` is the
 * user-entered model list for compatible endpoints (the one case where model
 * ids are stored — catalogs otherwise come live from the runtimes).
 * Subscription payloads arrive with the device-login flows (M5 step 2).
 */
export interface ApiKeyConnectionConfig {
  kind: 'api_key'
  provider: string
  apiKey: string
  baseUrl?: string
  modelIds?: string[]
}

/**
 * A completed subscription login. The OAuth material itself lives in the
 * provider's own credential pool under the vault directory
 * (`<dataDir>/vault/<connectionId>/`) — the provider's login/refresh
 * machinery manages it; the row only names the provider type.
 */
export interface SubscriptionConnectionConfig {
  kind: 'subscription'
  provider: string
}

export type ConnectionConfig = ApiKeyConnectionConfig | SubscriptionConnectionConfig

export interface Connection {
  id: string
  ownerUserId: string | null
  label: string
  config: ConnectionConfig
  createdAt: string
}

/**
 * The credential vault over the control plane: config plaintext exists only in
 * this process's memory — rows carry AES-256-GCM ciphertext under the
 * instance secret.
 */
export class ConnectionVault {
  constructor(
    private readonly control: ControlService,
    private readonly secret: Uint8Array,
  ) {}

  async create(options: { ownerUserId: string | null; label: string; config: ConnectionConfig }): Promise<Connection> {
    const record = await this.control.createConnection({
      ownerUserId: options.ownerUserId,
      type: options.config.provider,
      label: options.label,
      config: encryptJson(this.secret, options.config),
    })
    return this.decode(record)
  }

  async get(id: string): Promise<Connection | null> {
    const record = await this.control.getConnection(id)
    return record ? this.decode(record) : null
  }

  async list(): Promise<Connection[]> {
    return (await this.control.listConnections()).map((record) => this.decode(record))
  }

  async delete(id: string): Promise<void> {
    await this.control.deleteConnection(id)
  }

  private decode(record: ConnectionRecord): Connection {
    return {
      id: record.id,
      ownerUserId: record.ownerUserId,
      label: record.label,
      config: decryptJson<ConnectionConfig>(this.secret, record.config),
      createdAt: record.createdAt,
    }
  }
}

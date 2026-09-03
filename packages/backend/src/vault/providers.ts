import type { ProviderRecord, ProviderScope, ControlService } from '../storage/control'
import { decryptJson, encryptJson } from './crypto'

/**
 * A provider's credential payload. `provider` names a registered
 * provider type in the LLM module's factory table; `modelIds` is the
 * user-entered model list for compatible endpoints (the one case where model
 * ids are stored — catalogs otherwise come live from the runtimes).
 * Subscription payloads arrive with the device-login flows (M5 step 2).
 */
export interface ApiKeyProviderConfig {
  kind: 'api_key'
  providerType: string
  apiKey: string
  baseUrl?: string
  modelIds?: string[]
}

/**
 * A completed subscription login. The OAuth material itself lives in the
 * provider's own credential pool under the vault directory
 * (`<dataDir>/vault/<providerId>/`) — the provider's login/refresh
 * machinery manages it; the row only names the provider type.
 */
export interface SubscriptionProviderConfig {
  kind: 'subscription'
  providerType: string
}

export type ProviderConfig = ApiKeyProviderConfig | SubscriptionProviderConfig

export interface ProviderEntry {
  id: string
  ownerUserId: string | null
  label: string
  config: ProviderConfig
  createdAt: string
}

/**
 * The credential vault over the control plane: config plaintext exists only in
 * this process's memory — rows carry AES-256-GCM ciphertext under the
 * instance secret.
 */
export class ProviderVault {
  constructor(
    private readonly control: ControlService,
    private readonly secret: Uint8Array,
  ) {}

  async create(options: { ownerUserId: string | null; label: string; config: ProviderConfig }): Promise<ProviderEntry> {
    const record = await this.control.createProvider({
      ownerUserId: options.ownerUserId,
      providerType: options.config.providerType,
      label: options.label,
      config: encryptJson(this.secret, options.config),
    })
    return this.decode(record)
  }

  async get(id: string): Promise<ProviderEntry | null> {
    const record = await this.control.getProvider(id)
    return record ? this.decode(record) : null
  }

  async list(scope: ProviderScope): Promise<ProviderEntry[]> {
    return (await this.control.listProviders(scope)).map((record) => this.decode(record))
  }

  async delete(id: string): Promise<void> {
    await this.control.deleteProvider(id)
  }

  private decode(record: ProviderRecord): ProviderEntry {
    return {
      id: record.id,
      ownerUserId: record.ownerUserId,
      label: record.label,
      config: decryptJson<ProviderConfig>(this.secret, record.config),
      createdAt: record.createdAt,
    }
  }
}

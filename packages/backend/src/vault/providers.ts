import type { ProviderRecord, ProviderScope, ControlService } from '../storage/control'
import { decryptJson, encryptJson } from './crypto'

/**
 * An API-key entry: one runtime family (`providerType`, a registered
 * provider type in the LLM module's factory table) at an endpoint. The model
 * source is `modelIds` when the user typed a list, else the models.dev
 * vendor named by `vendorId` (fetched live, never stored), else the
 * runtime's own catalog. `wireApi` is the openai family's protocol choice.
 */
export interface ApiKeyProviderConfig {
  kind: 'api_key'
  providerType: string
  apiKey: string
  baseUrl?: string
  wireApi?: 'responses' | 'chat-completions'
  vendorId?: string
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

  /** Rewrites an entry's label and/or config; the row keeps its id, owner, type and creation time. */
  async update(id: string, patch: { label?: string; config?: ProviderConfig }): Promise<ProviderEntry | null> {
    const record = await this.control.updateProvider(id, {
      ...(patch.label !== undefined ? { label: patch.label } : {}),
      ...(patch.config ? { config: encryptJson(this.secret, patch.config) } : {}),
    })
    return record ? this.decode(record) : null
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

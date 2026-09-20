import type {
  ProviderRecord,
  ProviderScope,
  ControlService
} from '../storage/control'
import type { InstanceMode } from '../auth/identity'
import { decryptJson, encryptJson } from './crypto'

import { z } from 'zod'
import { configuredModelsSchema } from '../llm/model-config'

const apiKeyConfigSchema = z.strictObject({
  kind: z.literal('api_key'),
  providerType: z.string().min(1),
  apiKey: z.string().min(1),
  baseUrl: z.url().optional(),
  wireApi: z.enum(['responses', 'chat-completions']).optional(),
  vendorId: z.string().min(1).optional(),
  models: configuredModelsSchema.optional(),
})
const subscriptionConfigSchema = z.strictObject({
  kind: z.literal('subscription'),
  providerType: z.string().min(1)
})
const providerConfigSchema = z.discriminatedUnion('kind', [
  apiKeyConfigSchema,
  subscriptionConfigSchema
])
export type ApiKeyProviderConfig = z.infer<typeof apiKeyConfigSchema>
export type SubscriptionProviderConfig = z.infer<typeof subscriptionConfigSchema>
export type ProviderConfig = z.infer<typeof providerConfigSchema>

export interface ProviderEntry {
  id: string
  ownerUserId: string
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
  private masterId: string | null = null

  constructor(
    private readonly control: ControlService,
    private readonly secret: Uint8Array,
    private readonly mode: InstanceMode,
  ) {}

  /**
   * Whose entries a user infers with (`product.md` § Instance mode): the
   * master's on a shared instance, their own on an isolated one.
   */
  async ownerFor(userId: string): Promise<string> {
    if (this.mode === 'isolated')
      return userId
    // The master is created once by setup and never changes.
    this.masterId ??= (await this.control.getMaster())?.id ?? null
    if (!this.masterId)
      throw new Error('This instance has no master account yet')
    return this.masterId
  }

  /** The provider a user may name, or null. */
  async visible(userId: string, providerId: string): Promise<ProviderEntry | null> {
    const [provider, ownerUserId] = await Promise.all([
      this.get(providerId),
      this.ownerFor(userId)
    ])
    return provider?.ownerUserId === ownerUserId ? provider : null
  }

  async create(options: {
    id?: string;
    ownerUserId: string;
    label: string;
    config: ProviderConfig
  }): Promise<ProviderEntry> {
    const record = await this.control.createProvider({
      ...(options.id ? { id: options.id } : {}),
      ownerUserId: options.ownerUserId,
      providerType: options.config.providerType,
      credentialKind: options.config.kind,
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
    return (await this.control.listProviders(scope)).map(
      (record) => this.decode(record)
    )
  }

  /**
   * Rewrites an entry's label and/or config; the row keeps its id, owner, type
   * and creation time.
   */
  async update(
    id: string,
    patch: {
      label?: string;
      config?: ProviderConfig
    }
  ): Promise<ProviderEntry | null> {
    const record = await this.control.updateProvider(id, {
      ...(patch.label !== undefined ? { label: patch.label } : {}),
      ...(patch.config
        ? { config: encryptJson(this.secret, patch.config) }
        : {}),
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
      config: providerConfigSchema.parse(
        decryptJson(this.secret, record.config)
      ),
      createdAt: record.createdAt,
    }
  }
}

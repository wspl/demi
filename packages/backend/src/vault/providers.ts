import type {
  ProviderRecord,
  ProviderScope,
  ControlService
} from '../storage/control'
import { decryptJson, encryptJson } from './crypto'

import { z } from 'zod'
import { configuredModelsSchema } from '@demicodes/product-contracts'

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

  async create(options: {
    id?: string;
    ownerUserId: string | null;
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

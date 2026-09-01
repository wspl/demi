import { providerRuntime, withProviderId, type Provider, type ProviderModelList } from '@demicodes/provider'
import { createId, errorMessage } from '@demicodes/utils'
import { createAnthropicApiProvider } from '@demicodes/provider-anthropic-api'
import { createGoogleProvider } from '@demicodes/provider-google'
import { createOpenAIApiProvider } from '@demicodes/provider-openai-api'
import type { ControlService } from '../storage/control'
import type { ApiKeyConnectionConfig, Connection, ConnectionVault } from '../vault/connections'

/** Builds a base provider for one connection; the connection id is the provider id. */
export type ProviderTypeFactory = (options: {
  connectionId: string
  label: string
  config: ApiKeyConnectionConfig
}) => Provider

export function builtinProviderTypes(): Record<string, ProviderTypeFactory> {
  const common = ({ connectionId, label }: { connectionId: string; label: string }) => ({
    id: connectionId,
    displayName: label,
  })
  return {
    anthropic: (options) =>
      createAnthropicApiProvider({
        ...common(options),
        apiKey: () => options.config.apiKey,
        ...(options.config.baseUrl ? { baseUrl: options.config.baseUrl } : {}),
      }),
    openai: (options) =>
      createOpenAIApiProvider({
        ...common(options),
        apiKey: () => options.config.apiKey,
        ...(options.config.baseUrl ? { baseUrl: options.config.baseUrl } : {}),
      }),
    google: (options) =>
      createGoogleProvider({
        ...common(options),
        apiKey: () => options.config.apiKey,
        ...(options.config.baseUrl ? { baseUrl: options.config.baseUrl } : {}),
      }),
  }
}

export interface CatalogConnection {
  connectionId: string
  displayName: string
  requiresProcessCapableHost: boolean
  models: ProviderModelList['models']
}

/**
 * The LLM module's provider assembly: one base provider runtime configuration
 * per connection (`connectionId` = `providerId`), built from vault
 * credentials through the registered type factories. Connections are
 * immutable rows (create/delete only), so the cache invalidates on delete.
 */
export class ProviderAssembly {
  private readonly cache = new Map<string, Provider>()

  constructor(
    private readonly vault: ConnectionVault,
    private readonly types: Record<string, ProviderTypeFactory>,
  ) {}

  /** The base (unmetered) provider for a connection, or null when unknown. */
  async providerFor(connectionId: string): Promise<{ connection: Connection; provider: Provider } | null> {
    const connection = await this.vault.get(connectionId)
    if (!connection) return null
    const cached = this.cache.get(connectionId)
    if (cached) return { connection, provider: cached }
    const factory = this.types[connection.config.provider]
    if (!factory) throw new Error(`Unknown provider type "${connection.config.provider}"`)
    const provider = factory({ connectionId, label: connection.label, config: connection.config })
    this.cache.set(connectionId, provider)
    return { connection, provider }
  }

  invalidate(connectionId: string): void {
    this.cache.delete(connectionId)
  }

  /** Whether a connection type is registered — the create endpoint's validation. */
  hasType(type: string): boolean {
    return type in this.types
  }

  /**
   * The connection **Test** button: one cheap real request against the
   * endpoint/key — first streamed event wins, errors report the provider's
   * own message.
   */
  async testConnection(connectionId: string): Promise<{ ok: boolean; message?: string }> {
    const resolved = await this.providerFor(connectionId)
    if (!resolved) return { ok: false, message: 'Unknown connection' }
    const { connection, provider } = resolved
    const modelId =
      connection.config.modelIds?.[0] ??
      (provider.listModels ? (await provider.listModels()).models[0]?.id : undefined)
    if (!modelId) return { ok: false, message: 'No model available to test with' }

    const cancel = new AbortController()
    try {
      const runtime = await providerRuntime(provider, {
        providerId: connection.id,
        model: {
          providerId: connection.id,
          model: { id: modelId, name: modelId, contextWindow: 128_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
          thinking: null,
        },
      })
      const run = runtime.run({
        sessionId: 'connection-test',
        turnId: createId(),
        requestId: createId(),
        modelId,
        systemPrompt: 'Reply with the word ok.',
        cwd: '/',
        items: [{ type: 'user_message', content: [{ type: 'text', text: 'ping' }] }],
        tools: [],
        thinking: null,
        cancel: cancel.signal,
      })
      for await (const event of run) {
        if (event.type === 'error') return { ok: false, message: event.message }
        cancel.abort()
        break
      }
      await runtime.dispose?.()
      return { ok: true }
    } catch (error) {
      return { ok: false, message: errorMessage(error) }
    }
  }

  /**
   * The aggregated model catalog, grouped by connection. Model ids are never
   * stored — lists come live from each provider runtime — except
   * compatible-endpoint connections, whose user-entered `modelIds` become
   * minimal catalog entries.
   */
  async catalog(): Promise<CatalogConnection[]> {
    const connections = await this.vault.list()
    return Promise.all(
      connections.map(async (connection) => {
        const resolved = await this.providerFor(connection.id)
        const provider = resolved?.provider
        const models = connection.config.modelIds
          ? connection.config.modelIds.map((modelId) => userEnteredModel(connection.id, modelId))
          : provider?.listModels
            ? withProviderId(await provider.listModels(), connection.id).models
            : []
        return {
          connectionId: connection.id,
          displayName: connection.label,
          requiresProcessCapableHost: provider?.requiresProcessCapableHost ?? false,
          models,
        }
      }),
    )
  }
}

/**
 * A compatible endpoint's user-entered model id as a catalog entry: the
 * capabilities are unknown to us, so everything capability-shaped is null
 * (unknown) and the context window uses a conservative mainstream default.
 */
function userEnteredModel(connectionId: string, modelId: string) {
  return {
    providerId: connectionId,
    id: modelId,
    displayName: modelId,
    contextWindow: 128_000,
    outputLimit: null,
    supportsTools: null,
    supportsAttachments: null,
    supportsReasoning: null,
    supportedThinkingEfforts: null,
    defaultThinkingEffort: null,
    sourceFetchedAt: new Date().toISOString(),
    stale: false,
  }
}

/** The ledger row appender used by the metering wrap — one row per provider request. */
export function usageAppender(
  control: ControlService,
  context: { userId: string; conversationId: string; connectionId: string },
) {
  return (usage: { inputTokens: number; outputTokens: number; cacheReadTokens: number; cacheWriteTokens: number }, request: { modelId: string }) => {
    void control
      .appendUsage({
        userId: context.userId,
        conversationId: context.conversationId,
        connectionId: context.connectionId,
        modelId: request.modelId,
        inputTokens: usage.inputTokens,
        outputTokens: usage.outputTokens,
        cacheReadTokens: usage.cacheReadTokens,
        cacheWriteTokens: usage.cacheWriteTokens,
      })
      .catch(() => {})
  }
}

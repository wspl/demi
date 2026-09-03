import { rm } from 'node:fs/promises'
import { join } from 'node:path'
import { providerRuntime, withProviderId, type Provider, type ProviderModelList } from '@demicodes/provider'
import { createId, errorMessage } from '@demicodes/utils'
import { createAnthropicApiProvider } from '@demicodes/provider-anthropic-api'
import { createClaudeCodeProvider, type ClaudeSpawn } from '@demicodes/provider-claude-code'
import { createCodexProvider } from '@demicodes/provider-codex'
import { createGoogleProvider } from '@demicodes/provider-google'
import { createGrokBuildProvider } from '@demicodes/provider-grok-build'
import { createOpenAIApiProvider } from '@demicodes/provider-openai-api'
import type { ControlService } from '../storage/control'
import type { ApiKeyConnectionConfig, Connection, ConnectionConfig, ConnectionVault } from '../vault/connections'

/**
 * Builds a base provider for one connection; the connection id is the
 * provider id. `vaultDir` is the connection's private credential-pool root —
 * subscription providers keep their OAuth material there.
 */
export type ProviderTypeFactory = (options: {
  connectionId: string
  label: string
  config: ConnectionConfig
  vaultDir: string
  /** Present when the provider needs the session's execution target (CLI transports). */
  session?: SessionProviderContext
}) => Provider

/**
 * Session context for providers whose transport runs on the session's
 * execution target: the target's spawn. Credentials are the provider's own
 * business — its runtime resolves and injects them at spawn time.
 */
export interface SessionProviderContext {
  spawn: ClaudeSpawn
}

function apiKey(config: ConnectionConfig): ApiKeyConnectionConfig {
  if (config.kind !== 'api_key') throw new Error(`Provider type "${config.provider}" expects an API key connection`)
  return config
}

export function builtinProviderTypes(): Record<string, ProviderTypeFactory> {
  const common = ({ connectionId, label }: { connectionId: string; label: string }) => ({
    id: connectionId,
    displayName: label,
  })
  const keyed =
    (create: (options: { id: string; displayName: string; apiKey: () => string; baseUrl?: string }) => Provider): ProviderTypeFactory =>
    (options) => {
      const config = apiKey(options.config)
      return create({
        ...common(options),
        apiKey: () => config.apiKey,
        ...(config.baseUrl ? { baseUrl: config.baseUrl } : {}),
      })
    }
  return {
    anthropic: keyed(createAnthropicApiProvider),
    openai: keyed(createOpenAIApiProvider),
    google: keyed(createGoogleProvider),
    'claude-code': (options) =>
      createClaudeCodeProvider({
        ...common(options),
        stateDir: options.vaultDir,
        ...(options.session ? { spawn: options.session.spawn } : {}),
      }),
    codex: (options) => createCodexProvider({ ...common(options), stateDir: options.vaultDir }),
    'grok-build': (options) => createGrokBuildProvider({ ...common(options), stateDir: options.vaultDir }),
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
    /** Per-connection credential-pool root: `<vaultRoot>/<connectionId>/`. */
    private readonly vaultRoot: string,
  ) {}

  vaultDir(connectionId: string): string {
    return join(this.vaultRoot, connectionId)
  }

  /** Builds a provider through a registered type factory without a connection row (login flows). */
  buildDetached(type: string, options: { id: string; label: string; vaultDir: string }): Provider {
    const factory = this.types[type]
    if (!factory) throw new Error(`Unknown provider type "${type}"`)
    return factory({
      connectionId: options.id,
      label: options.label,
      config: { kind: 'subscription', provider: type },
      vaultDir: options.vaultDir,
    })
  }

  /** Removes a deleted connection's credential-pool directory. */
  async deleteConnectionState(connectionId: string): Promise<void> {
    this.invalidate(connectionId)
    await rm(this.vaultDir(connectionId), { recursive: true, force: true })
  }

  /**
   * The base (unmetered) provider for a connection, or null when unknown.
   * With `session` the provider is built fresh and uncached — session-scoped
   * instances carry the target's spawn and a session passthrough token.
   */
  async providerFor(
    connectionId: string,
    session?: SessionProviderContext,
  ): Promise<{ connection: Connection; provider: Provider } | null> {
    const connection = await this.vault.get(connectionId)
    if (!connection) return null
    if (!session) {
      const cached = this.cache.get(connectionId)
      if (cached) return { connection, provider: cached }
    }
    const factory = this.types[connection.config.provider]
    if (!factory) throw new Error(`Unknown provider type "${connection.config.provider}"`)
    const provider = factory({
      connectionId,
      label: connection.label,
      config: connection.config,
      vaultDir: this.vaultDir(connectionId),
      ...(session ? { session } : {}),
    })
    if (!session) this.cache.set(connectionId, provider)
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
      (connection.config.kind === 'api_key' ? connection.config.modelIds?.[0] : undefined) ??
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
  async catalog(ownerUserId: string | null): Promise<CatalogConnection[]> {
    const connections = await this.vault.list({ ownerUserId })
    return Promise.all(
      connections.map(async (connection) => {
        const resolved = await this.providerFor(connection.id)
        const provider = resolved?.provider
        const modelIds = connection.config.kind === 'api_key' ? connection.config.modelIds : undefined
        const models = modelIds
          ? modelIds.map((modelId) => userEnteredModel(connection.id, modelId))
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

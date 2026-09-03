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
import type { ApiKeyProviderConfig, ProviderEntry, ProviderConfig, ProviderVault } from '../vault/providers'

/**
 * Builds a base provider for one provider; the provider id is the
 * provider id. `vaultDir` is the provider's private credential-pool root —
 * subscription providers keep their OAuth material there.
 */
export type ProviderTypeFactory = (options: {
  providerId: string
  label: string
  config: ProviderConfig
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

function apiKey(config: ProviderConfig): ApiKeyProviderConfig {
  if (config.kind !== 'api_key') throw new Error(`Provider type "${config.providerType}" expects an API key`)
  return config
}

export function builtinProviderTypes(): Record<string, ProviderTypeFactory> {
  const common = ({ providerId, label }: { providerId: string; label: string }) => ({
    id: providerId,
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

export interface CatalogProvider {
  providerId: string
  displayName: string
  requiresProcessCapableHost: boolean
  models: ProviderModelList['models']
}

/**
 * The LLM module's provider assembly: one base provider runtime configuration
 * per provider (`providerId` = `providerId`), built from vault
 * credentials through the registered type factories. Providers are
 * immutable rows (create/delete only), so the cache invalidates on delete.
 */
export class ProviderAssembly {
  private readonly cache = new Map<string, Provider>()

  constructor(
    private readonly vault: ProviderVault,
    private readonly types: Record<string, ProviderTypeFactory>,
    /** Per-provider credential-pool root: `<vaultRoot>/<providerId>/`. */
    private readonly vaultRoot: string,
  ) {}

  vaultDir(providerId: string): string {
    return join(this.vaultRoot, providerId)
  }

  /** Builds a provider through a registered type factory without a provider row (login flows). */
  buildDetached(providerType: string, options: { id: string; label: string; vaultDir: string }): Provider {
    const factory = this.types[providerType]
    if (!factory) throw new Error(`Unknown provider type "${providerType}"`)
    return factory({
      providerId: options.id,
      label: options.label,
      config: { kind: 'subscription', providerType },
      vaultDir: options.vaultDir,
    })
  }

  /** Removes a deleted provider's credential-pool directory. */
  async deleteProviderState(providerId: string): Promise<void> {
    this.invalidate(providerId)
    await rm(this.vaultDir(providerId), { recursive: true, force: true })
  }

  /**
   * The base (unmetered) provider instance behind a provider entry, or null when unknown.
   * With `session` the provider is built fresh and uncached — session-scoped
   * instances carry the target's spawn and a session passthrough token.
   */
  async providerFor(
    providerId: string,
    session?: SessionProviderContext,
  ): Promise<{ entry: ProviderEntry; provider: Provider } | null> {
    const entry = await this.vault.get(providerId)
    if (!entry) return null
    if (!session) {
      const cached = this.cache.get(providerId)
      if (cached) return { entry, provider: cached }
    }
    const factory = this.types[entry.config.providerType]
    if (!factory) throw new Error(`Unknown provider type "${entry.config.providerType}"`)
    const provider = factory({
      providerId,
      label: entry.label,
      config: entry.config,
      vaultDir: this.vaultDir(providerId),
      ...(session ? { session } : {}),
    })
    if (!session) this.cache.set(providerId, provider)
    return { entry, provider }
  }

  invalidate(providerId: string): void {
    this.cache.delete(providerId)
  }

  /** Whether a provider type is registered — the create endpoint's validation. */
  hasType(providerType: string): boolean {
    return providerType in this.types
  }

  /**
   * The provider **Test** button: one cheap real request against the
   * endpoint/key — first streamed event wins, errors report the provider's
   * own message.
   */
  async testProvider(providerId: string): Promise<{ ok: boolean; message?: string }> {
    const resolved = await this.providerFor(providerId)
    if (!resolved) return { ok: false, message: 'Unknown provider' }
    const { entry, provider } = resolved
    const modelId =
      (entry.config.kind === 'api_key' ? entry.config.modelIds?.[0] : undefined) ??
      (provider.listModels ? (await provider.listModels()).models[0]?.id : undefined)
    if (!modelId) return { ok: false, message: 'No model available to test with' }

    const cancel = new AbortController()
    try {
      const runtime = await providerRuntime(provider, {
        providerId: entry.id,
        model: {
          providerId: entry.id,
          model: { id: modelId, name: modelId, contextWindow: 128_000, inputLimit: null, thinking: [], acceptedExtensions: [] },
          thinking: null,
        },
      })
      const run = runtime.run({
        sessionId: 'provider-test',
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
   * The aggregated model catalog, grouped by provider. Model ids are never
   * stored — lists come live from each provider runtime — except
   * compatible-endpoint providers, whose user-entered `modelIds` become
   * minimal catalog entries.
   */
  async catalog(ownerUserId: string | null): Promise<CatalogProvider[]> {
    const entries = await this.vault.list({ ownerUserId })
    return Promise.all(
      entries.map(async (entry) => {
        const resolved = await this.providerFor(entry.id)
        const provider = resolved?.provider
        const modelIds = entry.config.kind === 'api_key' ? entry.config.modelIds : undefined
        const models = modelIds
          ? modelIds.map((modelId) => userEnteredModel(entry.id, modelId))
          : provider?.listModels
            ? withProviderId(await provider.listModels(), entry.id).models
            : []
        return {
          providerId: entry.id,
          displayName: entry.label,
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
function userEnteredModel(providerId: string, modelId: string) {
  return {
    providerId: providerId,
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
  context: { userId: string; conversationId: string; providerId: string },
) {
  return (usage: { inputTokens: number; outputTokens: number; cacheReadTokens: number; cacheWriteTokens: number }, request: { modelId: string }) => {
    void control
      .appendUsage({
        userId: context.userId,
        conversationId: context.conversationId,
        providerId: context.providerId,
        modelId: request.modelId,
        inputTokens: usage.inputTokens,
        outputTokens: usage.outputTokens,
        cacheReadTokens: usage.cacheReadTokens,
        cacheWriteTokens: usage.cacheWriteTokens,
      })
      .catch(() => {})
  }
}

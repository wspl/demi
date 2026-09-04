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
import type { VendorCatalog } from './vendors'
import { vendorRequestOptions } from './vendor-requests'

/**
 * A registered runtime family: how it is credentialed — an API key typed
 * in, or a subscription claimed through the family's device login, of
 * which a scope holds at most one entry — and how an entry's provider is
 * built.
 */
export interface ProviderType {
  credential: 'api_key' | 'subscription'
  create: ProviderTypeFactory
}

/**
 * Builds a base provider for one entry; the provider id is the entry id.
 * `vaultDir` is the entry's private credential-pool root — subscription
 * providers keep their OAuth material there.
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

export function builtinProviderTypes(): Record<string, ProviderType> {
  const common = ({ providerId, label }: { providerId: string; label: string }) => ({
    id: providerId,
    displayName: label,
  })
  const keyed = (
    create: (options: { id: string; displayName: string; apiKey: () => string; baseUrl?: string }) => Provider,
  ): ProviderType => ({
    credential: 'api_key',
    create: (options) => {
      const config = apiKey(options.config)
      return create({
        ...common(options),
        apiKey: () => config.apiKey,
        ...(config.baseUrl ? { baseUrl: config.baseUrl } : {}),
      })
    },
  })
  const subscription = (create: ProviderTypeFactory): ProviderType => ({ credential: 'subscription', create })
  return {
    anthropic: keyed(createAnthropicApiProvider),
    openai: {
      credential: 'api_key',
      create: (options) => {
        const config = apiKey(options.config)
        return createOpenAIApiProvider({
          ...common(options),
          apiKey: () => config.apiKey,
          ...(config.baseUrl ? { baseUrl: config.baseUrl } : {}),
          ...(config.wireApi ? { wireApi: config.wireApi } : {}),
          request: vendorRequestOptions(config.vendorId),
        })
      },
    },
    google: keyed(createGoogleProvider),
    'claude-code': subscription((options) =>
      createClaudeCodeProvider({
        ...common(options),
        stateDir: options.vaultDir,
        ...(options.session ? { spawn: options.session.spawn } : {}),
      }),
    ),
    codex: subscription((options) => createCodexProvider({ ...common(options), stateDir: options.vaultDir })),
    'grok-build': subscription((options) => createGrokBuildProvider({ ...common(options), stateDir: options.vaultDir })),
  }
}

export interface CatalogProvider {
  providerId: string
  displayName: string
  requiresProcessCapableHost: boolean
  models: ProviderModelList['models']
}

/**
 * The LLM module's provider assembly: one base provider runtime per entry,
 * built from vault credentials through the registered families. The cache
 * invalidates when an entry is edited or deleted.
 */
export class ProviderAssembly {
  private readonly cache = new Map<string, { entry: ProviderEntry; provider: Provider }>()

  constructor(
    private readonly vault: ProviderVault,
    private readonly types: Record<string, ProviderType>,
    /** Per-entry credential-pool root: `<vaultRoot>/<providerId>/`. */
    private readonly vaultRoot: string,
    private readonly vendors: VendorCatalog,
  ) {}

  /** The registered families of one credential kind. */
  typesOf(credential: ProviderType['credential']): string[] {
    return Object.entries(this.types)
      .filter(([, type]) => type.credential === credential)
      .map(([name]) => name)
  }

  vaultDir(providerId: string): string {
    return join(this.vaultRoot, providerId)
  }

  /** Builds a provider through a registered type factory without a provider row (login flows). */
  buildDetached(providerType: string, options: { id: string; label: string; vaultDir: string }): Provider {
    const type = this.types[providerType]
    if (!type) throw new Error(`Unknown provider type "${providerType}"`)
    return type.create({
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

  /** The base provider instance behind an entry, or null when unknown; edits invalidate its identity. */
  async providerFor(providerId: string): Promise<{ entry: ProviderEntry; provider: Provider } | null> {
    const entry = await this.vault.get(providerId)
    if (!entry) return null
    const cached = this.cache.get(providerId)
    if (cached && cached.entry.label === entry.label && JSON.stringify(cached.entry.config) === JSON.stringify(entry.config)) {
      return { entry, provider: cached.provider }
    }
    const provider = this.build(entry)
    this.cache.set(providerId, { entry, provider })
    return { entry, provider }
  }

  /** Builds an independent session provider from the entry snapshot selected for this request. */
  forSession(entry: ProviderEntry, session: SessionProviderContext): Provider {
    return this.build(entry, session)
  }

  private build(entry: ProviderEntry, session?: SessionProviderContext): Provider {
    const type = this.types[entry.config.providerType]
    if (!type) throw new Error(`Unknown provider type "${entry.config.providerType}"`)
    return type.create({
      providerId: entry.id,
      label: entry.label,
      config: entry.config,
      vaultDir: this.vaultDir(entry.id),
      ...(session ? { session } : {}),
    })
  }

  invalidate(providerId: string): void {
    this.cache.delete(providerId)
  }

  /** A registered family's credential kind, or null for an unknown type — the create endpoint's validation. */
  credentialOf(providerType: string): ProviderType['credential'] | null {
    return this.types[providerType]?.credential ?? null
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
    const modelId = (await this.modelsOf(entry, provider))[0]?.id
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
   * The aggregated model catalog, grouped by entry. Lists come live: the
   * user-entered ids of a custom endpoint, the models.dev vendor an entry
   * names, or the runtime's own catalog.
   */
  async catalog(ownerUserId: string | null): Promise<CatalogProvider[]> {
    const entries = await this.vault.list({ ownerUserId })
    return Promise.all(
      entries.map(async (entry) => {
        const provider = (await this.providerFor(entry.id))?.provider ?? null
        return {
          providerId: entry.id,
          displayName: entry.label,
          requiresProcessCapableHost: provider?.requiresProcessCapableHost ?? false,
          models: await this.modelsOf(entry, provider),
        }
      }),
    )
  }

  private async modelsOf(entry: ProviderEntry, provider: Provider | null): Promise<ProviderModelList['models']> {
    if (entry.config.kind === 'api_key') {
      if (entry.config.modelIds) return entry.config.modelIds.map((modelId: string) => userEnteredModel(entry.id, modelId))
      if (entry.config.vendorId) return (await this.vendors.models(entry.config.vendorId, entry.id))?.models ?? []
    }
    return provider?.listModels ? withProviderId(await provider.listModels(), entry.id).models : []
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

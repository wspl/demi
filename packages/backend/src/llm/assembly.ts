import { createHash } from 'node:crypto'
import { ModelCatalogCache } from './model-catalog-cache'
import {
  modelSelectionFromCatalog,
  providerRuntime,
  type AgentProvider,
  withCatalogProviderId,
  type Provider,
  type ProviderModelList
} from '@demicodes/provider'
import type { ModelSelection } from '@demicodes/core'
import type { ProviderQuotaSnapshots } from '@demicodes/provider'
import type { CredentialPool } from '@demicodes/provider/credentials-pool'
import type { AccountQuotas } from '../vault/credential-pool'
import { createId, errorMessage } from '@demicodes/utils'
import { createAnthropicApiProvider } from '@demicodes/provider-anthropic-api'
import {
  createClaudeCodeProvider,
  type ClaudeSpawn
} from '@demicodes/provider-claude-code'
import { createCodexProvider } from '@demicodes/provider-codex'
import { createGoogleProvider } from '@demicodes/provider-google'
import { createGrokBuildProvider } from '@demicodes/provider-grok-build'
import { createOpenAIApiProvider } from '@demicodes/provider-openai-api'
import type { ControlService } from '../storage/control'
import type {
  ApiKeyProviderConfig,
  ProviderEntry,
  ProviderConfig,
  ProviderVault
} from '../vault/providers'
import type { VendorCatalog } from './vendors'
import {
  applyConfiguredModel,
  configuredCatalogModel,
  runtimeModelOptions
} from './model-config'
import { providerDetails } from './provider-details'
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
 * Builds a base provider for one entry; the provider id is the entry id. A
 * subscription provider reads its accounts from `credentialPool` and stands for
 * the account `credentialId` names, whose usage `quotaSnapshots` keeps.
 */
export type ProviderTypeFactory = (options: {
  providerId: string
  label: string
  config: ProviderConfig
  credentialPool: CredentialPool
  credentialId?: string
  quotaSnapshots?: ProviderQuotaSnapshots
  /**
   * Present when the provider needs the session's execution target (CLI
   * transports).
   */
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
  if (config.kind !== 'api_key')
    throw new Error(`Provider type "${config.providerType}" expects an API key`)
  return config
}

export function builtinProviderTypes(): Record<string, ProviderType> {
  const accountOf = (options: Parameters<ProviderTypeFactory>[0]) => ({
    credentialPool: options.credentialPool,
    ...(options.credentialId ? { credentialId: options.credentialId } : {}),
    ...(options.quotaSnapshots ? { quotaSnapshots: options.quotaSnapshots } : {}),
  })
  const common = ({ providerId, label }: {
    providerId: string;
    label: string
  }) => ({
    id: providerId,
    displayName: label,
  })
  const keyed = (
    create: (options: {
      id: string;
      displayName: string;
      apiKey: () => string;
      baseUrl?: string;
      models?: ReturnType<typeof runtimeModelOptions>[]
    }) => Provider,
  ): ProviderType => ({
    credential: 'api_key',
    create: (options) => {
      const config = apiKey(options.config)
      return create({
        ...common(options),
        apiKey: () => config.apiKey,
        ...(config.baseUrl ? { baseUrl: config.baseUrl } : {}),
        ...(config.models
          ? { models: config.models.map(runtimeModelOptions) }
          : {}),
      })
    },
  })
  const subscription = (create: ProviderTypeFactory): ProviderType => ({
    credential: 'subscription',
    create
  })
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
          ...(config.models
            ? { models: config.models.map(runtimeModelOptions) }
            : {}),
          ...(config.wireApi ? { wireApi: config.wireApi } : {}),
          request: vendorRequestOptions(config.vendorId),
        })
      },
    },
    google: keyed(createGoogleProvider),
    'claude-code': subscription((options) =>
    createClaudeCodeProvider({
      ...common(options),
      ...accountOf(options),
      ...(options.session ? { spawn: options.session.spawn } : {}),
    }),
    ),
    codex: subscription(
      (options) => createCodexProvider(
        { ...common(options), ...accountOf(options) }
      )
    ),
    'grok-build': subscription(
      (options) => createGrokBuildProvider(
        { ...common(options), ...accountOf(options) }
      )
    ),
  }
}

export interface CatalogProvider {
  providerId: string
  displayName: string
  requiresProcessCapableHost: boolean
  models: ProviderModelList['models']
  sourceFetchedAt: string
  stale: boolean
  warnings: string[]
  auth: import('@demicodes/provider').ProviderAuthState
  runtime: import('@demicodes/provider').ProviderRuntimeState
}

/**
 * The LLM module's provider assembly: one base provider runtime per entry,
 * built from vault credentials through the registered families. The cache
 * invalidates when an entry is edited or deleted.
 */
export class ProviderAssembly {
  private readonly cache = new Map<string, {
    entry: ProviderEntry;
    provider: Provider
  }>()

  constructor(
    private readonly vault: ProviderVault,
    private readonly types: Record<string, ProviderType>,
    private readonly quotas: AccountQuotas,
    private readonly vendors: VendorCatalog,
    private readonly catalogs: ModelCatalogCache,
  ) {}

  /** The registered families of one credential kind. */
  typesOf(credential: ProviderType['credential']): string[] {
    return Object.entries(this.types)
      .filter(([, type]) => type.credential === credential)
      .map(([name]) => name)
  }

  /**
   * Builds a provider through a registered type factory without a provider row
   * (login flows): its accounts are the staged pool's until it is published.
   */
  buildDetached(
    providerType: string,
    options: {
      id: string;
      label: string;
      credentialPool: CredentialPool
    }
  ): Provider {
    const type = this.types[providerType]
    if (!type)
      throw new Error(`Unknown provider type "${providerType}"`)
    return type.create({
      providerId: options.id,
      label: options.label,
      config: { kind: 'subscription', providerType },
      credentialPool: options.credentialPool,
    })
  }

  /**
   * The base provider instance behind an entry, or null when unknown; edits
   * invalidate its identity.
   */
  async providerFor(
    providerId: string
  ): Promise<{
    entry: ProviderEntry;
    provider: Provider
  } | null> {
    const entry = await this.vault.get(providerId)
    if (!entry)
      return null
    const cached = this.cache.get(providerId)
    if (cached &&
      cached.entry.label === entry.label &&
      cached.entry.activeCredentialId === entry.activeCredentialId &&
      JSON.stringify(cached.entry.config) === JSON.stringify(entry.config)) {
      return { entry, provider: cached.provider }
    }
    const provider = await this.build(entry, entry.activeCredentialId)
    this.cache.set(providerId, { entry, provider })
    return { entry, provider }
  }

  /** The entry's public facts, with the usage kept for each of its accounts. */
  async details(entry: ProviderEntry, provider: Provider) {
    const usage = new Map((await this.vault.accounts(entry.id)).map(
      account => [account.id, this.quotas.latest(entry.id, account)] as const
    ))
    return providerDetails(provider, {
      requireAccount: entry.config.kind === 'subscription',
      usage
    })
  }

  /**
   * Builds an independent session provider from the entry snapshot selected for
   * this request.
   */
  forSession(
    entry: ProviderEntry,
    session: SessionProviderContext
  ): Promise<Provider> {
    return this.build(entry, entry.activeCredentialId, session)
  }

  /**
   * The entry's provider standing for one of its accounts, whichever is
   * active; null when the entry has no such account.
   */
  async forAccount(
    entry: ProviderEntry,
    credentialId: string
  ): Promise<Provider | null> {
    if (credentialId === entry.activeCredentialId)
      return (await this.providerFor(entry.id))?.provider ?? null
    return await this.vault.account(entry.id, credentialId)
      ? this.build(entry, credentialId)
      : null
  }

  private async build(
    entry: ProviderEntry,
    credentialId: string | null,
    session?: SessionProviderContext
  ): Promise<Provider> {
    const type = this.types[entry.config.providerType]
    if (!type)
      throw new Error(`Unknown provider type "${entry.config.providerType}"`)
    const account = credentialId
      ? await this.vault.account(entry.id, credentialId)
      : null
    return type.create({
      providerId: entry.id,
      label: entry.label,
      config: entry.config,
      credentialPool: this.vault.credentialPool(entry.id),
      ...(account ? {
        credentialId: account.id,
        quotaSnapshots: this.quotas.keeper(entry.id, account),
      } : {}),
      ...(session ? { session } : {}),
    })
  }

  /** Forgets what is held of a deleted provider. */
  async deleteProviderState(providerId: string): Promise<void> {
    await this.invalidate(providerId)
    this.quotas.forget(providerId)
  }

  async invalidate(providerId: string): Promise<void> {
    this.cache.delete(providerId)
    await this.catalogs.invalidate(providerId)
  }

  async close(): Promise<void> {
    await this.catalogs.close()
  }

  /**
   * A registered family's credential kind, or null for an unknown type — the
   * create endpoint's validation.
   */
  credentialOf(providerType: string): ProviderType['credential'] | null {
    return this.types[providerType]?.credential ?? null
  }

  /**
   * The provider **Test** button: one cheap real request against the
   * endpoint/key — first streamed event wins, errors report the provider's
   * own message.
   */
  /**
   * One minimal request to the model the caller names: a plan can cover some
   * models and refuse others, and which ones the user keeps enabled is the
   * browser's knowledge. A test that ran and failed is a result, not an
   * error: `message` is the provider's own text, passed through.
   */
  async testProvider(
    providerId: string,
    modelId: string,
    /** The account to test, instead of the one the entry infers with. */
    credentialId?: string
  ): Promise<{
    ok: boolean;
    message?: string
    model?: string
  }> {
    const resolved = await this.providerFor(providerId)
    if (!resolved)
      return { ok: false, message: 'Unknown provider' }
    const { entry } = resolved
    const provider = credentialId
      ? await this.forAccount(entry, credentialId)
      : resolved.provider
    if (!provider)
      return { ok: false, message: 'Unknown account' }
    // A CLI transport runs on the conversation's execution target, never on this machine.
    if (provider.requiresProcessCapableHost) {
      return {
        ok: false,
        message: "This provider runs on a conversation's execution target; start a conversation to try it"
      }
    }
    const model = (await this.modelsOf(entry, provider)).find(
      candidate => candidate.id === modelId
    )
    if (!model)
      return { ok: false, message: `This provider lists no model ${modelId}` }
    const selection = await this.selection(
      entry.id,
      modelSelectionFromCatalog(entry.id, model)
    )
    const cancel = new AbortController()
    let runtime: AgentProvider | undefined
    try {
      runtime = await providerRuntime(
        provider,
        { providerId: entry.id, model: selection }
      )
      const run = runtime.run({
        sessionId: 'provider-test',
        turnId: createId(),
        requestId: createId(),
        outputLimit: selection.model.outputLimit,
        modelId: model.id,
        systemPrompt: 'Reply with the word ok.',
        cwd: '/',
        items: [{
            type: 'user_message',
            content: [{ type: 'text', text: 'ping' }]
          }],
        tools: [],
        thinking: null,
        cancel: cancel.signal,
      })
      for await (const event of run) {
        if (event.type === 'error')
          return { ok: false, message: event.message, model: model.displayName }
        if (event.type === 'abort')
          return {
            ok: false,
            message: 'Provider test was cancelled',
            model: model.displayName
          }
        return { ok: true, model: model.displayName }
      }
      return { ok: false, message: 'Provider returned no events', model: model.displayName }
    } catch (error) {
      return { ok: false, message: errorMessage(error), model: model.displayName }
    } finally {
      cancel.abort()
      await runtime?.dispose?.()
    }
  }

  /**
   * The aggregated catalog, grouped by entry: configured model metadata or
   * cached vendor/provider metadata, combined with current provider health.
   */
  async catalog(
    ownerUserId: string,
    refresh = false
  ): Promise<CatalogProvider[]> {
    const entries = await this.vault.list({ ownerUserId })
    return Promise.all(entries.map(async entry => {
      const provider = (await this.providerFor(entry.id))?.provider ?? null
      const list = await this.catalogOf(entry, provider, refresh)
      let health: Pick<CatalogProvider, 'auth' | 'runtime'> = {
        auth: { status: 'unknown' },
        runtime: { status: 'unknown' }
      }
      if (provider) {
        try {
          health = await providerDetails(
            provider,
            { requireAccount: entry.config.kind === 'subscription' }
          )
        } catch (error) {
          health = {
            auth: { status: 'error', message: errorMessage(error) },
            runtime: { status: 'unknown' }
          }
        }
      }
      return {
        providerId: entry.id,
        displayName: entry.label,
        requiresProcessCapableHost: provider?.requiresProcessCapableHost ??
          false,
        ...list,
        auth: health.auth,
        runtime: health.runtime,
      }
    }))
  }

  async selection(providerId: string, selection: ModelSelection) {
    const resolved = await this.providerFor(providerId)
    if (!resolved)
      throw new Error('Provider is no longer available')
    return this.selectionForEntry(resolved.entry, selection)
  }

  selectionForEntry(
    entry: ProviderEntry,
    selection: ModelSelection
  ): ModelSelection {
    if (entry.config.kind !== 'api_key' || !entry.config.models)
      return selection
    const model = entry.config.models.find(
      model => model.id === selection.model.id
    )
    if (!model)
      throw new Error('Model is not configured')
    return applyConfiguredModel(entry.id, model, selection)
  }

  private async catalogOf(
    entry: ProviderEntry,
    provider: Provider | null,
    refresh = false
  ) {
    if (entry.config.kind === 'api_key' && entry.config.models) {
      return {
        models: entry.config.models.map(model => configuredCatalogModel(entry.id, model)),
        sourceFetchedAt: '1970-01-01T00:00:00.000Z',
        stale: false,
        warnings: [],
      }
    }
    try {
      const account = await provider?.credentials?.getActive()
      const key = createHash('sha256').update(JSON.stringify({
        config: entry.config,
        account: account?.credentialId ?? null,
      })).digest('hex')
      return await this.catalogs.get(entry.id, key, async signal => {
        const raw = entry.config.kind === 'api_key' && entry.config.vendorId
          ? await this.vendors.models(entry.config.vendorId, entry.id, true, signal)
          : await provider?.listModels?.({ refresh: true, signal })
        signal.throwIfAborted()
        const list = raw ? withCatalogProviderId(raw, entry.id) : null
        return {
          models: list?.models ?? [],
          sourceFetchedAt: list?.sourceFetchedAt ?? '1970-01-01T00:00:00.000Z',
          stale: list?.stale ?? false,
          warnings: list?.warnings ?? [],
        }
      }, refresh)
    } catch (error) {
      return {
        models: [],
        sourceFetchedAt: '1970-01-01T00:00:00.000Z',
        stale: true,
        warnings: [errorMessage(error)],
      }
    }
  }

  private async modelsOf(
    entry: ProviderEntry,
    provider: Provider | null
  ): Promise<ProviderModelList['models']> {
    return (await this.catalogOf(entry, provider)).models
  }
}

/**
 * The ledger row appender used by the metering wrap — one row per provider
 * request.
 */
export function usageAppender(
  control: ControlService,
  context: {
    userId: string;
    conversationId: string;
    providerId: string
  },
) {
  return (
    usage: {
      inputTokens: number;
      outputTokens: number;
      cacheReadTokens: number;
      cacheWriteTokens: number
    },
    request: { modelId: string }
  ) => {
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

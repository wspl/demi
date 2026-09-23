import { computed, onScopeDispose, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { z } from 'zod'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { createQuotaRefreshCache } from '@demicodes/web-ui/settings/quota-refresh'
import type { ProviderLoginPhase } from '@demicodes/web-ui/settings/types'
import {
  defaultApiVendors,
  defaultEndpointUrl,
} from '@demicodes/web-ui/settings/provider-defaults'
import {
  WIRE_API_LABELS,
  type SettingsModelDraft,
  type SettingsQuotaRefresh,
  type SettingsModelEditor,
  type SettingsProviderCli,
  type SettingsProviderEntry,
  type SettingsProviderOperation,
  type SettingsProviderModel,
  type SettingsVendor,
  type SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import { providerSchema } from '../api/contracts'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { subscriptionName, type ProductProvider } from '../state/catalog'

export const useProviderSettings = defineStore('provider-settings', () => {
  const resources = useResources()
  const product = useProduct()
  const drafts = ref<ProductProvider[]>([])
  const modelEditor = ref<SettingsModelEditor | null>(null)
  const edits = ref<Record<string, Partial<SettingsProviderEntry>>>({})
  const manualDrafts = ref<Record<string, SettingsProviderModel[]>>({})
  const operations = ref<Record<string, SettingsProviderOperation>>({})
  const refreshingUsage = ref<Record<string, Record<string, SettingsQuotaRefresh>>>({})
  const quotaRefreshCache = createQuotaRefreshCache()
  const testing = computed(
    () =>
      Object.keys(operations.value).find(
        (id) => operations.value[id]?.kind === 'testing',
      ) ?? null,
  )
  const refreshing = computed(
    () =>
      Object.keys(operations.value).find(
        (id) => operations.value[id]?.kind === 'refreshing',
      ) ?? null,
  )
  const testResults = ref<
    Record<
      string,
      {
        state: 'ready' | 'error'
        testPassed?: boolean
        detail?: string
        testedWith?: string
      }
    >
  >({})
  /** Each process provider's CLI, as last read (`claude-code.md` § What the user sees). */
  const clis = ref<Record<string, SettingsProviderCli>>({})
  let lifetime = new AbortController()
  const writes = new SerialQueue()
  const providers = computed(() => {
    const configured = resources.providers.map((provider) => ({
      ...provider,
      ...testResults.value[provider.id],
      ...(provider.runsOnHost ? { cli: clis.value[provider.id] ?? null } : {}),
      ...edits.value[provider.id],
      ...(manualDrafts.value[provider.id]
        ? {
            modelSource: 'manual' as const,
            models: manualDrafts.value[provider.id],
          }
        : {}),
    }))
    const subscriptions = (product.vendors?.subscriptions ?? []).map(
      (subscription) =>
        configured.find(
          (provider) =>
            provider.kind === 'subscription' &&
            provider.providerType === subscription.providerType,
        ) ??
        emptyProvider(
          subscription.providerType,
          subscriptionName(subscription.providerType),
          'subscription',
        ),
    )
    const remaining = configured.filter(
      (provider) =>
        !subscriptions.some((subscription) => subscription.id === provider.id),
    )
    return [...subscriptions, ...remaining, ...drafts.value]
  })

  function report(error: unknown): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      reportError('Provider operation failed', error, { userVisible: true })
    }
  }

  async function run(
    id: string,
    operation: SettingsProviderOperation,
    action: (signal: AbortSignal) => Promise<void>,
  ): Promise<void> {
    if (operations.value[id]) {
      return
    }
    const current = lifetime
    operations.value[id] = operation
    try {
      await writes.run(async () => {
        current.signal.throwIfAborted()
        await action(current.signal)
      })
    } finally {
      if (current === lifetime) {
        delete operations.value[id]
      }
    }
  }

  function perform(
    id: string,
    operation: SettingsProviderOperation,
    action: (signal: AbortSignal) => Promise<void>,
  ): void {
    const current = lifetime
    void run(id, operation, action).catch((error) => {
      if (!current.signal.aborted) {
        report(error)
      }
    })
  }

  function emptyProvider(
    id: string,
    name: string,
    kind: 'api_key' | 'subscription',
  ): ProductProvider {
    return {
      id,
      name,
      kind,
      providerType: id,
      configured: false,
      keyConfigured: false,
      vendorId: null,
      baseUrl: '',
      wireApi: 'openai-responses',
      apiKey: '',
      modelSource: 'catalog',
      catalogFetched: null,
      stale: false,
      state: 'unconfigured',
      enabled: true,
      models: [],
      accounts: [],
      logo: null,
    }
  }

  function select(id: string): void {
    resources.selectedProviderId = id
    resources.providerDetailOpen = true
  }

  function vendorDraft(vendor: SettingsVendor): ProductProvider {
    const provider = emptyProvider(crypto.randomUUID(), vendor.name, 'api_key')
    provider.vendorId = vendor.id
    provider.baseUrl = vendor.baseUrl ?? ''
    provider.wireApi = vendor.wireApi
    return provider
  }

  function addProvider(vendor: SettingsVendor): void {
    const provider = vendorDraft(vendor)
    drafts.value.push(provider)
    select(provider.id)
  }

  watch(
    () => [
      product.vendors,
      JSON.stringify(resources.providers.map((provider) => provider.id)),
    ],
    () => {
      const defaults = defaultApiVendors(resources.vendors, [
        ...resources.providers,
        ...drafts.value,
      ])
      for (const vendor of defaults) {
        const provider = vendorDraft(vendor)
        provider.name = `${vendor.name} API`
        drafts.value.push(provider)
      }
    },
    { immediate: true },
  )

  function addEndpoint(wireApi: SettingsWireApi): void {
    const provider = emptyProvider(
      crypto.randomUUID(),
      `${WIRE_API_LABELS[wireApi]} API`,
      'api_key',
    )
    provider.wireApi = wireApi
    provider.baseUrl = defaultEndpointUrl(resources.vendors, wireApi)
    provider.modelSource = 'manual'
    drafts.value.push(provider)
    select(provider.id)
  }

  function modelConfig(model: SettingsModelDraft) {
    if (model.contextWindow === null) {
      throw new Error(`Enter a context window for ${model.name || model.id}.`)
    }
    return {
      id: model.id,
      displayName: model.name || model.id,
      contextWindow: model.contextWindow,
      outputLimit: model.outputLimit,
      thinkingEfforts: model.efforts,
      acceptedExtensions:
        model.extensions?.map((extension) => extension.replace(/^\./, '')) ??
        null,
      fastTier: model.fastTier,
    }
  }

  function canPersistDraft(provider: ProductProvider): boolean {
    return (
      !!provider.apiKey &&
      (provider.modelSource !== 'manual' || provider.models.length > 0)
    )
  }

  async function saveDraft(provider: ProductProvider): Promise<void> {
    const signal = lifetime.signal
    const family =
      provider.wireApi === 'anthropic-messages'
        ? 'anthropic'
        : provider.wireApi === 'google-generative'
          ? 'google'
          : 'openai'
    const response = await apiRequest('/providers', {
      method: 'POST',
      signal,
      ...jsonBody({
        ...(provider.vendorId
          ? { vendorId: provider.vendorId }
          : {
              providerType: family,
              ...(family === 'openai'
                ? {
                    wireApi:
                      provider.wireApi === 'openai-chat'
                        ? 'chat-completions'
                        : 'responses',
                  }
                : {}),
            }),
        label: provider.name,
        apiKey: provider.apiKey,
        ...(provider.baseUrl ? { baseUrl: provider.baseUrl } : {}),
        ...(provider.modelSource === 'manual'
          ? { models: provider.models.map(modelConfig) }
          : {}),
      }),
    })
    const saved = await readResponse(
      response,
      z.object({ provider: providerSchema }),
    )
    signal.throwIfAborted()
    provider.apiKey = ''
    resources.hideProvider(saved.provider.id, provider.enabled)
    await product.revalidate(true)
    signal.throwIfAborted()
    select(saved.provider.id)
    drafts.value = drafts.value.filter((draft) => draft.id !== provider.id)
  }

  async function patch(
    provider: SettingsProviderEntry,
    body: unknown,
  ): Promise<void> {
    const signal = lifetime.signal
    await apiRequest(`/providers/${encodeURIComponent(provider.id)}`, {
      method: 'PATCH',
      signal,
      ...jsonBody(body),
    })
    signal.throwIfAborted()
    delete testResults.value[provider.id]
    await product.revalidate(true)
    signal.throwIfAborted()
  }

  function change(
    provider: SettingsProviderEntry,
    changes: Partial<SettingsProviderEntry>,
  ): void {
    if (changes.enabled !== undefined) {
      const draft = drafts.value.find((entry) => entry.id === provider.id)
      if (draft) {
        draft.enabled = changes.enabled
      } else {
        resources.hideProvider(provider.id, changes.enabled)
      }
      return
    }
    if (operations.value[provider.id]) {
      return
    }
    changes = { ...edits.value[provider.id], ...changes }
    edits.value[provider.id] = changes
    const draft = drafts.value.find((entry) => entry.id === provider.id)
    if (draft) {
      Object.assign(draft, changes)
      if (!canPersistDraft(draft)) {
        delete edits.value[provider.id]
        return
      }
    }
    perform(provider.id, { kind: 'saving' }, async (signal) => {
      if (draft) {
        await saveDraft(draft)
        delete edits.value[provider.id]
        return
      }
      if (changes.modelSource === 'manual') {
        manualDrafts.value[provider.id] = provider.models.map((model) => ({
          ...model,
        }))
        if (
          !provider.models.length ||
          provider.models.some((model) => model.contextWindow === null)
        ) {
          return
        }
      }
      if (changes.modelSource === 'catalog') {
        delete manualDrafts.value[provider.id]
      }
      await patch(provider, {
        ...(changes.name !== undefined ? { label: changes.name } : {}),
        ...(changes.baseUrl !== undefined
          ? { baseUrl: changes.baseUrl || null }
          : {}),
        ...(changes.apiKey ? { apiKey: changes.apiKey } : {}),
        ...(changes.modelSource
          ? {
              models:
                changes.modelSource === 'catalog'
                  ? null
                  : provider.models.map(modelConfig),
            }
          : {}),
      })
      if (changes.modelSource) {
        delete manualDrafts.value[provider.id]
      }
      delete edits.value[provider.id]
    })
  }

  function removeProvider(id: string): void {
    perform(id, { kind: 'removing' }, async (signal) => {
      if (drafts.value.some((provider) => provider.id === id)) {
        drafts.value = drafts.value.filter((provider) => provider.id !== id)
        return
      }
      await apiRequest(`/providers/${encodeURIComponent(id)}`, {
        method: 'DELETE',
        signal,
      })
      signal.throwIfAborted()
      await product.revalidate(true)
    })
  }

  async function saveModels(
    provider: SettingsProviderEntry,
    models: SettingsProviderModel[],
  ): Promise<void> {
    if (operations.value[provider.id]) {
      throw new Error('Wait for the current provider operation to finish.')
    }
    const draft = drafts.value.find((entry) => entry.id === provider.id)
    if (draft) {
      draft.models = models
      if (!canPersistDraft(draft)) {
        return
      }
    }
    await run(provider.id, { kind: 'saving' }, async (signal) => {
      signal.throwIfAborted()
      if (draft) {
        await saveDraft(draft)
        return
      }
      await patch(provider, { models: models.map(modelConfig) })
      delete manualDrafts.value[provider.id]
      delete edits.value[provider.id]
    })
  }

  async function saveModel(
    provider: SettingsProviderEntry,
    model: SettingsModelDraft,
    original: SettingsProviderModel | null,
  ): Promise<void> {
    const models = original
      ? provider.models.map((entry) =>
          entry.id === original.id
            ? {
                ...model,
                enabled: entry.enabled,
              }
            : entry,
        )
      : [
          ...provider.models,
          {
            ...model,
            enabled: true,
          },
        ]
    try {
      await saveModels(provider, models)
    } catch (error) {
      report(error)
      throw error
    }
  }

  /**
   * Tests with the first model the user keeps enabled; the page offers no test
   * without one. A subscription names the account to test, in use or not.
   */
  function test(provider: SettingsProviderEntry, accountId?: string): void {
    const model = provider.models.find((candidate) => candidate.enabled)
    if (!model) {
      return
    }
    const operation = accountId
      ? { kind: 'account' as const, accountId, action: 'test' as const }
      : { kind: 'testing' as const }
    perform(provider.id, operation, async (signal) => {
      try {
        const response = await apiRequest(
          `/providers/${encodeURIComponent(provider.id)}/test`,
          {
            method: 'POST',
            signal,
            ...jsonBody({ modelId: model.id, ...(accountId ? { credentialId: accountId } : {}) }),
          },
        )
        const result = await readResponse(
          response,
          z.object({
            ok: z.boolean(),
            message: z.string().optional(),
            model: z.string().optional(),
          }),
        )
        signal.throwIfAborted()
        testResults.value[provider.id] = {
          state: result.ok ? 'ready' : 'error',
          testPassed: result.ok,
          detail: result.ok ? undefined : result.message ?? 'The provider gave no reason.',
          testedWith: result.model,
        }
      } catch (error) {
        signal.throwIfAborted()
        testResults.value[provider.id] = {
          state: 'error',
          detail: error instanceof Error ? error.message : String(error),
        }
        throw error
      }
    })
  }

  const cliSchema = z.object({
    newest: z.union([z.object({ version: z.string() }), z.object({ error: z.string() })]),
    install: z.union([
      z.object({ state: z.literal('installing') }),
      z.object({ state: z.literal('installed') }),
      z.object({ state: z.literal('failed'), message: z.string() }),
    ]).nullable(),
    machines: z.array(z.object({
      deviceId: z.string(),
      name: z.string(),
      versions: z.array(z.string()).nullable(),
    })),
  })

  /**
   * Reads a process provider's CLI. An install under way is read again until
   * it ends: it is the one thing here that changes without the user.
   */
  async function readCli(providerId: string, refresh: boolean, signal: AbortSignal): Promise<void> {
    const path = `/providers/${encodeURIComponent(providerId)}/cli${refresh ? '?refresh=true' : ''}`
    const cli = await readResponse(await apiRequest(path, { signal }), cliSchema)
    signal.throwIfAborted()
    clis.value[providerId] = {
      newest: cli.newest,
      install: cli.install,
      machines: cli.machines.map((machine) => ({
        id: machine.deviceId,
        name: machine.name,
        versions: machine.versions,
      })),
    }
    if (cli.install?.state === 'installing') {
      window.setTimeout(() => {
        if (!signal.aborted) {
          void readCli(providerId, false, signal).catch(() => {})
        }
      }, 2_000)
    }
  }

  /** Loads the CLI of a provider the page is showing; its failure is not the page's. */
  function loadCli(provider: SettingsProviderEntry): void {
    if (!provider.runsOnHost || provider.configured === false) {
      return
    }
    void readCli(provider.id, false, lifetime.signal).catch((error) => {
      if (!lifetime.signal.aborted) {
        reportError('Could not read the command-line tool', error)
      }
    })
  }

  function checkCli(provider: SettingsProviderEntry): void {
    perform(provider.id, { kind: 'cli' }, async (signal) => {
      try {
        await readCli(provider.id, true, signal)
      } catch (error) {
        report(error)
      }
    })
  }

  function installCli(provider: SettingsProviderEntry): void {
    perform(provider.id, { kind: 'cli' }, async (signal) => {
      try {
        await apiRequest(`/providers/${encodeURIComponent(provider.id)}/cli/install`, { method: 'POST', signal })
        await readCli(provider.id, false, lifetime.signal)
      } catch (error) {
        report(error)
      }
    })
  }

  async function refreshUsage(
    provider: SettingsProviderEntry,
    accountId: string,
    automatic = false,
  ): Promise<void> {
    if (refreshingUsage.value[provider.id]?.[accountId]) {
      if (!automatic) {
        refreshingUsage.value[provider.id]![accountId] = 'manual'
      }
      return
    }
    if (automatic && quotaRefreshCache.isFresh(provider.id, accountId)) {
      return
    }
    const current = lifetime
    refreshingUsage.value[provider.id] ??= {}
    refreshingUsage.value[provider.id]![accountId] = automatic ? 'automatic' : 'manual'
    try {
      await apiRequest(`/providers/${encodeURIComponent(provider.id)}/quota`, {
        method: 'POST',
        signal: current.signal,
        ...jsonBody({ credentialId: accountId }),
      })
      current.signal.throwIfAborted()
      await product.refresh()
    } catch (error) {
      // Automatic refresh stays silent unless the user explicitly joins it.
      if (!current.signal.aborted && refreshingUsage.value[provider.id]?.[accountId] === 'manual') {
        reportError('Could not refresh usage', error, { userVisible: true })
      }
    } finally {
      if (current === lifetime) {
        const pending = refreshingUsage.value[provider.id]!
        if (!current.signal.aborted) {
          quotaRefreshCache.record(provider.id, accountId)
        }
        delete pending[accountId]
        if (!Object.keys(pending).length) {
          delete refreshingUsage.value[provider.id]
        }
      }
    }
  }

  function refresh(provider: SettingsProviderEntry): void {
    perform(provider.id, { kind: 'refreshing' }, async (signal) => {
      await product.loadModels(true)
      signal.throwIfAborted()
      await product.refresh()
    })
  }

  function accountAction(
    provider: SettingsProviderEntry,
    id: string,
    action: 'activate' | 'remove',
  ): void {
    perform(
      provider.id,
      { kind: 'account', accountId: id, action },
      async (signal) => {
        const base = `/providers/${encodeURIComponent(provider.id)}/accounts`
        await apiRequest(
          action === 'activate'
            ? `${base}/active`
            : `${base}/${encodeURIComponent(id)}`,
          {
            method: action === 'activate' ? 'PUT' : 'DELETE',
            signal,
            ...(action === 'activate' ? jsonBody({ credentialId: id }) : {}),
          },
        )
        signal.throwIfAborted()
        await product.revalidate(true)
      },
    )
  }

  const loginSchema = z.object({
    login: z.discriminatedUnion('status', [
      z.object({
        status: z.literal('pending'),
        verificationUrl: z.string().nullable(),
        userCode: z.string().nullable(),
      }),
      z.object({
        status: z.literal('completed'),
        providerId: z.string(),
        credentialId: z.string(),
      }),
      z.object({
        status: z.literal('failed'),
        message: z.string(),
      }),
    ]),
  })
  const login = ref<{
    provider: ProductProvider
    phase: ProviderLoginPhase
  } | null>(null)
  let loginController: AbortController | null = null
  let loginId: string | null = null
  let loginTimer: ReturnType<typeof setTimeout> | null = null

  function cancelLogin(): void {
    loginController?.abort()
    loginController = null
    if (loginTimer !== null) {
      clearTimeout(loginTimer)
    }
    loginTimer = null
    const id = loginId
    loginId = null
    if (id) {
      void apiRequest(
        `/providers/subscription-login/${encodeURIComponent(id)}`,
        {
          method: 'DELETE',
        },
      ).catch(report)
    }
  }

  function closeLogin(): void {
    cancelLogin()
    login.value = null
  }

  async function pollLogin(controller: AbortController): Promise<void> {
    try {
      const response = await apiRequest(
        `/providers/subscription-login/${encodeURIComponent(loginId!)}`,
        { signal: controller.signal },
      )
      const result = (await readResponse(response, loginSchema)).login
      controller.signal.throwIfAborted()
      if (!login.value) {
        return
      }
      if (result.status === 'completed') {
        loginId = null
        await product.refresh()
        controller.signal.throwIfAborted()
        const provider = resources.providers.find(
          (entry) => entry.id === result.providerId,
        )
        // The account this login added, which is not the active one when another already was.
        const added = provider?.accounts.find((account) => account.id === result.credentialId)
        login.value.phase = {
          kind: 'done',
          account: added?.label ?? 'Account connected',
          active: added?.active ?? false,
        }
        select(result.providerId)
      } else if (result.status === 'failed') {
        loginId = null
        login.value.phase = {
          kind: 'failed',
          message: result.message,
        }
      } else {
        if (result.verificationUrl && result.userCode) {
          login.value.phase = {
            kind: 'device',
            url: result.verificationUrl,
            code: result.userCode,
          }
        }
        loginTimer = setTimeout(() => void pollLogin(controller), 1500)
      }
    } catch (error) {
      if (!controller.signal.aborted && login.value) {
        login.value.phase = {
          kind: 'failed',
          message: error instanceof Error ? error.message : String(error),
        }
      }
    }
  }

  async function beginLogin(entry: SettingsProviderEntry): Promise<void> {
    cancelLogin()
    const provider = providers.value.find(
      (provider) => provider.id === entry.id,
    )!
    const controller = new AbortController()
    loginController = controller
    if (provider.providerType === 'claude-code') {
      login.value = {
        provider,
        phase: {
          kind: 'token',
          command: 'claude setup-token',
          prefix: 'sk-ant-oat01-',
          install: {
            label: 'Get Claude Code',
            url: 'https://code.claude.com/docs/en/setup',
          },
        },
      }
      return
    }
    login.value = {
      provider,
      phase: { kind: 'starting' },
    }
    try {
      const response = await apiRequest(
        provider.configured
          ? `/providers/${encodeURIComponent(provider.id)}/accounts/login`
          : '/providers/subscription-login',
        {
          method: 'POST',
          ...jsonBody({
            providerType: provider.providerType,
            label: provider.name,
          }),
        },
      )
      const result = await readResponse(
        response,
        z.object({
          login: z.object({
            id: z.string(),
            status: z.literal('pending'),
          }),
        }),
      )
      if (controller.signal.aborted) {
        await apiRequest(
          `/providers/subscription-login/${encodeURIComponent(result.login.id)}`,
          { method: 'DELETE' },
        )
        return
      }
      loginId = result.login.id
      await pollLogin(controller)
    } catch (error) {
      if (!controller.signal.aborted && login.value) {
        login.value.phase = {
          kind: 'failed',
          message: error instanceof Error ? error.message : String(error),
        }
      }
    }
  }

  async function submitToken(token: string): Promise<void> {
    const current = login.value
    const controller = loginController
    if (!current || !controller) {
      return
    }
    current.phase = { kind: 'starting' }
    try {
      await apiRequest(
        current.provider.configured
          ? `/providers/${encodeURIComponent(current.provider.id)}/accounts`
          : '/providers/setup-token',
        {
          method: 'POST',
          signal: controller.signal,
          ...jsonBody(
            current.provider.configured
              ? { token }
              : {
                  token,
                  label: current.provider.name,
                },
          ),
        },
      )
      await product.revalidate(true)
      controller.signal.throwIfAborted()
      current.phase = {
        kind: 'done',
        account: 'Account connected',
        active: true,
      }
    } catch (error) {
      if (!controller.signal.aborted) {
        current.phase = {
          kind: 'failed',
          message: error instanceof Error ? error.message : String(error),
        }
      }
    }
  }

  watch(
    () => useSession().user?.id,
    () => {
      lifetime.abort()
      lifetime = new AbortController()
      closeLogin()
      drafts.value = []
      edits.value = {}
      modelEditor.value = null
      manualDrafts.value = {}
      testResults.value = {}
      operations.value = {}
      refreshingUsage.value = {}
      quotaRefreshCache.clear()
    },
  )

  onScopeDispose(() => {
    lifetime.abort()
    quotaRefreshCache.clear()
    cancelLogin()
  })

  return {
    modelEditor,
    operations,
    refreshingUsage,
    providers,
    testing,
    refreshing,
    login,
    report,
    change,
    addProvider,
    addEndpoint,
    removeProvider,
    beginLogin,
    closeLogin,
    test,
    refresh,
    accountAction,
    saveModel,
    saveModels,
    submitToken,
    refreshUsage,
    loadCli,
    checkCli,
    installCli,
  }
})

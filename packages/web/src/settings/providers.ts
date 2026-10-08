import { computed, onScopeDispose, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { fileExtensionSchema } from '@demicodes/protocol'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import { createQuotaRefreshCache } from '@demicodes/web-ui/settings/quota-refresh'
import type { ProviderLoginPhase } from '@demicodes/web-ui/settings/types'
import { defaultEndpointUrl } from '@demicodes/web-ui/settings/provider-defaults'
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
import {
  loginAnswerSchema,
  loginStartedSchema,
  providerAnswerSchema,
  providerCliSchema,
  testResultSchema,
  type ActivateAccount,
  type ConfiguredModel,
  type CreateProvider,
  type ProviderPatch,
  type LoginCode,
  type QuotaRequest,
  type SubscriptionLogin,
  type TestRequest,
} from '../api/generated/web-api'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { subscriptionName, type ProductProvider } from '../state/catalog'

export const useProviderSettings = defineStore('provider-settings', () => {
  const resources = useResources()
  const product = useProduct()
  /**
   * The provider being added: its form shows in the page's detail, and it
   * joins the list only once it is saved. Choosing another provider drops it.
   */
  const draft = ref<ProductProvider | null>(null)
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
  /** The operation running for each provider, which an edit made during a test waits for. */
  const running = new Map<string, Promise<void>>()
  const providers = computed(() => {
    const configured = resources.providers.map((provider) => ({
      ...provider,
      ...testResults.value[provider.id],
      ...(provider.cliPackage === null ? {} : { cli: clis.value[provider.id] ?? null }),
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
    return [...subscriptions, ...remaining]
  })

  function report(error: unknown): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      reportError('Provider Operation Failed', error, { userVisible: true })
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
    const done = writes.run(async () => {
      current.signal.throwIfAborted()
      await action(current.signal)
    })
    running.set(id, done.then(() => {}, () => {}))
    try {
      await done
    } finally {
      running.delete(id)
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
      // Not configured, the entry has no provider to name a CLI.
      cliPackage: null,
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

  /** Opens `provider`'s form in place of any other draft. */
  function openDraft(provider: ProductProvider): void {
    if (draft.value) {
      delete edits.value[draft.value.id]
    }
    draft.value = provider
    select(provider.id)
  }

  function addProvider(vendor: SettingsVendor): void {
    openDraft(vendorDraft(vendor))
  }

  /** Drops the provider being added; nothing of it was saved. */
  function discardDraft(): void {
    if (!draft.value || operations.value[draft.value.id]) {
      return
    }
    delete edits.value[draft.value.id]
    draft.value = null
  }

  // Choosing another provider leaves the form of the one being added.
  watch(
    () => resources.selectedProviderId,
    (id) => {
      if (draft.value && id !== draft.value.id) {
        discardDraft()
      }
    },
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
    openDraft(provider)
  }

  function modelConfig(model: SettingsModelDraft): ConfiguredModel {
    if (model.contextWindow === null) {
      throw new Error(`Enter a context window for ${model.name || model.id}.`)
    }
    return {
      id: model.id,
      displayName: model.name || model.id,
      contextWindow: model.contextWindow,
      outputLimit: model.outputLimit,
      thinkingEfforts: model.efforts,
      acceptedExtensions: model.extensions
        ? fileExtensionSchema.array().parse(model.extensions.map((extension) => extension.replace(/^\./, '')))
        : null,
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
    const shared = {
      label: provider.name,
      apiKey: provider.apiKey,
      ...(provider.baseUrl ? { baseUrl: provider.baseUrl } : {}),
      ...(provider.modelSource === 'manual'
        ? { models: provider.models.map(modelConfig) }
        : {}),
    }
    const body: CreateProvider = provider.vendorId
      ? { source: 'vendor', vendorId: provider.vendorId, ...shared }
      : {
          source: 'custom',
          providerType: family,
          ...(family === 'openai'
            ? { wireApi: provider.wireApi === 'openai-chat' ? 'chat-completions' : 'responses' }
            : {}),
          ...shared,
        }
    const response = await apiRequest('/providers', {
      method: 'POST',
      signal,
      ...jsonBody(body),
    })
    const saved = await readResponse(response, providerAnswerSchema)
    signal.throwIfAborted()
    provider.apiKey = ''
    resources.hideProvider(saved.provider.id, provider.enabled)
    // The new entry's page opens once the channel brings it; the catalog
    // follows the channel's providers (`web-application.md` § Requests for
    // one action).
    await product.until((state) => state.providers.some((entry) => entry.id === saved.provider.id), signal)
    if (draft.value?.id === provider.id) {
      draft.value = null
    }
    select(saved.provider.id)
  }

  async function patch(
    provider: SettingsProviderEntry,
    body: ProviderPatch,
  ): Promise<void> {
    const signal = lifetime.signal
    await apiRequest(`/providers/${encodeURIComponent(provider.id)}`, {
      method: 'PATCH',
      signal,
      ...jsonBody(body),
    })
    signal.throwIfAborted()
    // The entry shows as the channel brings it, and the catalog follows the
    // channel's providers; nothing is read back.
    delete testResults.value[provider.id]
  }

  function change(
    provider: SettingsProviderEntry,
    changes: Partial<SettingsProviderEntry>,
  ): void {
    const adding = draft.value?.id === provider.id ? draft.value : null
    if (changes.enabled !== undefined) {
      if (adding) {
        adding.enabled = changes.enabled
      } else {
        resources.hideProvider(provider.id, changes.enabled)
      }
      return
    }
    const busy = operations.value[provider.id]
    if (busy && busy.kind !== 'testing' && busy.kind !== 'refreshing') {
      return
    }
    changes = { ...edits.value[provider.id], ...changes }
    edits.value[provider.id] = changes
    if (busy) {
      // A test or a refresh only reads: the edit shows at once and is saved when it ends.
      void running.get(provider.id)?.then(() => change(provider, {}))
      return
    }
    if (adding) {
      Object.assign(adding, changes)
      if (!canPersistDraft(adding)) {
        delete edits.value[provider.id]
        return
      }
    }
    perform(provider.id, { kind: 'saving' }, async (signal) => {
      if (adding) {
        await saveDraft(adding)
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
      await apiRequest(`/providers/${encodeURIComponent(id)}`, {
        method: 'DELETE',
        signal,
      })
      signal.throwIfAborted()
    })
  }

  async function saveModels(
    provider: SettingsProviderEntry,
    models: SettingsProviderModel[],
  ): Promise<void> {
    // A model saved during a test, a refresh or another save waits for it to
    // end, and for an edit queued behind it, rather than being refused.
    for (let pending = running.get(provider.id); pending; pending = running.get(provider.id)) {
      await pending
    }
    const adding = draft.value?.id === provider.id ? draft.value : null
    if (adding) {
      adding.models = models
      if (!canPersistDraft(adding)) {
        return
      }
    }
    await run(provider.id, { kind: 'saving' }, async (signal) => {
      signal.throwIfAborted()
      if (adding) {
        await saveDraft(adding)
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
            ...jsonBody({ modelId: model.id, ...(accountId ? { credentialId: accountId } : {}) } satisfies TestRequest),
          },
        )
        const result = await readResponse(response, testResultSchema)
        signal.throwIfAborted()
        testResults.value[provider.id] = result.type === 'passed'
          ? { state: 'ready', testPassed: true, testedWith: result.model }
          : { state: 'error', testPassed: false, detail: result.message, testedWith: result.model }
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

  /**
   * Reads a process provider's CLI. An install under way is read again until
   * it ends: it is the one thing here that changes without the user.
   */
  async function readCli(providerId: string, refresh: boolean, signal: AbortSignal): Promise<void> {
    const path = `/providers/${encodeURIComponent(providerId)}/cli${refresh ? '?refresh=true' : ''}`
    const cli = await readResponse(await apiRequest(path, { signal }), providerCliSchema)
    signal.throwIfAborted()
    clis.value[providerId] = {
      newest: cli.newest.type === 'read' ? { version: cli.newest.version } : { error: cli.newest.message },
      install: cli.install && (cli.install.state === 'failed'
        ? { state: 'failed', message: cli.install.message }
        : { state: cli.install.state }),
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
    if (provider.cli === undefined || provider.configured === false) {
      return
    }
    void readCli(provider.id, false, lifetime.signal).catch((error) => {
      if (!lifetime.signal.aborted) {
        reportError('Could Not Read the Command-Line Tool', error)
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
        ...jsonBody({ credentialId: accountId } satisfies QuotaRequest),
      })
      current.signal.throwIfAborted()
    } catch (error) {
      // Automatic refresh stays silent unless the user explicitly joins it.
      if (!current.signal.aborted && refreshingUsage.value[provider.id]?.[accountId] === 'manual') {
        reportError('Could Not Refresh Usage', error, { userVisible: true })
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
    perform(provider.id, { kind: 'refreshing' }, async () => {
      await product.loadModels(true)
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
            ...(action === 'activate' ? jsonBody({ credentialId: id } satisfies ActivateAccount) : {}),
          },
        )
        signal.throwIfAborted()
      },
    )
  }

  const login = ref<{
    provider: ProductProvider
    phase: ProviderLoginPhase
  } | null>(null)
  let loginController: AbortController | null = null
  let loginId: string | null = null
  let loginTimer: ReturnType<typeof setTimeout> | null = null
  /** When the last pasted code reached the login. */
  let codeSentAt = 0

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
    const polled = Date.now()
    try {
      const response = await apiRequest(
        `/providers/subscription-login/${encodeURIComponent(loginId!)}`,
        { signal: controller.signal },
      )
      const result = (await readResponse(response, loginAnswerSchema)).login
      controller.signal.throwIfAborted()
      if (!login.value) {
        return
      }
      if (result.status === 'completed') {
        loginId = null
        // The account the login added shows once the channel brings it.
        await product.until((state) => state.providers.some((entry) =>
          entry.id === result.providerId &&
          (entry.details.type !== 'read' || entry.details.accounts.some((account) => account.id === result.credentialId)),
        ), controller.signal)
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
        if (result.verificationUrl && result.needsCode) {
          // A pasted code keeps the dialog signing in until the login ends or
          // refuses it; an answer asked before the code reached the login
          // still names the refusal of the code before.
          const shown = login.value.phase.kind === 'code' ? login.value.phase : null
          const error = polled >= codeSentAt ? result.codeError ?? undefined : shown?.error
          login.value.phase = {
            kind: 'code',
            url: result.verificationUrl,
            submitted: !!shown?.submitted && !error,
            ...(error ? { error } : {}),
          }
        } else if (result.verificationUrl && result.userCode) {
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
          ...(provider.configured
            ? {}
            : jsonBody({ providerType: provider.providerType, label: provider.name } satisfies SubscriptionLogin)),
        },
      )
      const result = await readResponse(response, loginStartedSchema)
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

  /** Hands the code the user pasted to the sign-in, which then finishes on its own. */
  async function submitCode(code: string): Promise<void> {
    const current = login.value
    const controller = loginController
    const id = loginId
    if (!current || !controller || !id || current.phase.kind !== 'code') {
      return
    }
    current.phase = { kind: 'code', url: current.phase.url, submitted: true }
    codeSentAt = Number.POSITIVE_INFINITY
    try {
      await apiRequest(
        `/providers/subscription-login/${encodeURIComponent(id)}/code`,
        {
          method: 'POST',
          signal: controller.signal,
          ...jsonBody({ code } satisfies LoginCode),
        },
      )
      codeSentAt = Date.now()
    } catch (error) {
      if (!controller.signal.aborted && login.value === current) {
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
      draft.value = null
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
    draft,
    discardDraft,
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
    submitCode,
    refreshUsage,
    loadCli,
    checkCli,
    installCli,
  }
})

import { computed, onScopeDispose, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { fileExtensionSchema } from '@demicodes/protocol'
import { SerialQueue } from '@demicodes/utils'
import { reportError, shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { createQuotaRefreshCache } from '@demicodes/web-ui/settings/quota-refresh'
import type { ProviderLoginPhase } from '@demicodes/web-ui/settings/types'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { providerLogo } from '@demicodes/web-ui/settings/vendor-logos'
import { providerOperation, providerTarget } from '@demicodes/web-ui/settings/provider-operations'
import { operations } from '@demicodes/web-ui/ui/operations'
import {
  type SettingsCompatibleDraft,
  type SettingsModelDraft,
  type SettingsModelMatch,
  type SettingsQuotaRefresh,
  type SettingsModelEditor,
  type SettingsProviderCli,
  type SettingsProviderEntry,
  type SettingsProviderOperation,
  type SettingsProviderModel,
  type SettingsVendor,
} from '@demicodes/web-ui/settings/types'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  loginStartedSchema,
  modelSearchResultsSchema,
  providerAnswerSchema,
  providerCliSchema,
  testResultSchema,
  type ActivateAccount,
  type ConfiguredModel,
  type CreateProvider,
  type ProviderPatch,
  type ListedLogin,
  type LoginCode,
  type QuotaRequest,
  type SubscriptionLogin,
  type TestRequest,
} from '../api/generated/web-api'
import { useResources } from '../state/resources'
import { usePreferences } from '../state/preferences'
import { useProduct } from '../state/product'
import { subscriptionName, type ProductProvider } from '../state/catalog'

export const useProviderSettings = defineStore('provider-settings', () => {
  const resources = useResources()
  const preferences = usePreferences()
  const product = useProduct()
  const modelEditor = ref<SettingsModelEditor | null>(null)
  const edits = ref<Record<string, Partial<SettingsProviderEntry>>>({})
  const manualDrafts = ref<Record<string, SettingsProviderModel[]>>({})
  const refreshingUsage = ref<Record<string, Record<string, SettingsQuotaRefresh>>>({})
  /** The vendor's words for the last manual usage refresh that failed, per provider and account, until that account's next answer; the surfaces show them, no toast does. */
  const usageFailures = ref<Record<string, Record<string, ShownError>>>({})
  const quotaRefreshCache = createQuotaRefreshCache()
  /** Whether provider `id`'s operation of `kind` runs, as the page's one store of operations keeps it. */
  const runs = (id: string, kind: 'saving' | 'testing' | 'refreshing' | 'removing' | 'cli') =>
    operations.runs(providerOperation(id, { kind }))
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
  /** The provider operations on their way, by provider: each provider's in order. */
  const writes = new SerialQueue()
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

  /** The one toast of a provider operation that failed, titled by what failed. */
  function report(title: HeadlineText, error: unknown): void {
    if (error instanceof DOMException && error.name === 'AbortError') {
      return
    }
    if (!lifetime.signal.aborted) {
      reportError(title, error, { userVisible: true })
    }
  }

  /** What a failed operation's toast is titled, in the Writing page's alert style. */
  function failureTitle(operation: SettingsProviderOperation): HeadlineText {
    switch (operation.kind) {
      case 'saving':
        return 'Could Not Save the Provider'
      case 'testing':
        return 'Could Not Test the Provider'
      case 'refreshing':
        return 'Could Not Refresh the Models'
      case 'removing':
        return 'Could Not Remove the Provider'
      case 'cli':
        return 'Could Not Update the Command-Line Tool'
      case 'account':
        switch (operation.action) {
          case 'remove':
            return 'Could Not Remove the Account'
          case 'test':
            return 'Could Not Test the Account'
        }
    }
  }

  /**
   * Runs the provider's operation after its earlier writes, as one of the
   * page's operations (`operations.ts`): a control bound to it shows it, and
   * nothing starts while another of the provider's runs. Answers whether it
   * ran.
   */
  async function run(
    id: string,
    operation: SettingsProviderOperation,
    action: (signal: AbortSignal) => Promise<void>,
  ): Promise<boolean> {
    const current = lifetime
    const done = operations.run(providerOperation(id, operation), () =>
      writes.run(id, async () => {
        current.signal.throwIfAborted()
        await action(current.signal)
      }),
    )
    if (done === null) {
      return false
    }
    await done
    return true
  }

  /** Resolves once the provider does nothing, for work that waits for that rather than being refused. */
  async function untilIdle(id: string): Promise<void> {
    const any = providerOperation(id, { kind: 'saving' })
    while (operations.blocked(any)) {
      await operations.whenFree(any)
    }
  }

  /** Runs the operation and reports its failure; the promise ends with it, for the control that started it to show. */
  function perform(
    id: string,
    operation: SettingsProviderOperation,
    action: (signal: AbortSignal) => Promise<void>,
  ): Promise<void> {
    const current = lifetime
    return run(id, operation, action).then(
      () => {},
      (error) => {
        if (!current.signal.aborted) {
          report(failureTitle(operation), error)
        }
      },
    )
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
      compatible: false,
      baseUrl: '',
      wireApi: 'openai-responses',
      apiKey: '',
      modelSource: 'catalog',
      catalogFetched: null,
      state: 'setup',
      enabled: true,
      models: [],
      accounts: [],
      // A subscription not yet configured goes by its type; an added vendor's draft takes its mark below.
      logo: providerLogo({ kind, providerType: id, vendorId: null }),
    }
  }

  function select(id: string): void {
    resources.selectedProviderId = id
    resources.providerDetailOpen = true
  }

  /**
   * Adding asks only the kind and creates the entry at once: it joins the list
   * once the channel brings it, selected, and its page shows what it still
   * lacks (`models.md` § A provider is in the list from the start).
   */
  function create(body: CreateProvider): void {
    const current = lifetime
    void run('new', { kind: 'saving' }, async (signal) => {
      const response = await apiRequest('/providers', {
        method: 'POST',
        signal,
        ...jsonBody(body),
      })
      const saved = await readResponse(response, providerAnswerSchema)
      signal.throwIfAborted()
      await product.until((state) => state.providers.some((entry) => entry.id === saved.provider.id), signal)
      select(saved.provider.id)
    }).catch((error) => {
      if (!current.signal.aborted) {
        report('Could Not Add the Provider', error)
      }
    })
  }

  function addProvider(vendor: SettingsVendor): void {
    create({ source: 'vendor', vendorId: vendor.id })
  }

  /** A compatible API: its endpoint is what makes the entry one, so it is sent with the family. */
  function addCompatible(draft: SettingsCompatibleDraft): void {
    const { wireApi, label, baseUrl, apiKey } = draft
    const entry = { source: 'custom' as const, label, baseUrl, ...(apiKey ? { apiKey } : {}) }
    create(
      wireApi === 'anthropic-messages'
        ? { ...entry, providerType: 'anthropic' }
        : wireApi === 'google-generative'
          ? { ...entry, providerType: 'google' }
          : { ...entry, providerType: 'openai', wireApi: wireApi === 'openai-chat' ? 'chat-completions' : 'responses' },
    )
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
    if (changes.enabled !== undefined) {
      preferences.showProvider(provider.id, changes.enabled)
      return
    }
    // A test or a refresh only reads; any other operation of the provider
    // holds its fields, so a new edit then is refused, and one made before
    // waits for it too.
    const reading = runs(provider.id, 'testing') || runs(provider.id, 'refreshing')
    const holding = !reading && operations.blocked(providerOperation(provider.id, { kind: 'saving' }))
    if (holding && Object.keys(changes).length > 0) {
      return
    }
    changes = { ...edits.value[provider.id], ...changes }
    edits.value[provider.id] = changes
    if (reading || holding) {
      // The edit shows at once and is saved when the provider is free.
      void untilIdle(provider.id).then(() => change(provider, {}))
      return
    }
    perform(provider.id, { kind: 'saving' }, async (signal) => {
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
        // Only a compatible API's endpoint is edited, and the page never empties it.
        ...(changes.baseUrl !== undefined ? { baseUrl: changes.baseUrl } : {}),
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

  function removeProvider(id: string): Promise<void> {
    return perform(id, { kind: 'removing' }, async (signal) => {
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
    const save = async (signal: AbortSignal) => {
      signal.throwIfAborted()
      await patch(provider, { models: models.map(modelConfig) })
      delete manualDrafts.value[provider.id]
      delete edits.value[provider.id]
    }
    do {
      await untilIdle(provider.id)
    } while (!(await run(provider.id, { kind: 'saving' }, save)))
  }

  async function saveModel(
    provider: SettingsProviderEntry,
    model: SettingsModelDraft,
    original: SettingsProviderModel | null,
  ): Promise<void> {
    // An entry that follows its vendor's list starts a list of its own from it
    // (the vendor's copy of a model gives way to the one added), so adding removes none.
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
          ...provider.models.filter((entry) => provider.modelSource !== 'catalog' || entry.id !== model.id),
          {
            ...model,
            enabled: true,
          },
        ]
    // A refusal is the dialog's own, shown under its fields; it is no toast too.
    await saveModels(provider, models)
  }

  /**
   * The models.dev models `query` matches, filled from the entry's own vendor
   * where it lists them (`web-api.md` § Model configuration and provider
   * inspection); the backend reads its kept copy, so every keystroke may ask.
   */
  async function searchModels(
    providerId: string,
    query: string,
    signal: AbortSignal,
  ): Promise<SettingsModelMatch[]> {
    const response = await apiRequest(
      `/models/search?q=${encodeURIComponent(query)}&entry=${encodeURIComponent(providerId)}`,
      { signal },
    )
    const found = await readResponse(response, modelSearchResultsSchema)
    return found.results.map((match) => ({
      ref: match.ref,
      vendor: match.vendor,
      model: {
        id: match.id,
        name: match.displayName,
        contextWindow: match.contextWindow,
        outputLimit: match.outputLimit,
        efforts: match.thinkingEfforts,
        extensions: match.acceptedExtensions?.map((extension) => `.${extension}`) ?? null,
        fastTier: null,
      },
    }))
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
        report('Could Not Check for Updates', error)
      }
    })
  }

  function installCli(provider: SettingsProviderEntry): Promise<void> {
    return perform(provider.id, { kind: 'cli' }, async (signal) => {
      try {
        await apiRequest(`/providers/${encodeURIComponent(provider.id)}/cli/install`, { method: 'POST', signal })
        await readCli(provider.id, false, lifetime.signal)
      } catch (error) {
        report('Could Not Install the Command-Line Tool', error)
      }
    })
  }

  function clearUsageFailure(providerId: string, accountId: string): void {
    const failures = usageFailures.value[providerId]
    if (!failures) {
      return
    }
    delete failures[accountId]
    if (!Object.keys(failures).length) {
      delete usageFailures.value[providerId]
    }
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
      clearUsageFailure(provider.id, accountId)
    } catch (error) {
      // Automatic refresh stays silent unless the user explicitly joins it.
      if (!current.signal.aborted && refreshingUsage.value[provider.id]?.[accountId] === 'manual') {
        usageFailures.value[provider.id] ??= {}
        usageFailures.value[provider.id]![accountId] = shownError(error)
        // The account's line shows it, wherever its windows show; no toast.
        reportError('Could Not Refresh Usage', error)
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

  function removeAccount(provider: SettingsProviderEntry, id: string): Promise<void> {
    return perform(
      provider.id,
      { kind: 'account', accountId: id, action: 'remove' },
      async (signal) => {
        await apiRequest(`/providers/${encodeURIComponent(provider.id)}/accounts/${encodeURIComponent(id)}`, {
          method: 'DELETE',
          signal,
        })
        signal.throwIfAborted()
      },
    )
  }

  /**
   * Makes `id` the provider's active account as the user presses Activate:
   * the Active mark moves at once (it is a switch, `web-application.md`
   * § Responding to the user), the change is sent after the provider's
   * earlier writes, and a refusal moves the mark back with a toast.
   */
  function activateAccount(provider: SettingsProviderEntry, id: string): Promise<void> {
    const shown = product.change('providers', (state) => ({
      ...state,
      providers: state.providers.map((entry) =>
        entry.id === provider.id && entry.details.type === 'read'
          ? { ...entry, details: { ...entry.details, active: id } }
          : entry,
      ),
    }))
    const current = lifetime
    return writes
      .run(provider.id, async () => {
        current.signal.throwIfAborted()
        shown.send()
        await apiRequest(`/providers/${encodeURIComponent(provider.id)}/accounts/active`, {
          method: 'PUT',
          signal: current.signal,
          ...jsonBody({ credentialId: id } satisfies ActivateAccount),
        })
        shown.land()
      })
      .catch((error) => {
        shown.drop()
        report('Could Not Switch Accounts', error)
      })
  }

  /**
   * The sign-in the dialog shows (`providers.md` § Login and publication):
   * one this page started, whose `id` is null until the start answers, the
   * one a busy start named, or a running one the vault announced, as after
   * a reload. The vault announces each change of it on the channel, which
   * the dialog follows; `failure` is why this page could not start or
   * answer it, `code` where the last pasted code is, and `seen` whether the
   * channel has listed it.
   */
  const opened = ref<{
    provider: ProductProvider
    id: string | null
    failure: ShownError | null
    code: 'none' | 'sending' | 'sent'
    seen: boolean
  } | null>(null)
  /** The sign-ins this page closed, which it never opens again on its own. */
  const dismissed = ref<string[]>([])
  /** The starts on their way: the sign-in one makes is this page's, and not opened as another's. */
  const starts = ref(0)
  const logins = computed(() => product.snapshot?.logins ?? [])
  /** The opened sign-in as the channel last listed it. */
  const followed = computed(() => {
    const id = opened.value?.id
    return id ? logins.value.find((login) => login.id === id) ?? null : null
  })

  /** The provider a listed sign-in is for: its entry, or the subscription it makes. */
  function providerOf(login: ListedLogin): ProductProvider | undefined {
    return providers.value.find((provider) => login.providerId === null
      ? provider.kind === 'subscription' && provider.providerType === login.providerType
      : provider.id === login.providerId)
  }

  /** A running sign-in this page has not closed, with its provider, which the dialog opens when it shows none. */
  const running = computed(() => {
    for (const login of logins.value) {
      const provider = login.login.status === 'pending' && !dismissed.value.includes(login.id) ? providerOf(login) : undefined
      if (provider) {
        return { id: login.id, provider }
      }
    }
    return null
  })
  watch([running, () => opened.value === null, starts], () => {
    if (opened.value === null && starts.value === 0 && running.value) {
      opened.value = { ...running.value, failure: null, code: 'none', seen: true }
    }
  }, { immediate: true })
  watch(followed, (login, before) => {
    const shown = opened.value
    if (!shown || !login) {
      return
    }
    shown.seen = true
    const state = login.login
    // A code the sign-in refused after this page sent it: the dialog shows why and takes another.
    if (shown.code === 'sent' && state.status === 'pending' && state.codeError) {
      shown.code = 'none'
    }
    if (state.status === 'completed' && before?.login.status !== 'completed') {
      select(state.providerId)
    }
  })

  /** What the dialog shows of the opened sign-in. */
  function phaseOf(shown: NonNullable<typeof opened.value>): ProviderLoginPhase {
    if (shown.failure) {
      return { kind: 'failed', ...shown.failure }
    }
    const listed = followed.value
    if (!listed) {
      // Gone from the list while it ran, as when the server restarted.
      return shown.seen
        ? { kind: 'failed', message: 'This sign-in is no longer running.', requestId: null }
        : { kind: 'starting', stage: 'preparing' }
    }
    const state = listed.login
    switch (state.status) {
      case 'completed': {
        // The channel brings the entries before the sign-ins, so the account is listed by now.
        const added = resources.providers
          .find((entry) => entry.id === state.providerId)
          ?.accounts.find((account) => account.id === state.credentialId)
        return { kind: 'done', account: added?.label ?? 'Account connected', active: added?.active ?? false }
      }
      case 'failed':
        return { kind: 'failed', message: state.message, requestId: state.requestId }
      case 'pending':
        if (state.verificationUrl && state.needsCode) {
          const error = shown.code === 'none' ? state.codeError ?? undefined : undefined
          return {
            kind: 'code',
            url: state.verificationUrl,
            submitted: shown.code !== 'none',
            ...(error ? { error } : {}),
          }
        }
        if (state.verificationUrl && state.userCode) {
          return { kind: 'device', url: state.verificationUrl, code: state.userCode }
        }
        return { kind: 'starting', stage: state.stage }
    }
  }

  const login = computed(() => opened.value && {
    provider: opened.value.provider,
    phase: phaseOf(opened.value),
  })

  /**
   * Closes the dialog: a sign-in still running is cancelled, and the close
   * resolves once it has ended; a start still on its way is cancelled when
   * it answers.
   */
  async function closeLogin(): Promise<void> {
    const shown = opened.value
    opened.value = null
    const id = shown?.id
    if (!id) {
      return
    }
    dismissed.value.push(id)
    const listed = logins.value.find((login) => login.id === id)
    // An ended sign-in, or one gone, has nothing to cancel.
    if (listed ? listed.login.status !== 'pending' : shown.seen) {
      return
    }
    await cancelLogin(id)
  }

  async function cancelLogin(id: string): Promise<void> {
    await apiRequest(`/providers/subscription-login/${encodeURIComponent(id)}`, { method: 'DELETE' })
      .catch((error: unknown) => report('Could Not Cancel the Sign-In', error))
  }

  /**
   * Starts a sign-in into `entry`, after the one the dialog showed is
   * cancelled and has ended. A start the entry's running sign-in holds
   * opens that one instead.
   */
  async function beginLogin(entry: SettingsProviderEntry): Promise<void> {
    await closeLogin()
    const provider = providers.value.find(
      (provider) => provider.id === entry.id,
    )!
    opened.value = { provider, id: null, failure: null, code: 'none', seen: false }
    const shown = opened.value
    starts.value++
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
      if (opened.value !== shown) {
        dismissed.value.push(result.login.id)
        await cancelLogin(result.login.id)
        return
      }
      shown.id = result.login.id
    } catch (error) {
      if (opened.value !== shown) {
        return
      }
      if (error instanceof ApiError && error.code === 'provider_busy' && error.loginId) {
        shown.id = error.loginId
        return
      }
      shown.failure = shownError(error)
    } finally {
      starts.value--
    }
  }

  /** Hands the code the user pasted to the sign-in, which then finishes on its own. */
  async function submitCode(code: string): Promise<void> {
    const shown = opened.value
    const id = shown?.id
    if (!shown || !id || login.value?.phase.kind !== 'code') {
      return
    }
    shown.code = 'sending'
    try {
      await apiRequest(
        `/providers/subscription-login/${encodeURIComponent(id)}/code`,
        {
          method: 'POST',
          ...jsonBody({ code } satisfies LoginCode),
        },
      )
      shown.code = 'sent'
    } catch (error) {
      if (opened.value === shown) {
        shown.failure = shownError(error)
      }
    }
  }

  watch(
    () => useSession().user?.id,
    () => {
      lifetime.abort()
      lifetime = new AbortController()
      opened.value = null
      dismissed.value = []
      edits.value = {}
      modelEditor.value = null
      manualDrafts.value = {}
      testResults.value = {}
      refreshingUsage.value = {}
      usageFailures.value = {}
      quotaRefreshCache.clear()
    },
  )

  onScopeDispose(() => {
    lifetime.abort()
    quotaRefreshCache.clear()
  })

  return {
    modelEditor,
    refreshingUsage,
    usageFailures,
    providers,
    login,
    report,
    change,
    addProvider,
    addCompatible,
    removeProvider,
    beginLogin,
    closeLogin,
    test,
    refresh,
    removeAccount,
    activateAccount,
    saveModel,
    saveModels,
    searchModels,
    submitCode,
    refreshUsage,
    loadCli,
    checkCli,
    installCli,
  }
})

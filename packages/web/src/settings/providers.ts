import { t } from './locales/text'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { fileExtensionSchema } from '@demicodes/protocol'
import { SerialQueue } from '@demicodes/utils'
import { errorMessage, reportError, shownError, type ShownError } from '@demicodes/web-ui/infra/errors'
import { createQuotaRefreshCache } from '@demicodes/web-ui/settings/quota-refresh'
import type { ProviderLoginPhase } from '@demicodes/web-ui/settings/types'
import type { HeadlineText } from '@demicodes/web-ui/ui/ui-text'
import { providerLogo } from '@demicodes/web-ui/settings/vendor-logos'
import { canTest, testedModel } from '@demicodes/web-ui/settings/tested-model'
import { providerOperation, providerTarget } from '@demicodes/web-ui/settings/provider-operations'
import { operations } from '@demicodes/web-ui/ui/operations'
import {
  SETTINGS_MEDIA_KINDS,
  type SettingsModelDraft,
  type SettingsModelMatch,
  type SettingsQuotaRefresh,
  type SettingsModelEditor,
  type SettingsProviderCli,
  type SettingsProviderEntry,
  type SettingsProviderOperation,
  type SettingsProviderModel,
  type SettingsVendor,
  type SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import {
  codeTakenSchema,
  loginStartedSchema,
  modelSearchResultsSchema,
  providerAnswerSchema,
  providerCliSchema,
  testResultSchema,
  type AccountOrder,
  type ActivateAccount,
  type ConfiguredModel,
  type CreateProvider,
  type MediaPatch,
  type ProviderOrder,
  type ProviderPatch,
  type ListedLogin,
  type LoginState,
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
        notExposed?: { deviceId: string; device: string; port: number }
      }
    >
  >({})
  /** Each process provider's CLI, as last read (`crates/backend-user-shard/src/conversation/claude_cli.rs`). */
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
    }))
    // The signed-in subscriptions in the user's order, which generated
    // media follow, then each one nobody signed in to, in the vendors' order,
    // as what could be added follows what was (`media.md` § Which entry
    // makes a call).
    const placeholders = (product.vendors?.subscriptions ?? []).flatMap((subscription) =>
      configured.some(
        (provider) =>
          provider.kind === 'subscription' &&
          provider.providerType === subscription.providerType,
      )
        ? []
        : [emptyProvider(subscription.providerType, subscriptionName(subscription.providerType), 'subscription')],
    )
    const subscriptions = configured.filter((provider) => provider.kind === 'subscription')
    const keys = configured.filter((provider) => provider.kind !== 'subscription')
    return [...subscriptions, ...placeholders, ...keys]
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
        return t('providers.saveFailed')
      case 'testing':
        return t('providers.testFailed')
      case 'refreshing':
        return t('providers.refreshModelsFailed')
      case 'removing':
        return t('providers.removeFailed')
      case 'cli':
        return t('providers.cliUpdateFailed')
      case 'account':
        switch (operation.action) {
          case 'remove':
            return t('providers.removeAccountFailed')
          case 'test':
            return t('providers.testAccountFailed')
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
      media: { image: { shape: null, enabled: true }, video: { shape: null, enabled: true }, audio: { shape: null, enabled: true } },
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
   * lacks (`crates/backend-user-shard/src/provider_settings.rs`).
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
        report(t('providers.addFailed'), error)
      }
    })
  }

  function addProvider(vendor: SettingsVendor): void {
    create({ source: 'vendor', vendorId: vendor.id })
  }

  /** A compatible API: its kind alone, and the backend's defaults for the rest. */
  function addCompatible(): void {
    create({ source: 'compatible' })
  }

  /** A protocol as the backend names it: the family that speaks it, and its wire. */
  function protocol(wireApi: SettingsWireApi): Pick<ProviderPatch, 'providerType' | 'wireApi'> {
    switch (wireApi) {
      case 'anthropic-messages':
        return { providerType: 'anthropic' }
      case 'google-generative':
        return { providerType: 'google' }
      case 'openai-chat':
        return { providerType: 'openai', wireApi: 'chat-completions' }
      case 'openai-responses':
        return { providerType: 'openai', wireApi: 'responses' }
    }
  }

  function modelConfig(model: SettingsModelDraft): ConfiguredModel {
    return {
      ...(model.kind && model.kind !== 'text' ? { kind: model.kind } : {}),
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
    perform(provider.id, { kind: 'saving' }, async () => {
      try {
        await save(changes)
      } catch (error) {
        // A switch that did not save goes back to what the backend keeps,
        // as Activate does; a typed field keeps its text for the next try.
        const kept = { ...edits.value[provider.id] }
        delete kept.media
        edits.value[provider.id] = kept
        throw error
      }
      delete edits.value[provider.id]
    })

    /** What of each kind's shape and switch the user changed: a vendor entry's shape is never sent. */
    function mediaPatch(media: SettingsProviderEntry['media']): MediaPatch {
      return Object.fromEntries(SETTINGS_MEDIA_KINDS.flatMap((kind) => {
        const was = provider.media[kind]
        const now = media[kind]
        const change = {
          ...(now.shape !== was.shape ? { shape: now.shape } : {}),
          ...(now.enabled !== was.enabled ? { enabled: now.enabled } : {}),
        }
        return Object.keys(change).length ? [[kind, change]] : []
      }))
    }

    function save(changes: Partial<SettingsProviderEntry>): Promise<void> {
      return patch(provider, {
        ...(changes.name !== undefined ? { label: changes.name } : {}),
        // Only a compatible API's protocol and endpoint are edited; an
        // emptied endpoint or key is removed, as the API's null does.
        ...(changes.wireApi !== undefined ? protocol(changes.wireApi) : {}),
        ...(changes.baseUrl !== undefined ? { baseUrl: changes.baseUrl || null } : {}),
        ...(changes.apiKey !== undefined ? { apiKey: changes.apiKey || null } : {}),
        ...(changes.rotation !== undefined ? { rotation: changes.rotation } : {}),
        ...(changes.media !== undefined ? { media: mediaPatch(changes.media) } : {}),
        ...(changes.modelSource
          ? {
              models:
                changes.modelSource === 'catalog'
                  ? null
                  : provider.models.map(modelConfig),
            }
          : {}),
      })
    }
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
    // A model just added that the channel has not brought yet is the list's last.
    const models = original
      ? provider.models.some((entry) => entry.id === original.id)
        ? provider.models.map((entry) =>
            entry.id === original.id
              ? {
                  ...model,
                  enabled: entry.enabled,
                }
              : entry,
          )
        : [...provider.models, { ...model, enabled: original.enabled }]
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
   * where it lists them (`web-api-settings.md` § Model configuration and provider
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
   * Tests with the first text model the user keeps enabled, or for an entry
   * of media alone with its key alone; the page offers no test without one
   * of them. A subscription names the account to test, in use or not.
   */
  function test(provider: SettingsProviderEntry, accountId?: string): void {
    if (!canTest(provider)) {
      return
    }
    const model = provider.makesText === false ? undefined : testedModel(provider)
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
            ...jsonBody({ ...(model ? { modelId: model.id } : {}), ...(accountId ? { credentialId: accountId } : {}) } satisfies TestRequest),
          },
        )
        const result = await readResponse(response, testResultSchema)
        signal.throwIfAborted()
        testResults.value[provider.id] = result.type === 'passed'
          ? { state: 'ready', testPassed: true, testedWith: result.model }
          : { state: 'error', testPassed: false, detail: result.message, testedWith: result.model, notExposed: result.notExposed }
      } catch (error) {
        signal.throwIfAborted()
        testResults.value[provider.id] = {
          state: 'error',
          detail: errorMessage(error),
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
        reportError(t('providers.cliReadFailed'), error)
      }
    })
  }

  function checkCli(provider: SettingsProviderEntry): void {
    perform(provider.id, { kind: 'cli' }, async (signal) => {
      try {
        await readCli(provider.id, true, signal)
      } catch (error) {
        report(t('providers.checkUpdatesFailed'), error)
      }
    })
  }

  function installCli(provider: SettingsProviderEntry): Promise<void> {
    return perform(provider.id, { kind: 'cli' }, async (signal) => {
      try {
        await apiRequest(`/providers/${encodeURIComponent(provider.id)}/cli/install`, { method: 'POST', signal })
        await readCli(provider.id, false, lifetime.signal)
      } catch (error) {
        report(t('providers.cliInstallFailed'), error)
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
        // The vendor's own words, as they came (`localization.md` § Text from the backend).
        usageFailures.value[provider.id]![accountId] = error instanceof ApiError && error.code === 'quota_unavailable'
          ? { message: error.message, requestId: error.requestId }
          : shownError(error)
        // The account's line shows it, wherever its windows show; no toast.
        reportError(t('providers.usageFailed'), error)
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
   * Puts the provider's accounts in the order the user dragged them to: the
   * list shows it at once, the change is sent after the provider's earlier
   * writes, and a refusal puts the list back with a toast.
   */
  function reorderAccounts(provider: SettingsProviderEntry, ids: string[]): Promise<void> {
    const shown = product.change('providers', (state) => ({
      ...state,
      providers: state.providers.map((entry) =>
        entry.id === provider.id && entry.details.type === 'read'
          ? {
              ...entry,
              details: {
                ...entry.details,
                accounts: ids.flatMap((id) => {
                  const account = entry.details.type === 'read'
                    ? entry.details.accounts.find((candidate) => candidate.id === id)
                    : undefined
                  return account ? [account] : []
                }),
              },
            }
          : entry,
      ),
    }))
    const current = lifetime
    return writes
      .run(provider.id, async () => {
        current.signal.throwIfAborted()
        shown.send()
        await apiRequest(`/providers/${encodeURIComponent(provider.id)}/accounts/order`, {
          method: 'PUT',
          signal: current.signal,
          ...jsonBody({ credentialIds: ids } satisfies AccountOrder),
        })
        shown.land()
      })
      .catch((error) => {
        shown.drop()
        report(t('providers.reorderFailed'), error)
      })
  }

  /**
   * Puts the entries in the order the user dragged the list to, which
   * generated media follow (`media.md` § Which entry makes a call): the list
   * shows it at once, the change is sent after every entry's earlier
   * writes, and a refusal puts the list back with a toast.
   */
  function reorderProviders(ids: string[]): Promise<void> {
    const shown = product.change('providers', (state) => ({
      ...state,
      providers: ids.flatMap((id) => state.providers.filter((entry) => entry.id === id)),
    }))
    const current = lifetime
    return writes
      // After every entry's earlier writes, which the order must not overtake.
      .run(ids, async () => {
        current.signal.throwIfAborted()
        shown.send()
        await apiRequest('/providers/order', {
          method: 'PUT',
          signal: current.signal,
          ...jsonBody({ providerIds: ids } satisfies ProviderOrder),
        })
        shown.land()
      })
      .catch((error) => {
        shown.drop()
        report(t('providers.reorderProvidersFailed'), error)
      })
  }

  /**
   * Makes `id` the provider's active account as the user presses Activate:
   * the Active mark moves at once (it is a switch, `web-application.md`
   * § Responding to the user), the change is sent after the provider's
   * earlier writes, and a refusal moves the mark back with a toast. Answers
   * whether the account became active.
   */
  function activateAccount(provider: SettingsProviderEntry, id: string): Promise<boolean> {
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
        return true
      })
      .catch((error) => {
        shown.drop()
        report(t('providers.switchFailed'), error)
        return false
      })
  }

  /**
   * The sign-in the dialog shows (`crates/backend-providers/src/vault/logins.rs`):
   * one this page started, whose `id` is null until the start answers, the
   * one a busy start named, or a running one the vault announced, as after
   * a reload. The vault announces each change of it on the channel, which
   * the dialog follows; `failure` is why this page could not start or
   * answer it, `pasted` the code this page pasted last, `sending` until
   * the sign-in answers the number it gave it, `seen` whether the channel
   * has listed it, and `done` the account it added, as listed when the
   * page learned it completed.
   */
  const opened = ref<{
    provider: ProductProvider
    id: string | null
    failure: ShownError | null
    pasted: 'sending' | number | null
    seen: boolean
    done: { account: string; active: boolean } | null
  } | null>(null)
  /** The sign-ins this page closed, which it never opens again on its own. */
  const dismissed = ref<string[]>([])
  /** The starts on their way: the sign-in one makes is this page's, and not opened as another's. */
  const starts = ref(0)
  type PendingLogin = Extract<LoginState, { status: 'pending' }>
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
      opened.value = { ...running.value, failure: null, pasted: null, seen: true, done: null }
    }
  }, { immediate: true })
  /**
   * The account a completed sign-in added. The page has it by the time it
   * learns the sign-in completed (`web-api-settings.md` § Subscription accounts);
   * its id stands for it only when it was removed before then.
   */
  function addedBy(state: Extract<LoginState, { status: 'completed' }>): { account: string; active: boolean } {
    const account = resources.providers
      .find((entry) => entry.id === state.providerId)
      ?.accounts.find((candidate) => candidate.id === state.credentialId)
    return { account: account?.label ?? state.credentialId, active: account?.active ?? false }
  }
  watch(followed, (login) => {
    const shown = opened.value
    if (!shown || !login) {
      return
    }
    shown.seen = true
    const state = login.login
    // Done from the moment the page learns it, which an account removed
    // later does not undo; the dialog selects the entry once.
    if (state.status === 'completed' && shown.done === null) {
      shown.done = addedBy(state)
      select(state.providerId)
    }
    // A sign-in cancelled on any page closes its dialog on every page.
    if (state.status === 'cancelled') {
      dismissed.value.push(login.id)
      opened.value = null
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
        ? { kind: 'failed', message: t('providers.loginStopped'), requestId: null }
        : { kind: 'starting', stage: 'preparing' }
    }
    const state = listed.login
    switch (state.status) {
      case 'completed':
        return { kind: 'done', ...(shown.done ?? addedBy(state)) }
      case 'failed':
        return { kind: 'failed', message: state.message, requestId: state.requestId }
      // The dialog closes; until it does, nothing new.
      case 'cancelled':
        return { kind: 'starting', stage: 'preparing' }
      case 'pending':
        return pendingPhase(shown, state)
    }
  }

  /** What the dialog shows of a sign-in that waits as `state` says. */
  function pendingPhase(shown: NonNullable<typeof opened.value>, state: PendingLogin): ProviderLoginPhase {
    // No expiry while the sign-in prepares: it does not know its lifetime yet.
    const expiry = state.expiresAt === null ? {} : { expiresAt: Date.parse(state.expiresAt) }
    if (state.verificationUrl && state.needsCode) {
      // A refusal shows only of the code this page pasted, by the number
      // the sign-in answered for it: the vault's refusal is always of code
      // number `codesTaken`.
      const mine = typeof shown.pasted === 'number' ? shown.pasted : null
      const error = mine !== null && state.codesTaken === mine ? state.codeError ?? undefined : undefined
      const judging = shown.pasted === 'sending' || (mine !== null && state.codesTaken < mine) || (state.codesTaken > 0 && !state.codeError)
      return {
        kind: 'code',
        url: state.verificationUrl,
        ...expiry,
        submitted: judging,
        ...(mine !== null ? { taken: mine } : {}),
        ...(error ? { error } : {}),
      }
    }
    if (state.verificationUrl && state.userCode) {
      return { kind: 'device', url: state.verificationUrl, code: state.userCode, ...expiry }
    }
    return { kind: 'starting', stage: state.stage }
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
      .catch((error: unknown) => report(t('providers.cancelLoginFailed'), error))
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
    opened.value = { provider, id: null, failure: null, pasted: null, seen: false, done: null }
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
    shown.pasted = 'sending'
    try {
      const response = await apiRequest(
        `/providers/subscription-login/${encodeURIComponent(id)}/code`,
        {
          method: 'POST',
          ...jsonBody({ code } satisfies LoginCode),
        },
      )
      const taken = await readResponse(response, codeTakenSchema)
      if (shown.pasted === 'sending') {
        shown.pasted = taken.code
      }
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
    reorderAccounts,
    reorderProviders,
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

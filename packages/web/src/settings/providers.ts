import { readConfiguredModelDraft } from '@demicodes/web-ui/settings/model-draft'
import { computed, onScopeDispose, ref, watch } from 'vue'
import { defineStore } from 'pinia'
import { useSession } from '../auth/session'
import { z } from 'zod'
import { SerialQueue } from '@demicodes/utils'
import { reportError } from '@demicodes/web-ui/infra/errors'
import type { ProviderLoginPhase } from '@demicodes/web-ui/settings/types'
import {
  defaultApiVendors,
  defaultEndpointUrl,
} from '@demicodes/web-ui/settings/provider-defaults'
import {
  WIRE_API_LABELS,
  type SettingsModelDraft,
  type SettingsModelEditor,
  type SettingsProviderEntry,
  type SettingsProviderOperation,
  type SettingsProviderModel,
  type SettingsVendor,
  type SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import { providerSchema } from '@demicodes/product-contracts'
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
      }
    >
  >({})
  let lifetime = new AbortController()
  const writes = new SerialQueue()
  const providers = computed(() => {
    const configured = resources.providers.map((provider) => ({
      ...provider,
      ...testResults.value[provider.id],
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
    const result = readConfiguredModelDraft(model)
    if (!result.success) {
      throw result.error
    }
    return result.data
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
    await product.revalidate()
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
    await product.revalidate()
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
      await product.revalidate()
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

  function test(provider: SettingsProviderEntry): void {
    perform(provider.id, { kind: 'testing' }, async (signal) => {
      try {
        const response = await apiRequest(
          `/providers/${encodeURIComponent(provider.id)}/test`,
          {
            method: 'POST',
            signal,
          },
        )
        const result = await readResponse(
          response,
          z.object({
            ok: z.boolean(),
            message: z.string().optional(),
          }),
        )
        signal.throwIfAborted()
        testResults.value[provider.id] = {
          state: result.ok ? 'ready' : 'error',
          testPassed: result.ok,
          detail: result.message,
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

  function refresh(provider: SettingsProviderEntry): void {
    perform(provider.id, { kind: 'refreshing' }, async (signal) => {
      await product.loadModels(undefined, true)
      signal.throwIfAborted()
      const capability = product.snapshot?.providers.find(
        (entry) => entry.id === provider.id,
      )?.details?.quotaCapability
      if (capability?.canProbe && capability.probeCost === 'free') {
        await apiRequest(
          `/providers/${encodeURIComponent(provider.id)}/quota`,
          {
            method: 'POST',
            signal,
          },
        )
      }
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
        await product.revalidate()
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
        await product.revalidate()
        controller.signal.throwIfAborted()
        const provider = resources.providers.find(
          (entry) => entry.id === result.providerId,
        )
        login.value.phase = {
          kind: 'done',
          account:
            provider?.accounts.find((account) => account.active)?.label ??
            'Account connected',
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
      await product.revalidate()
      controller.signal.throwIfAborted()
      current.phase = {
        kind: 'done',
        account: 'Account connected',
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
    },
  )

  onScopeDispose(() => {
    lifetime.abort()
    cancelLogin()
  })

  return {
    modelEditor,
    operations,
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
  }
})

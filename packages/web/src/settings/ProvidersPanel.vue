<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { z } from 'zod'
import { SerialQueue } from '@demicodes/utils'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import { showToast } from '@demicodes/web-ui/infra/toast'
import ProviderLoginDialog, {
  type ProviderLoginPhase,
} from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import {
  defaultApiVendors,
  defaultEndpointUrl,
} from '@demicodes/web-ui/settings/provider-defaults'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import {
  WIRE_API_LABELS,
  type SettingsModelDraft,
  type SettingsProviderEntry,
  type SettingsProviderModel,
  type SettingsVendor,
  type SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import { apiRequest, jsonBody, readResponse } from '../api/client'
import { providerSchema } from '../api/contracts'
import { useResources } from '../state/resources'
import { useProduct } from '../state/product'
import { subscriptionName, type ProductProvider } from '../state/catalog'

const resources = useResources()
const product = useProduct()
const drafts = ref<ProductProvider[]>([])
const manualDrafts = ref<Record<string, SettingsProviderModel[]>>({})
const testing = ref<string | null>(null)
const refreshing = ref<string | null>(null)
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
const lifetime = new AbortController()
const writes = new SerialQueue()
const providers = computed(() => {
  const configured = resources.providers.map((provider) => ({
    ...provider,
    ...testResults.value[provider.id],
    ...(manualDrafts.value[provider.id]
      ? {
          modelSource: 'manual' as const,
          models: manualDrafts.value[provider.id],
        }
      : {}),
  }))
  const missing = (product.vendors?.subscriptions ?? [])
    .filter((subscription) => !subscription.configured)
    .map((subscription) =>
      emptyProvider(
        subscription.providerType,
        subscriptionName(subscription.providerType),
        'subscription',
      ),
    )
  return [...configured, ...missing, ...drafts.value]
})

function report(error: unknown): void {
  if (!lifetime.signal.aborted) {
    showToast({
      title: 'Provider operation failed',
      message: error instanceof Error ? error.message : String(error),
      tone: 'danger',
    })
  }
}

function perform(action: () => Promise<void>): void {
  void writes
    .run(async () => {
      lifetime.signal.throwIfAborted()
      await action()
    })
    .catch(report)
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
  () => product.vendors,
  () => {
    const defaults = defaultApiVendors(
      resources.vendors,
      [...resources.providers, ...drafts.value],
    )
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
      model.extensions?.map((extension) => extension.replace(/^\./, '')) ?? null,
    fastTier: model.fastTier,
  }
}

async function saveDraft(provider: ProductProvider): Promise<void> {
  if (
    !provider.apiKey ||
    (provider.modelSource === 'manual' && !provider.models.length)
  ) {
    return
  }
  const family =
    provider.wireApi === 'anthropic-messages'
      ? 'anthropic'
      : provider.wireApi === 'google-generative'
        ? 'google'
        : 'openai'
  const response = await apiRequest('/providers', {
    method: 'POST',
    signal: lifetime.signal,
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
  const saved = await readResponse(response, z.object({ provider: providerSchema }))
  provider.apiKey = ''
  resources.hideProvider(saved.provider.id, provider.enabled)
  await product.revalidate()
  select(saved.provider.id)
  drafts.value = drafts.value.filter((draft) => draft.id !== provider.id)
}

async function patch(provider: SettingsProviderEntry, body: unknown): Promise<void> {
  await apiRequest(`/providers/${encodeURIComponent(provider.id)}`, {
    method: 'PATCH',
    signal: lifetime.signal,
    ...jsonBody(body),
  })
  delete testResults.value[provider.id]
  await product.revalidate()
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
  perform(async () => {
    const draft = drafts.value.find((entry) => entry.id === provider.id)
    if (draft) {
      Object.assign(draft, changes)
      await saveDraft(draft)
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
      ...(changes.baseUrl !== undefined ? { baseUrl: changes.baseUrl || null } : {}),
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
  })
}

function removeProvider(id: string): void {
  perform(async () => {
    if (drafts.value.some((provider) => provider.id === id)) {
      drafts.value = drafts.value.filter((provider) => provider.id !== id)
      return
    }
    await apiRequest(`/providers/${encodeURIComponent(id)}`, {
      method: 'DELETE',
      signal: lifetime.signal,
    })
    await product.revalidate()
  })
}

async function saveModels(
  provider: SettingsProviderEntry,
  models: SettingsProviderModel[],
): Promise<void> {
  await writes.run(async () => {
    lifetime.signal.throwIfAborted()
    const draft = drafts.value.find((entry) => entry.id === provider.id)
    if (draft) {
      draft.models = models
      await saveDraft(draft)
      return
    }
    await patch(provider, { models: models.map(modelConfig) })
    delete manualDrafts.value[provider.id]
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
  await saveModels(provider, models)
}

function test(provider: SettingsProviderEntry): void {
  perform(async () => {
    testing.value = provider.id
    try {
      const response = await apiRequest(
        `/providers/${encodeURIComponent(provider.id)}/test`,
        {
          method: 'POST',
          signal: lifetime.signal,
        },
      )
      const result = await readResponse(
        response,
        z.object({
          ok: z.boolean(),
          message: z.string().optional(),
        }),
      )
      testResults.value[provider.id] = {
        state: result.ok ? 'ready' : 'error',
        testPassed: result.ok,
        detail: result.message,
      }
    } catch (error) {
      testResults.value[provider.id] = {
        state: 'error',
        detail: error instanceof Error ? error.message : String(error),
      }
      throw error
    } finally {
      testing.value = null
    }
  })
}

function refresh(provider: SettingsProviderEntry): void {
  perform(async () => {
    refreshing.value = provider.id
    try {
      await product.loadModels(null, true)
      const capability = product.snapshot?.providers.find(
        (entry) => entry.id === provider.id,
      )?.details?.quotaCapability
      if (capability?.canProbe && capability.probeCost === 'free') {
        await apiRequest(`/providers/${encodeURIComponent(provider.id)}/quota`, {
          method: 'POST',
          signal: lifetime.signal,
        })
      }
      await product.refresh()
    } finally {
      refreshing.value = null
    }
  })
}

function accountAction(
  provider: SettingsProviderEntry,
  id: string,
  action: 'activate' | 'remove',
): void {
  perform(async () => {
    const base = `/providers/${encodeURIComponent(provider.id)}/accounts`
    await apiRequest(
      action === 'activate' ? `${base}/active` : `${base}/${encodeURIComponent(id)}`,
      {
        method: action === 'activate' ? 'PUT' : 'DELETE',
        signal: lifetime.signal,
        ...(action === 'activate' ? jsonBody({ credentialId: id }) : {}),
      },
    )
    await product.revalidate()
  })
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
    void apiRequest(`/providers/subscription-login/${encodeURIComponent(id)}`, {
      method: 'DELETE',
    }).catch(report)
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
  const provider = providers.value.find((provider) => provider.id === entry.id)!
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

function openUrl(url: string): void {
  window.open(url, '_blank', 'noopener,noreferrer')
}

onMounted(() => void product.loadVendors().catch(report))
onUnmounted(() => {
  cancelLogin()
  lifetime.abort()
  drafts.value = []
})
</script>

<template>
  <SettingsProvidersPage
    v-model:selected-id="resources.selectedProviderId"
    v-model:detail-open="resources.providerDetailOpen"
    :providers="providers"
    :vendors="resources.vendors"
    :overlay-store="appOverlayStore"
    :testing="testing"
    :refreshing="refreshing"
    @change="change"
    @toggle-model="
      (provider, model, enabled) =>
        resources.hideModel(provider.id, model.id, enabled)
    "
    @add="addProvider"
    @add-endpoint="addEndpoint"
    @remove="removeProvider"
    @sign-in="beginLogin"
    @test="test"
    @refresh="refresh"
    @activate-account="(provider, id) => accountAction(provider, id, 'activate')"
    @remove-account="(provider, id) => accountAction(provider, id, 'remove')"
    :save-model="saveModel"
    @remove-model="
      (provider, model) =>
        saveModels(
          provider,
          provider.models.filter((entry) => entry.id !== model.id),
        ).catch(report)
    "
  />
  <ProviderLoginDialog
    v-if="login"
    :is-open="true"
    :overlay-store="appOverlayStore"
    :vendor-name="login.provider.name"
    :phase="login.phase"
    @close="closeLogin"
    @open="openUrl"
    @retry="beginLogin(login.provider)"
    @submit-token="submitToken"
  />
</template>

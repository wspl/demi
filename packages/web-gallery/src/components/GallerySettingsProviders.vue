<script setup lang="ts">
import { computed, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ProviderLoginDialog, {
  type ProviderLoginPhase,
} from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import { defaultApiVendors } from '@demicodes/web-ui/settings/provider-defaults'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import {
  WIRE_API_LABELS,
  type SettingsModelDraft,
  type SettingsProviderEntry,
  type SettingsProviderModel,
  type SettingsVendor,
  type SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import {
  mockVendors,
  provider,
  type MockProvider,
  type SettingsState,
} from '../fixtures/settings'

/**
 * The shared providers page over the mock state. Everything the page emits is
 * answered here the way a host would, with timers standing in for the network.
 */
const props = defineProps<{
  state: SettingsState
}>()

const s = computed(() => props.state)
const testing = ref<string | null>(null)

for (const vendor of defaultApiVendors(mockVendors, s.value.providers)) {
  s.value.providers.push(
    provider({
      id: `default-${vendor.id}`,
      name: `${vendor.name} API`,
      kind: 'api_key',
      family: vendor.id,
      vendorId: vendor.id,
      baseUrl: vendor.baseUrl ?? '',
      wireApi: vendor.wireApi,
      logo: vendor.logo,
      configured: false,
      state: 'unconfigured',
    }),
  )
}

function changeProvider(
  entry: SettingsProviderEntry,
  patch: Partial<SettingsProviderEntry>,
): void {
  Object.assign(entry, patch)
  if (patch.apiKey) {
    entry.configured = true
    entry.keyConfigured = true
  }
}

function select(id: string) {
  s.value.selectedProviderId = id
  s.value.providerDetailOpen = true
}

function addProvider(vendor: SettingsVendor) {
  const id = `p-${Date.now()}`
  s.value.providers.push(
    provider({
      id,
      name: vendor.name,
      kind: 'api_key',
      family: vendor.id,
      vendorId: vendor.id,
      baseUrl: vendor.baseUrl ?? '',
      wireApi: vendor.wireApi,
      modelSource: 'catalog',
      catalogFetched: 'just now',
      logo: vendor.logo,
      state: 'unconfigured',
    }),
  )
  select(id)
}

function addEndpoint(wireApi: SettingsWireApi) {
  const id = `p-${Date.now()}`
  s.value.providers.push(
    provider({
      id,
      name: `${WIRE_API_LABELS[wireApi]} API`,
      kind: 'api_key',
      family: 'custom',
      vendorId: null,
      wireApi,
      modelSource: 'manual',
      state: 'unconfigured',
    }),
  )
  select(id)
}

function removeProvider(id: string) {
  const list = s.value.providers
  const index = list.findIndex((p) => p.id === id)
  if (index < 0) {
    return
  }
  list.splice(index, 1)
  if (s.value.selectedProviderId === id) {
    const next = list.find((p) => p.kind === 'api_key') ?? list[0]
    s.value.selectedProviderId = next?.id ?? null
  }
}

/** The mock judges a connection by its inputs: no key is rejected, localhost is unreachable. */
function test(p: SettingsProviderEntry) {
  testing.value = p.id
  window.setTimeout(() => {
    testing.value = null
    if (p.vendorId && !p.apiKey) {
      p.state = 'error'
      p.detail = '401 · No API key provided'
    } else if (/localhost|127\.0\.0\.1/.test(p.baseUrl)) {
      p.state = 'unreachable'
      p.detail = `connect ECONNREFUSED ${p.baseUrl.replace(/^https?:\/\//, '').replace(/\/.*$/, '')}`
    } else {
      p.state = 'ready'
      p.detail = undefined
      p.testedIn = `${180 + Math.round(Math.random() * 400)} ms`
    }
  }, 1200)
}

const refreshing = ref<string | null>(null)
function refresh(p: SettingsProviderEntry) {
  refreshing.value = p.id
  window.setTimeout(() => {
    refreshing.value = null
    p.catalogFetched = 'just now'
    p.stale = false
  }, 1400)
}

function activateAccount(p: SettingsProviderEntry, id: string) {
  for (const a of p.accounts) {
    a.active = a.id === id
  }
}

function removeAccount(p: SettingsProviderEntry, id: string) {
  p.accounts = p.accounts.filter((a) => a.id !== id)
  if (!p.accounts.length) {
    p.state = 'signed-out'
  }
}

async function saveModel(
  p: SettingsProviderEntry,
  draft: SettingsModelDraft,
  original: SettingsProviderModel | null,
) {
  const mock = p as MockProvider
  if (original) {
    Object.assign(original, {
      name: draft.name,
      contextWindow: draft.contextWindow,
      outputLimit: draft.outputLimit,
      efforts: draft.efforts,
      extensions: draft.extensions,
      fastTier: draft.fastTier,
    })
  } else if (!mock.models.some((m) => m.id === draft.id)) {
    mock.models.push({
      ...draft,
      tools: true,
      defaultEffort: draft.efforts[0] ?? null,
      enabled: true,
    })
  }
}

function removeModel(p: SettingsProviderEntry, m: SettingsProviderModel) {
  p.models = p.models.filter((x) => x.id !== m.id)
}

// Sign-in runs in its own dialog; the mock walks the device-code flow to completion.
const login = ref<{
  provider: SettingsProviderEntry
  phase: ProviderLoginPhase
} | null>(null)
const loginOpen = ref(false)
let loginTimer = 0

/**
 * Claude Code hands out a token from its own CLI; Codex confirms a device code in the
 * browser; Grok Build shows a code the user copies back. The mock walks each to done.
 */
function beginLogin(p: SettingsProviderEntry) {
  window.clearTimeout(loginTimer)
  loginOpen.value = true
  const mock = p as MockProvider
  if (mock.family === 'claude-code') {
    login.value = {
      provider: p,
      phase: {
        kind: 'token',
        command: 'claude setup-token',
        install: {
          label: 'Get Claude Code',
          url: 'https://docs.anthropic.com/en/docs/claude-code/setup',
        },
        prefix: 'sk-ant-oat01-',
      },
    }
    return
  }
  login.value = {
    provider: p,
    phase: { kind: 'starting' },
  }
  loginTimer = window.setTimeout(() => {
    if (!login.value) {
      return
    }
    if (mock.family === 'grok-build') {
      login.value.phase = {
        kind: 'code-input',
        url: 'https://accounts.x.ai/device',
      }
      return
    }
    login.value.phase = {
      kind: 'device',
      url: 'https://auth.openai.com/codex/device',
      code: 'HXRV-7K2M',
      expiresIn: '10 min',
    }
    loginTimer = window.setTimeout(() => {
      if (!login.value) {
        return
      }
      login.value.phase = {
        kind: 'done',
        account: 'zan@example.com · Plus',
      }
    }, 4000)
  }, 900)
}

function finishLogin(account: string) {
  if (!login.value) {
    return
  }
  window.clearTimeout(loginTimer)
  login.value.phase = {
    kind: 'done',
    account,
  }
}

function openUrl(url: string) {
  window.open(url, '_blank', 'noopener')
}

function closeLogin() {
  window.clearTimeout(loginTimer)
  if (login.value?.phase.kind === 'done') {
    const p = login.value.provider
    for (const a of p.accounts) {
      a.active = false
    }
    p.accounts.push({
      id: `a-${Date.now()}`,
      label: 'zan@example.com',
      plan: 'Plus',
      active: true,
      quota: [
        {
          id: 'hour',
          label: '5-hour',
          used: 4,
          max: 100,
          resets: 'in 4 h 58 min',
        },
        {
          id: 'week',
          label: 'Weekly',
          used: 4,
          max: 100,
          resets: 'Monday',
        },
      ],
    })
    p.state = 'ready'
  }
  loginOpen.value = false
}
</script>

<template>
  <SettingsProvidersPage
    v-model:selected-id="s.selectedProviderId"
    v-model:detail-open="s.providerDetailOpen"
    :providers="s.providers"
    :vendors="mockVendors"
    :overlay-store="appOverlayStore"
    :testing="testing"
    :refreshing="refreshing"
    @change="changeProvider"
    @toggle-model="(_provider, model, enabled) => (model.enabled = enabled)"
    @add="addProvider"
    @add-endpoint="addEndpoint"
    @remove="removeProvider"
    @sign-in="beginLogin"
    @test="test"
    @refresh="refresh"
    @activate-account="activateAccount"
    @remove-account="removeAccount"
    :save-model="saveModel"
    @remove-model="removeModel"
  />
  <ProviderLoginDialog
    v-if="login"
    :is-open="loginOpen"
    :overlay-store="appOverlayStore"
    :vendor-name="login.provider.name"
    :phase="login.phase"
    @close="closeLogin"
    @open="openUrl"
    @retry="beginLogin(login.provider)"
    @submit-code="finishLogin('zan@example.com · SuperGrok')"
    @submit-token="finishLogin('zan@example.com · Max 5×')"
  />
</template>

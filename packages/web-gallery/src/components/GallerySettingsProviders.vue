<script setup lang="ts">
import { requestFailure } from '../fixtures/request-failures'
import type { ShownError } from '@demicodes/web-ui/infra/errors'
import { computed, onScopeDispose, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ProviderLoginDialog, {
  type ProviderLoginPhase,
} from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import {
  WIRE_API_LABELS,
  type SettingsModelDraft,
  type SettingsQuotaRefresh,
  type SettingsProviderEntry,
  type SettingsProviderOperation,
  type SettingsProviderModel,
  type SettingsVendor,
  type SettingsWireApi,
} from '@demicodes/web-ui/settings/types'
import {
  missingParts,
  mockVendors,
  provider,
  searchMockModels,
  type MockProvider,
  type SettingsState,
} from '../fixtures/settings'
import { DAY, HOUR, MINUTE, ahead } from '../fixtures/time'

/**
 * The shared providers page over the mock state. Everything the page emits is
 * answered here the way a host would, with timers standing in for the network.
 */
const props = defineProps<{
  state: SettingsState
}>()

const s = computed(() => props.state)
const testing = ref<string | null>(null)

/**
 * As the product's page shows an entry from what it holds: it is incomplete,
 * with the yellow dot and its fields red, until the last part is saved.
 */
function settle(entry: SettingsProviderEntry): void {
  const missing = missingParts(entry)
  if (missing.length) {
    entry.state = 'setup'
    entry.missing = missing
  } else if (entry.state === 'setup') {
    entry.state = 'ready'
    delete entry.missing
  }
}

function changeProvider(
  entry: SettingsProviderEntry,
  patch: Partial<SettingsProviderEntry>,
): void {
  Object.assign(entry, patch)
  if (patch.apiKey) {
    // As in the product, the page never gets a stored key back: only that one exists.
    entry.apiKey = ''
    entry.configured = true
    entry.keyConfigured = true
  }
  settle(entry)
}

function select(id: string) {
  s.value.selectedProviderId = id
  s.value.providerDetailOpen = true
}

/** Adding asks only the kind: the entry joins the list at once, selected, and says what it lacks. */
function addProvider(vendor: SettingsVendor) {
  const id = `p-${Date.now()}`
  join(provider({
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
  }))
}

function addEndpoint(wireApi: SettingsWireApi) {
  const id = `p-${Date.now()}`
  join(provider({
    id,
    name: `${WIRE_API_LABELS[wireApi]} API`,
    kind: 'api_key',
    family: 'custom',
    vendorId: null,
    wireApi,
    baseUrl: '',
    modelSource: 'manual',
  }))
}

function join(entry: MockProvider) {
  settle(entry)
  s.value.providers.push(entry)
  select(entry.id)
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
function test(p: SettingsProviderEntry, accountId?: string) {
  if (accountId) {
    accountOperation.value = { providerId: p.id, accountId, action: 'test' }
  } else {
    testing.value = p.id
  }
  window.setTimeout(() => {
    testing.value = null
    accountOperation.value = null
    if (p.vendorId && !p.apiKey && !p.keyConfigured) {
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
  }, 1400)
}

// The product asks the vendor about one account, in use or not; here that account's
// windows move a little, as a fresh answer would.
const accountOperation = ref<{
  providerId: string
  accountId: string
  action: 'test'
} | null>(null)
const cliChecking = ref<string | null>(null)
const operations = computed<Record<string, SettingsProviderOperation>>(() =>
  cliChecking.value
    ? { [cliChecking.value]: { kind: 'cli' } }
    : accountOperation.value
    ? {
        [accountOperation.value.providerId]: {
          kind: 'account',
          accountId: accountOperation.value.accountId,
          action: accountOperation.value.action,
        },
      }
    : {},
)
const usageTimers = ref<Record<string, Record<string, { timer: number; trigger: SettingsQuotaRefresh }>>>({})
/** The vendor's words for a manual refresh that failed, by provider and account, until that account's next answer. */
const usageFailures = ref<Record<string, Record<string, ShownError>>>({})
/** Every other request a manual press asked for fails, so the page shows both outcomes; an automatic one always answers. */
let manualRefreshes = 0
const refreshingUsage = computed(() => Object.fromEntries(
  Object.entries(usageTimers.value).map(([id, accounts]) => [id, Object.fromEntries(
    Object.entries(accounts).map(([accountId, request]) => [accountId, request.trigger]),
  )]),
))
function refreshUsage(p: SettingsProviderEntry, accountId: string, automatic = false) {
  if (automatic && s.value.quotaRefreshCache.isFresh(p.id, accountId)) {
    return
  }
  usageTimers.value[p.id] ??= {}
  const timers = usageTimers.value[p.id]!
  const pending = timers[accountId]
  if (pending) {
    if (!automatic) {
      pending.trigger = 'manual'
    }
    return
  }
  timers[accountId] = {
    trigger: automatic ? 'automatic' : 'manual',
    timer: window.setTimeout(() => {
      s.value.quotaRefreshCache.record(p.id, accountId)
      // A manual press that joined an automatic request shares its outcome, as in the product.
      const fails = timers[accountId]?.trigger === 'manual' && ++manualRefreshes % 2 === 1
      delete timers[accountId]
      if (!Object.keys(timers).length) {
        delete usageTimers.value[p.id]
      }
      if (fails) {
        usageFailures.value[p.id] = { ...usageFailures.value[p.id], [accountId]: requestFailure('Anthropic’s usage service is unavailable.') }
        return
      }
      delete usageFailures.value[p.id]?.[accountId]
      for (const window of p.accounts.find((account) => account.id === accountId)?.quota ?? []) {
        window.used = Math.min(window.max, window.used + 1)
      }
    }, 1200),
  }
}
onScopeDispose(() => {
  for (const timer of cliInstalls) {
    window.clearTimeout(timer)
  }
  for (const accounts of Object.values(usageTimers.value)) {
    for (const request of Object.values(accounts)) {
      window.clearTimeout(request.timer)
    }
  }
})

// The product asks the vendor and the machines; here each action shows its own outcome.
function checkCli(p: SettingsProviderEntry) {
  if (!p.cli) {
    return
  }
  cliChecking.value = p.id
  window.setTimeout(() => {
    cliChecking.value = null
    if (p.cli) {
      p.cli.newest = { version: '2.1.279' }
    }
  }, 1200)
}

/** How long the Cloud takes to wake, read the release and install the CLI. */
const CLI_INSTALL_MS = 3000
/** The specimen's CLI installs under way, which end when the page goes. */
const cliInstalls = new Set<number>()

/** Installing on Cloud while the Cloud wakes and downloads, then installed. */
function installCli(p: SettingsProviderEntry) {
  const cli = p.cli
  if (!cli || cli.install?.state === 'installing') {
    return
  }
  cli.install = { state: 'installing' }
  const timer = window.setTimeout(() => {
    cliInstalls.delete(timer)
    cli.install = { state: 'installed' }
    if ('version' in cli.newest) {
      cli.machines[0]!.versions = [cli.newest.version]
    }
  }, CLI_INSTALL_MS)
  cliInstalls.add(timer)
}

function activateAccount(p: SettingsProviderEntry, id: string) {
  for (const a of p.accounts) {
    a.active = a.id === id
  }
}

/** As the backend does: the active account's removal makes the first remaining one active, or leaves none. */
function removeAccount(p: SettingsProviderEntry, id: string) {
  const wasActive = p.accounts.some((a) => a.id === id && a.active)
  p.accounts = p.accounts.filter((a) => a.id !== id)
  if (wasActive && p.accounts[0]) {
    p.accounts[0].active = true
  }
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
  } else {
    if (mock.models.some((m) => m.id === draft.id && mock.modelSource === 'manual')) {
      throw new Error(`The model ${draft.id} is already in the list.`)
    }
    // An entry that follows its vendor's list starts a list of its own from it.
    mock.modelSource = 'manual'
    mock.models = mock.models.filter((m) => m.id !== draft.id)
    mock.models.push({
      ...draft,
      tools: true,
      enabled: true,
    })
  }
  settle(p)
}

function removeModel(p: SettingsProviderEntry, m: SettingsProviderModel) {
  p.models = p.models.filter((x) => x.id !== m.id)
  settle(p)
}

/** The product asks the backend, which fills the match from the entry's vendor; here the fixtures answer. */
function searchModels(_providerId: string, query: string, signal: AbortSignal) {
  return searchMockModels(query, signal)
}

// Sign-in runs in its own dialog; the mock walks each flow to completion.
const login = ref<{
  provider: SettingsProviderEntry
  phase: ProviderLoginPhase
} | null>(null)
const loginOpen = ref(false)
let loginTimer = 0

/**
 * Claude Code signs in on Claude's page, whose code the user pastes back; Codex
 * and Grok Build confirm a device code in a web browser. The mock walks each to
 * done.
 */
function beginLogin(p: SettingsProviderEntry) {
  window.clearTimeout(loginTimer)
  loginOpen.value = true
  const mock = p as MockProvider
  login.value = {
    provider: p,
    phase: { kind: 'starting' },
  }
  loginTimer = window.setTimeout(() => {
    if (!login.value) {
      return
    }
    if (mock.family === 'claude-code') {
      login.value.phase = {
        kind: 'code',
        url: 'https://claude.com/cai/oauth/authorize?code=true',
        expiresIn: '15 min',
        submitted: false,
      }
      return
    }
    const grok = mock.family === 'grok-build'
    login.value.phase = {
      kind: 'device',
      url: grok ? 'https://accounts.x.ai/device' : 'https://auth.openai.com/codex/device',
      code: grok ? 'QK4P-9TWD' : 'HXRV-7K2M',
      expiresIn: '10 min',
    }
    loginTimer = window.setTimeout(() => {
      if (!login.value) {
        return
      }
      login.value.phase = {
        kind: 'done',
        account: grok ? 'zan@example.com · SuperGrok' : 'zan@example.com · Plus',
        // As in the product: an account added beside another is not switched to.
        active: login.value.provider.accounts.length === 0,
      }
    }, 4000)
  }, 900)
}

/**
 * As the product's sign-in does once it has the code: it signs in, then the account is there.
 * A code without the `#` the vendor's page shows is refused in Claude Code's words, and the
 * sign-in waits for another.
 */
function submitCode(code: string) {
  const phase = login.value?.phase
  if (phase?.kind !== 'code') {
    return
  }
  login.value!.phase = { kind: 'code', url: phase.url, expiresIn: phase.expiresIn, submitted: true }
  loginTimer = window.setTimeout(() => {
    if (login.value?.phase.kind !== 'code') {
      return
    }
    if (!code.includes('#')) {
      login.value.phase = {
        ...login.value.phase,
        submitted: false,
        error: 'Invalid code. Please make sure the full code was copied.',
      }
      return
    }
    finishLogin('zan@example.com · max')
  }, 1500)
}

function finishLogin(account: string) {
  if (!login.value) {
    return
  }
  window.clearTimeout(loginTimer)
  login.value.phase = {
    kind: 'done',
    account,
    active: login.value.provider.accounts.length === 0,
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
          resetsAt: ahead(298 * MINUTE),
        },
        {
          id: 'week',
          label: 'Weekly',
          used: 4,
          max: 100,
          resetsAt: ahead(3 * DAY + 4 * HOUR),
        },
      ],
    })
    p.state = 'ready'
    settle(p)
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
    :refreshing-usage="refreshingUsage"
    :usage-failures="usageFailures"
    @change="changeProvider"
    @toggle-model="(_provider, model, enabled) => (model.enabled = enabled)"
    @add="addProvider"
    @add-endpoint="addEndpoint"
    @remove="removeProvider"
    @sign-in="beginLogin"
    @test="test"
    @refresh="refresh"
    :operations="operations"
    @refresh-usage="refreshUsage"
    @check-cli="checkCli"
    @install-cli="installCli"
    @activate-account="activateAccount"
    @remove-account="removeAccount"
    :save-model="saveModel"
    :search-models="searchModels"
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
    @submit-code="submitCode"
  />
</template>

<script setup lang="ts">
import { requestFailure } from '../fixtures/request-failures'
import type { ShownError } from '@demicodes/web-ui/infra/errors'
import { computed, onScopeDispose, ref } from 'vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import ProviderLoginDialog, {
  type ProviderLoginPhase,
} from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import type { ProviderLoginStage } from '@demicodes/web-ui/settings/types'
import SettingsProvidersPage from '@demicodes/web-ui/settings/SettingsProvidersPage.vue'
import {
  type SettingsModelDraft,
  type SettingsQuotaRefresh,
  type SettingsProviderEntry,
  type SettingsProviderModel,
  type SettingsVendor,
} from '@demicodes/web-ui/settings/types'
import {
  isIncomplete,
  mockVendors,
  elevenLabsAudio,
  provider,
  searchMockModels,
  type MockProvider,
  type SettingsState,
} from '../fixtures/settings'
import { slow } from '../fixtures/slow'
import { operations } from '@demicodes/web-ui/ui/operations'
import { providerOperation } from '@demicodes/web-ui/settings/provider-operations'
import { DAY, HOUR, MINUTE, ahead } from '../fixtures/time'
import { macPortOf, useGalleryExposedPorts } from '../fixtures/exposed-ports'
import { HOUR_SECONDS, WEEK_SECONDS, windowNamed } from '../fixtures/usage-limits'

/**
 * The shared providers page over the mock state. Everything the page emits is
 * answered here the way a host would, with timers standing in for the network.
 */
const props = defineProps<{
  state: SettingsState
}>()

const s = computed(() => props.state)

/**
 * As the product's page shows an entry from what it holds: it is incomplete,
 * with the yellow dot and nothing red, until the last part is saved.
 */
function settle(entry: SettingsProviderEntry): void {
  if (isIncomplete(entry)) {
    entry.state = 'setup'
  } else if (entry.state === 'setup') {
    entry.state = 'ready'
  }
}

function changeProvider(
  entry: SettingsProviderEntry,
  patch: Partial<SettingsProviderEntry>,
): void {
  Object.assign(entry, patch)
  if (patch.apiKey !== undefined) {
    // As in the product, the page never gets a stored key back: only whether
    // one exists, none once the field was emptied.
    entry.apiKey = ''
    entry.configured = true
    entry.keyConfigured = patch.apiKey !== ''
  }
  settle(entry)
}

function select(id: string) {
  s.value.selectedProviderId = id
  s.value.providerDetailOpen = true
}

/** A vendor is picked alone: the entry joins the list at once, selected, with the vendor's own endpoint and catalog. */
function addProvider(vendor: SettingsVendor) {
  const id = `p-${Date.now()}`
  // A vendor of media alone, ElevenLabs here, makes no text: its entry has its Audio and no Models.
  const mediaOnly = vendor.wireApi === null
  join(provider({
    id,
    name: vendor.name,
    kind: 'api_key',
    family: mediaOnly ? 'media' : vendor.id,
    vendorId: vendor.id,
    baseUrl: vendor.baseUrl ?? '',
    wireApi: vendor.wireApi ?? 'openai-responses',
    modelSource: 'catalog',
    catalogFetched: new Date().toISOString(),
    logo: vendor.logo,
    ...(mediaOnly
      ? {
          makesText: false,
          media: { image: { shape: null, enabled: true }, video: { shape: null, enabled: true }, audio: elevenLabsAudio },
        }
      : {}),
  }))
}

/**
 * As the backend creates a compatible API: at once, as Anthropic Messages, with
 * no endpoint, key or model, named *Compatible API* or the first free *(2)*,
 * *(3)*; its page sets the rest.
 */
function addCompatible() {
  const taken = (name: string) => s.value.providers.some((entry) => entry.name === name)
  let name = 'Compatible API'
  for (let number = 2; taken(name); number += 1) {
    name = `Compatible API (${number})`
  }
  join(provider({
    id: `p-${Date.now()}`,
    name,
    kind: 'api_key',
    family: 'anthropic',
    vendorId: null,
    compatible: true,
    wireApi: 'anthropic-messages',
    baseUrl: '',
    keyConfigured: false,
    configured: true,
    modelSource: 'manual',
  }))
}

function join(entry: MockProvider) {
  settle(entry)
  s.value.providers.push(entry)
  select(entry.id)
}

async function removeProvider(id: string) {
  await slow()
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

/** The gallery's exposed ports, which a device's page and Expose Port on a failed test change. */
const exposedPorts = useGalleryExposedPorts()

/** The gallery is a shared instance whose master is Alex: an endpoint naming alex-mbp serves every user. */
function sharedDevice(endpoint: string): string | null {
  return macPortOf(endpoint) === null ? null : 'alex-mbp'
}

/**
 * The mock judges a connection by its inputs: no key is rejected, localhost
 * is unreachable, and a port of alex-mbp its id names is refused until it is
 * exposed. The test is the provider's operation, as the product's store
 * starts it (`provider-operations.ts`).
 */
function test(p: SettingsProviderEntry, accountId?: string) {
  void operations.start(
    providerOperation(p.id, accountId ? { kind: 'account', accountId, action: 'test' } : { kind: 'testing' }),
    () => slow(1200).then(() => {
      const port = macPortOf(p.baseUrl)
      p.notExposed = undefined
      if (port !== null && !exposedPorts.isExposed('mac', port)) {
        p.state = 'error'
        p.detail = `Port ${port} of alex-mbp isn’t exposed.`
        p.notExposed = { deviceId: 'mac', device: 'alex-mbp', port }
      } else if (p.vendorId && !p.apiKey && !p.keyConfigured) {
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
    }),
  )
}

/** Expose Port on a failed test: the port is exposed, as Settings › Devices then shows it, and the test runs again. */
async function exposePort(p: SettingsProviderEntry, deviceId: string, port: number) {
  await exposedPorts.expose(deviceId, port)
  test(p)
}

function refresh(p: SettingsProviderEntry) {
  void operations.start(providerOperation(p.id, { kind: 'refreshing' }), () =>
    slow(1400).then(() => {
      p.catalogFetched = new Date().toISOString()
    }),
  )
}

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
  void operations.start(providerOperation(p.id, { kind: 'cli' }), () =>
    slow(1200).then(() => {
      if (p.cli) {
        p.cli.newest = { version: '2.1.279' }
      }
    }),
  )
}

/** How long the Cloud takes to wake, read the release and install the CLI. */
const CLI_INSTALL_MS = 3000
/** The specimen's CLI installs under way, which end when the page goes. */
const cliInstalls = new Set<number>()

/** Installing while Claude Code’s machine wakes and downloads, then installed; Install works until the request is taken. */
async function installCli(p: SettingsProviderEntry) {
  const cli = p.cli
  if (!cli || cli.install?.state === 'installing') {
    return
  }
  await slow()
  cli.install = { state: 'installing' }
  const timer = window.setTimeout(() => {
    cliInstalls.delete(timer)
    cli.install = { state: 'installed' }
  }, CLI_INSTALL_MS)
  cliInstalls.add(timer)
}

/** Whether the specimen's user went on past the warning about several accounts; a reload asks again. */
const multiAccountWarned = ref(false)

/** As the product's list shows the order of the entries the user dragged to, at once. */
function reorderProviders(ids: string[]) {
  s.value.providers = ids.flatMap((id) => s.value.providers.filter((entry) => entry.id === id))
}

/** As the product's list shows the order the user dragged to, at once. */
function reorderAccounts(p: SettingsProviderEntry, ids: string[]) {
  p.accounts = ids.flatMap((id) => p.accounts.filter((account) => account.id === id))
}

function activateAccount(p: SettingsProviderEntry, id: string) {
  for (const a of p.accounts) {
    a.active = a.id === id
  }
}

/** As the backend does: the active account's removal makes the first remaining one active, or leaves none. */
async function removeAccount(p: SettingsProviderEntry, id: string) {
  await slow()
  const wasActive = p.accounts.some((a) => a.id === id && a.active)
  p.accounts = p.accounts.filter((a) => a.id !== id)
  if (wasActive && p.accounts[0]) {
    p.accounts[0].active = true
  }
  if (!p.accounts.length) {
    p.state = 'signed-out'
  }
}

/**
 * As the product's store saves a model of the list: the edited one in its
 * place, a new one, whose id the page already freed, at the end. An entry that
 * follows its vendor's list starts its own from it, the vendor's copy of the
 * model giving way to the one added.
 */
async function saveModel(
  p: SettingsProviderEntry,
  draft: SettingsModelDraft,
  original: SettingsProviderModel | null,
) {
  const mock = p as MockProvider
  if (original && mock.models.some((m) => m.id === original.id)) {
    await slow()
    mock.models = mock.models.map((m) => (m.id === original.id ? { ...m, ...draft, enabled: m.enabled } : m))
  } else {
    if (mock.modelSource === 'catalog') {
      mock.models = mock.models.filter((m) => m.id !== draft.id)
    }
    mock.modelSource = 'manual'
    // As the backend answers: an audio model of an entry whose audio shape
    // is OpenAI Speech makes speech, which its row names.
    const kinds = draft.kind === 'audio' && p.media.audio.shape === 'openai-speech' ? { kinds: ['speech' as const] } : {}
    mock.models = [...mock.models, { ...draft, tools: true, enabled: original?.enabled ?? true, ...kinds }]
  }
  settle(p)
}

async function removeModel(p: SettingsProviderEntry, m: SettingsProviderModel) {
  await slow()
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
    phase: { kind: 'starting', stage: 'preparing' },
  }
  // As the product's: a Claude sign-in waits for Claude Code's machine and the command-line tool's start first.
  const stages: ProviderLoginStage[] = mock.family === 'claude-code' ? ['waiting_for_machine', 'starting'] : []
  const next = () => {
    if (!login.value) {
      return
    }
    const stage = stages.shift()
    if (stage) {
      login.value.phase = { kind: 'starting', stage }
      loginTimer = window.setTimeout(next, 1200)
      return
    }
    if (mock.family === 'claude-code') {
      login.value.phase = {
        kind: 'code',
        url: 'https://claude.com/cai/oauth/authorize?code=true',
        expiresAt: Date.now() + 15 * 60_000,
        submitted: false,
      }
      return
    }
    const grok = mock.family === 'grok-build'
    login.value.phase = {
      kind: 'device',
      url: grok ? 'https://accounts.x.ai/device' : 'https://auth.openai.com/codex/device',
      code: grok ? 'QK4P-9TWD' : 'HXRV-7K2M',
      expiresAt: Date.now() + 10 * 60_000,
    }
    loginTimer = window.setTimeout(() => {
      if (!login.value) {
        return
      }
      login.value.phase = {
        kind: 'done',
        account: grok ? 'alex@example.com · SuperGrok' : 'alex@example.com · Plus',
        // As in the product: an account added beside another is not switched to.
        active: login.value.provider.accounts.length === 0,
      }
    }, 4000)
  }
  loginTimer = window.setTimeout(next, 900)
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
  // The sign-in takes the code at once, as the product's answers its number.
  login.value!.phase = {
    kind: 'code',
    url: phase.url,
    expiresAt: phase.expiresAt,
    submitted: true,
    taken: (phase.taken ?? 0) + 1,
  }
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
    // The user signs in to the account the vendor signed out, when there is one, as after
    // Sign In Again.
    const signedOut = login.value.provider.accounts.find((a) => a.signedOut)
    finishLogin(signedOut ? `${signedOut.label} · ${signedOut.plan}` : 'alex@example.com · max')
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
  const phase = login.value?.phase
  if (login.value && phase?.kind === 'done') {
    const p = login.value.provider
    // As the product's: the account added is the active one only when the
    // dialog said so, for the provider's first account.
    if (phase.active) {
      for (const a of p.accounts) {
        a.active = false
      }
    }
    const [label = phase.account, plan = ''] = phase.account.split(' · ')
    // As the product's: a sign-in to an account already there replaces its sign-in.
    const again = p.accounts.find((a) => a.label === label)
    if (again) {
      again.signedOut = false
    } else {
      p.accounts.push({
      id: `a-${Date.now()}`,
      label,
      plan,
      active: phase.active,
      quota: [
        {
          id: 'hour',
          name: windowNamed('limit', 5 * HOUR_SECONDS),
          used: 4,
          max: 100,
          resetsAt: ahead(298 * MINUTE),
        },
        {
          id: 'week',
          name: windowNamed('limit', WEEK_SECONDS),
          used: 4,
          max: 100,
          resetsAt: ahead(3 * DAY + 4 * HOUR),
        },
      ],
      })
    }
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
    :refreshing-usage="refreshingUsage"
    :usage-failures="usageFailures"
    @change="changeProvider"
    @toggle-model="(_provider, model, enabled) => (model.enabled = enabled)"
    @add="addProvider"
    @add-compatible="addCompatible"
    @remove="removeProvider"
    @sign-in="beginLogin"
    @test="test"
    @expose-port="exposePort"
    :is-exposed="exposedPorts.isExposed"
    :shared-device="sharedDevice"
    @refresh="refresh"
    @refresh-usage="refreshUsage"
    @check-cli="checkCli"
    @install-cli="installCli"
    @activate-account="activateAccount"
    @reorder-accounts="reorderAccounts"
    @reorder-providers="reorderProviders"
    :multi-account-warned="multiAccountWarned"
    @multi-account-warned="multiAccountWarned = true"
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

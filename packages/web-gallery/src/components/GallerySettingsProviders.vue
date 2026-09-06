<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { Brain, Check, Image, Info, Plug, Plus, RefreshCw, Search, SlidersHorizontal, Terminal, TextCursorInput, Trash2, TriangleAlert, Zap } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Meter from '@demicodes/web-ui/ui/Meter.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import ModelDialog from '@demicodes/web-ui/settings/ModelDialog.vue'
import ProviderLoginDialog, { type ProviderLoginPhase } from '@demicodes/web-ui/settings/ProviderLoginDialog.vue'
import type { SettingsModelDraft } from '@demicodes/web-ui/settings/types'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import SettingsListItem from '@demicodes/web-ui/settings/SettingsListItem.vue'
import SettingsPage from '@demicodes/web-ui/settings/SettingsPage.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import SettingsSplit from '@demicodes/web-ui/settings/SettingsSplit.vue'
import { mockVendors, subscriptionVendors, type MockModel, type MockProvider, type MockVendor, type SettingsState, type WireApi } from '../fixtures/settings'

/**
 * Models & providers as a list beside the selected provider. An API-key entry edits
 * its endpoint, key and model list in place; a subscription entry manages accounts
 * and its models are whatever the vendor serves. Signing in opens its own dialog.
 */
const props = defineProps<{
  state: SettingsState
}>()

const s = computed(() => props.state)
const subscriptions = computed(() => s.value.providers.filter((p) => p.kind === 'subscription'))
const apiKeys = computed(() => s.value.providers.filter((p) => p.kind === 'api_key'))
const renaming = ref(false)
const renameDraft = ref('')
const renameInput = ref<InstanceType<typeof TextInput>>()
function beginRename() {
  if (!selected.value) return
  renameDraft.value = selected.value.name
  renaming.value = true
  void nextTick(() => {
    renameInput.value?.focus()
    renameInput.value?.select()
  })
}
function commitRename() {
  if (!renaming.value) return
  renaming.value = false
  const name = renameDraft.value.trim()
  if (name && selected.value) selected.value.name = name
}
const selected = computed(() => s.value.providers.find((p) => p.id === s.value.selectedProviderId) ?? null)
const vendorOf = (p: MockProvider) => mockVendors.find((v) => v.id === p.vendorId) ?? null

/** Adding a provider: first pick where it comes from, then fill the form. */
type Draft =
  | { step: 'pick'; query: string }
  | { step: 'form'; vendor: MockVendor | null; name: string; baseUrl: string; wireApi: WireApi; key: string }
const draft = ref<Draft | null>(null)

const detailOpen = computed({
  get: () => s.value.providerDetailOpen && (draft.value !== null || selected.value !== null),
  set: (value: boolean) => {
    s.value.providerDetailOpen = value
  },
})

const modelFilter = ref('')
/** The model dialog: create or edit a manual model, or view a catalog one. */
const modelDialog = ref<{ mode: 'create' | 'edit' | 'view'; model: SettingsModelDraft; original: MockModel | null } | null>(null)
const modelDialogOpen = ref(false)
const testing = ref<string | null>(null)

const stateTone = { ready: 'success', error: 'danger', unreachable: 'danger', 'signed-out': 'warning', disabled: 'neutral' } as const
const stateWord = { ready: 'Connected', error: 'Key rejected', unreachable: 'Unreachable', 'signed-out': 'Signed out', disabled: 'Off' } as const
const wireOptions: { value: WireApi; label: string }[] = [
  { value: 'anthropic-messages', label: 'Anthropic Messages' },
  { value: 'openai-responses', label: 'OpenAI Responses' },
  { value: 'openai-chat', label: 'OpenAI Chat Completions' },
]
const wireLabel = (w: WireApi) => wireOptions.find((o) => o.value === w)?.label ?? w

const pickResults = computed(() => {
  if (draft.value?.step !== 'pick') return mockVendors
  const q = draft.value.query.trim().toLowerCase()
  return q ? mockVendors.filter((v) => v.name.toLowerCase().includes(q) || v.id.includes(q)) : mockVendors
})

function select(id: string) {
  draft.value = null
  s.value.selectedProviderId = id
  s.value.providerDetailOpen = true
}

function startDraft() {
  draft.value = { step: 'pick', query: '' }
  s.value.selectedProviderId = null
  s.value.providerDetailOpen = true
}

function pickVendor(vendor: MockVendor | null) {
  draft.value = {
    step: 'form',
    vendor,
    name: vendor?.name ?? '',
    baseUrl: vendor?.baseUrl ?? '',
    wireApi: vendor?.wireApi ?? 'openai-chat',
    key: '',
  }
}

function cancelDraft() {
  draft.value = null
  s.value.providerDetailOpen = false
}

function formatTokens(n: number | null): string {
  if (n === null) return '—'
  return n >= 1_000_000 ? `${n / 1_000_000}M` : `${Math.round(n / 1000)}K`
}

/** What a model can do, as marks: a context size, then icons with tooltips. */
const isUnknown = (m: MockModel) => m.contextWindow === null

function visibleModels(p: MockProvider): MockModel[] {
  const q = modelFilter.value.trim().toLowerCase()
  return q ? p.models.filter((m) => m.id.includes(q) || m.name.toLowerCase().includes(q)) : p.models
}

function toDraft(m: MockModel): SettingsModelDraft {
  return { id: m.id, name: m.name, contextWindow: m.contextWindow, outputLimit: m.outputLimit, efforts: [...m.efforts], extensions: [...m.extensions], fastTier: m.fastTier }
}

function openModel(mode: 'create' | 'edit' | 'view', m: MockModel | null) {
  modelDialogOpen.value = true
  modelDialog.value = {
    mode,
    original: m,
    model: m ? toDraft(m) : { id: '', name: '', contextWindow: null, outputLimit: null, efforts: [], extensions: [], fastTier: null },
  }
}

function saveModel(draft: SettingsModelDraft) {
  const p = selected.value
  const dialog = modelDialog.value
  if (!p || !dialog) return
  if (dialog.original) {
    Object.assign(dialog.original, { name: draft.name, contextWindow: draft.contextWindow, outputLimit: draft.outputLimit, efforts: draft.efforts, extensions: draft.extensions, fastTier: draft.fastTier })
  } else if (!p.models.some((m) => m.id === draft.id)) {
    p.models.push({ ...draft, tools: true, defaultEffort: draft.efforts[0] ?? null, enabled: true })
  }
  modelDialogOpen.value = false
}

/** The header checkbox over the visible models: all, none, or some of them on. */
function visibleSelection(p: MockProvider): { checked: boolean; partial: boolean } {
  const visible = visibleModels(p)
  const on = visible.filter((m) => m.enabled).length
  return { checked: visible.length > 0 && on === visible.length, partial: on > 0 && on < visible.length }
}

function setActive(p: MockProvider, id: string) {
  for (const a of p.accounts) a.active = a.id === id
}

function test(p: MockProvider) {
  testing.value = p.id
  window.setTimeout(() => {
    testing.value = null
  }, 1200)
}

// Sign-in runs in its own dialog; the mock walks the device-code flow to completion.
const login = ref<{ provider: MockProvider; phase: ProviderLoginPhase } | null>(null)
const loginOpen = ref(false)
let loginTimer = 0

function beginLogin(p: MockProvider) {
  window.clearTimeout(loginTimer)
  loginOpen.value = true
  login.value = { provider: p, phase: { kind: 'starting' } }
  loginTimer = window.setTimeout(() => {
    if (!login.value) return
    login.value.phase = { kind: 'device', url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expiresIn: '10 min' }
    loginTimer = window.setTimeout(() => {
      if (!login.value) return
      login.value.phase = { kind: 'done', account: 'zan@example.com · Plus' }
    }, 4000)
  }, 900)
}

function closeLogin() {
  window.clearTimeout(loginTimer)
  if (login.value?.phase.kind === 'done') {
    const p = login.value.provider
    for (const a of p.accounts) a.active = false
    p.accounts.push({ id: `a-${Date.now()}`, label: 'zan@example.com', plan: 'Plus', active: true, quota: { hour: { used: 4, max: 100, resets: 'in 4 h 58 min' }, week: { used: 4, max: 100, resets: 'Monday' } } })
    p.state = 'ready'
  }
  loginOpen.value = false
}

const cap = (word: string) => word.charAt(0).toUpperCase() + word.slice(1)

function setAll(p: MockProvider, enabled: boolean) {
  for (const m of visibleModels(p)) m.enabled = enabled
}
</script>

<template>
  <SettingsPage wide title="Models & providers" description="Where conversations get their models. Pick a provider to edit its connection and the models it offers.">
    <SettingsSplit v-model:detail-open="detailOpen" :detail-title="draft ? 'New provider' : selected?.name">
      <template #list>
        <div class="select-none px-1 pb-1 pt-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">Subscriptions</div>
        <SettingsListItem
          v-for="p in subscriptions"
          :key="p.id"
          :label="p.name"
          :selected="!draft && p.id === s.selectedProviderId"
          :dot="stateTone[p.state]"
          :muted="!p.enabled"
          @select="select(p.id)"
        >
          <template #leading><VendorMark :label="p.name" :src="p.logo" size="sm" /></template>
        </SettingsListItem>
        <div class="select-none px-1 pb-1 pt-3 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">API keys</div>
        <SettingsListItem
          v-for="p in apiKeys"
          :key="p.id"
          :label="p.name"
          :selected="!draft && p.id === s.selectedProviderId"
          :dot="stateTone[p.state]"
          :muted="!p.enabled"
          @select="select(p.id)"
        >
          <template #leading><VendorMark :label="p.name" :src="p.logo" size="sm" /></template>
        </SettingsListItem>
        <div class="mt-2 border-t border-line-subtle pt-2">
          <SettingsListItem label="Add provider" :icon="Plus" :selected="!!draft" @select="startDraft" />
        </div>
      </template>

      <template #detail>
        <!-- New provider: pick -->
        <div v-if="draft?.step === 'pick'" class="flex flex-col gap-5">
          <header class="select-none">
            <h3 class="text-[15px] font-medium text-fg-emphasis">Add a provider</h3>
            <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">A vendor from models.dev brings its endpoint and model catalog. A custom endpoint takes any compatible server.</p>
          </header>
          <div class="flex items-center gap-2">
            <Search :size="ICON_PX.in24" class="shrink-0 text-fg-subtle" />
            <TextInput v-model="draft.query" placeholder="Search vendors" class="w-64 max-w-full" />
          </div>
          <div class="settings-card overflow-hidden rounded-xl border border-line">
            <div
              v-for="v in pickResults"
              :key="v.id"
              role="button"
              class="flex h-12 cursor-default select-none items-center gap-3 px-3 hover:bg-hover"
              @click="pickVendor(v)"
            >
              <VendorMark :label="v.name" :src="v.logo" />
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">{{ v.name }}</span>
              <Tag>{{ wireLabel(v.wireApi) }}</Tag>
            </div>
            <div v-if="!pickResults.length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">No vendor matches. Add it as a custom endpoint.</div>
            <div role="button" class="flex h-12 cursor-default select-none items-center gap-3 px-3 hover:bg-hover" @click="pickVendor(null)">
              <span class="inline-flex size-7 items-center justify-center rounded-md bg-overlay/8 text-fg-muted"><Terminal :size="ICON_PX.in28" /></span>
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">Custom endpoint</span>
              <Tag>Any protocol</Tag>
            </div>
          </div>
          <div class="select-none text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">Subscriptions</div>
          <div class="settings-card overflow-hidden rounded-xl border border-line">
            <div v-for="v in subscriptionVendors" :key="v.id" role="button" class="flex h-12 cursor-default select-none items-center gap-3 px-3 hover:bg-hover">
              <VendorMark :label="v.name" :src="v.logo" />
              <span class="min-w-0 flex-1 truncate text-chrome text-fg">{{ v.name }}</span>
              <Tag>Sign in</Tag>
            </div>
          </div>
          <div class="flex justify-end">
            <Button @click="cancelDraft">Cancel</Button>
          </div>
        </div>

        <!-- New provider: form -->
        <div v-else-if="draft?.step === 'form'" class="flex flex-col gap-6">
          <header class="flex items-center gap-3">
            <VendorMark :label="draft.vendor?.name ?? 'Custom'" :src="draft.vendor?.logo" />
            <div class="min-w-0 select-none">
              <h3 class="truncate text-[15px] font-medium text-fg-emphasis">{{ draft.vendor ? draft.vendor.name : 'Custom endpoint' }}</h3>
              <p class="text-[12px] text-fg-subtle">{{ draft.vendor ? `${wireLabel(draft.wireApi)} · catalog from models.dev` : 'You name the protocol and the models.' }}</p>
            </div>
            <Button size="sm" class="ml-auto" @click="draft = { step: 'pick', query: '' }">Change</Button>
          </header>
          <div class="settings-card overflow-hidden rounded-xl border border-line">
            <SettingsRow label="Name" description="How it appears in the model picker.">
              <TextInput v-model="draft.name" placeholder="My provider" class="w-56 max-w-full" />
            </SettingsRow>
            <SettingsRow label="Base URL" :description="draft.vendor && !draft.vendor.baseUrl ? 'The vendor default. Change it for a proxy.' : undefined">
              <TextInput v-model="draft.baseUrl" :placeholder="draft.vendor ? 'Vendor default' : 'https://api.example.com/v1'" class="w-72 max-w-full" />
            </SettingsRow>
            <SettingsRow v-if="!draft.vendor" label="Protocol" description="What the endpoint speaks.">
              <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Protocol">
                <template #trigger>{{ wireLabel(draft.wireApi) }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem v-for="w in wireOptions" :key="w.value" :label="w.label" choice :is-selected="draft.wireApi === w.value" @select="draft!.step === 'form' && (draft.wireApi = w.value); close()" />
                  </Menu>
                </template>
              </Dropdown>
            </SettingsRow>
            <SettingsRow label="API key" description="Kept in the keychain of this device.">
              <TextInput v-model="draft.key" placeholder="sk-…" class="w-56 max-w-full" />
            </SettingsRow>
          </div>
          <div class="flex items-center justify-end gap-2">
            <Button @click="cancelDraft">Cancel</Button>
            <Button variant="primary" :disabled="!draft.name.trim() || (!draft.vendor && !draft.baseUrl.trim())">Add provider</Button>
          </div>
        </div>

        <!-- Existing provider -->
        <div v-else-if="selected" class="flex flex-col gap-6">
          <!-- The rail carries the mark; the first card's header names the provider, edits that
               name in place, and holds its actions. -->
          <SettingsGroup>
            <template #header>
              <header class="select-none">
                <div class="flex h-8 flex-wrap items-center gap-x-1.5 gap-y-2">
                  <TextInput
                    v-if="renaming"
                    ref="renameInput"
                    v-model="renameDraft"
                    class="w-56 max-w-full"
                    aria-label="Provider name"
                    @keydown.enter.prevent="commitRename"
                    @keydown.escape.prevent="renaming = false"
                    @blur="commitRename"
                  />
                  <template v-else>
                    <h3 class="min-w-0 truncate text-[15px] font-medium text-fg-emphasis">{{ selected.name }}</h3>
                    <Tooltip content="Rename"><IconButton :icon="TextCursorInput" variant="ghost" size="sm" aria-label="Rename provider" @click="beginRename" /></Tooltip>
                  </template>
                  <div class="ml-auto flex items-center gap-1.5">
                    <Tooltip content="Remove"><IconButton :icon="Trash2" variant="danger" size="sm" aria-label="Remove provider" /></Tooltip>
                    <Switch v-model="selected.enabled" size="sm" class="ml-2" />
                  </div>
                </div>
              </header>
            </template>

            <!-- Accounts (subscription) -->
            <template v-if="selected.kind === 'subscription'">
            <SettingsRow v-for="account in selected.accounts" :key="account.id" :label="account.label" compact>
              <template #tags><Tag>{{ account.plan }}</Tag><Tag v-if="account.active" tone="accent">Active</Tag><Tag v-if="account.quota && account.quota.hour.used >= 100" tone="danger">Limit reached</Tag></template>
              <template v-if="account.quota" #description>
                <div class="mt-1 grid max-w-72 grid-cols-[3.25rem_minmax(0,1fr)_auto] items-center gap-x-2 gap-y-1 text-[11px] tabular-nums">
                  <span>5-hour</span>
                  <Meter :value="account.quota.hour.used" :max="account.quota.hour.max" label="5-hour window" />
                  <span>{{ account.quota.hour.used }}% · resets {{ account.quota.hour.resets }}</span>
                  <span>Weekly</span>
                  <Meter :value="account.quota.week.used" :max="account.quota.week.max" label="Weekly window" />
                  <span>{{ account.quota.week.used }}% · resets {{ account.quota.week.resets }}</span>
                </div>
              </template>
              <Button v-if="!account.active" size="sm" @click="setActive(selected!, account.id)">Use</Button>
              <Tooltip content="Remove account"><IconButton :icon="Trash2" variant="danger" size="sm" aria-label="Remove account" /></Tooltip>
            </SettingsRow>
            <div v-if="!selected.accounts.length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">No account yet. Sign in to use this provider.</div>
            <SettingsRow label="Add an account" description="Signs in with the vendor's own login.">
              <Button variant="primary" size="sm" @click="beginLogin(selected!)">Sign in</Button>
            </SettingsRow>
            </template>

            <!-- Connection (API key) -->
            <template v-else>
            <SettingsRow label="Base URL" :description="vendorOf(selected) ? `${vendorOf(selected)!.name} on models.dev · ${wireLabel(selected.wireApi)}` : 'A custom endpoint. Protocol below.'">
              <TextInput v-model="selected.baseUrl" class="w-72 max-w-full" />
            </SettingsRow>
            <SettingsRow v-if="!vendorOf(selected)" label="Protocol">
              <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Protocol">
                <template #trigger>{{ wireLabel(selected.wireApi) }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem v-for="w in wireOptions" :key="w.value" :label="w.label" choice :is-selected="selected.wireApi === w.value" @select="selected!.wireApi = w.value; close()" />
                  </Menu>
                </template>
              </Dropdown>
            </SettingsRow>
            <SettingsRow label="API key" :description="selected.keyHint ? undefined : 'Local endpoints usually need none.'">
              <TextInput :model-value="selected.keyHint ? 'sk-ant-api03-3f2a9c1d7e5b4a6f8c2d1e9b' : ''" type="password" placeholder="sk-…" class="w-72 max-w-full" />
            </SettingsRow>
            <SettingsRow label="Test connection">
              <span v-if="testing === selected.id" class="flex items-center gap-1.5 text-[12px] text-fg-subtle"><IndeterminateSpinner :size="ICON_PX.in24" /> Testing…</span>
              <span v-else-if="selected.state === 'ready'" class="flex items-center gap-1 text-[12px] text-on-success"><Check :size="ICON_PX.in24" /> OK · 412 ms</span>
              <span v-else-if="selected.detail" class="min-w-0 truncate font-mono text-[12px] text-on-danger">{{ selected.detail }}</span>
              <Tooltip content="Test connection"><IconButton :icon="Plug" size="sm" aria-label="Test connection" @click="test(selected!)" /></Tooltip>
            </SettingsRow>
            </template>
          </SettingsGroup>

          <!-- Models -->
          <SettingsGroup title="Models">
            <SettingsRow
              v-if="selected.kind === 'api_key'"
              label="Model list"
              :description="selected.modelSource === 'catalog' ? `From the vendor catalog · fetched ${selected.catalogFetched}` : 'Ids you enter. Fill in what a model can do so the composer offers the right controls.'"
            >
              <template #tags><Tag v-if="selected.stale" tone="warning">Stale</Tag></template>
              <Tooltip v-if="selected.modelSource === 'catalog'" content="Refresh the catalog"><IconButton :icon="RefreshCw" size="sm" aria-label="Refresh models" /></Tooltip>
              <Segmented v-model="selected.modelSource" size="sm" :options="[{ value: 'catalog', label: 'Catalog' }, { value: 'manual', label: 'Manual' }]" />
            </SettingsRow>
            <div v-if="selected.kind === 'api_key'" class="flex items-center gap-3 px-3 py-2">
              <Checkbox
                :model-value="visibleSelection(selected).checked"
                :partial="visibleSelection(selected).partial"
                label=""
                aria-label="Enable every listed model"
                @update:model-value="(on) => setAll(selected!, on)"
              />
              <TextInput v-model="modelFilter" placeholder="Filter models" class="min-w-0 flex-1">
                <template #prefix><Search :size="ICON_PX.in24" /></template>
              </TextInput>
              <Tooltip v-if="selected.modelSource === 'manual'" content="Add model"><IconButton :icon="Plus" aria-label="Add model" @click="openModel('create', null)" /></Tooltip>
            </div>
            <SettingsRow v-for="m in visibleModels(selected)" :key="m.id" :label="m.name || m.id" compact :class="m.enabled ? '' : 'opacity-60'">
              <template v-if="selected.kind === 'api_key'" #leading><Checkbox v-model="m.enabled" label="" :aria-label="`Enable ${m.name || m.id}`" /></template>
              <template #tags>
                <Tooltip v-if="isUnknown(m)" content="Capabilities unknown. Edit to fill them in."><Tag tone="warning"><TriangleAlert :size="12" /></Tag></Tooltip>
                <Tag v-if="m.contextWindow !== null">{{ formatTokens(m.contextWindow) }}</Tag>
                <Tooltip v-if="m.extensions.length" :content="`Accepts ${m.extensions.join(' ')}`"><Tag><Image :size="12" /></Tag></Tooltip>
                <Tooltip v-if="m.efforts.length" :content="`Reasoning · ${m.efforts.map(cap).join(', ')}`"><Tag><Brain :size="12" /></Tag></Tooltip>
                <Tooltip v-if="m.fastTier" content="Has a fast tier"><Tag><Zap :size="12" /></Tag></Tooltip>
              </template>
              <template v-if="selected.kind === 'api_key' && selected.modelSource === 'manual'">
                <Tooltip content="Edit"><IconButton :icon="SlidersHorizontal" size="sm" aria-label="Edit model" @click="openModel('edit', m)" /></Tooltip>
                <Tooltip content="Remove"><IconButton :icon="Trash2" variant="danger" size="sm" aria-label="Remove model" @click="selected!.models = selected!.models.filter((x) => x.id !== m.id)" /></Tooltip>
              </template>
              <Tooltip v-else content="Details"><IconButton :icon="Info" size="sm" aria-label="Model details" @click="openModel('view', m)" /></Tooltip>
            </SettingsRow>
            <div v-if="!visibleModels(selected).length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">
              {{ modelFilter ? 'No model matches.' : selected.kind === 'subscription' ? 'Sign in to see what this account can use.' : 'No models yet.' }}
            </div>
          </SettingsGroup>
        </div>
      </template>
    </SettingsSplit>

    <ModelDialog
      v-if="modelDialog"
      :is-open="modelDialogOpen"
      :overlay-store="appOverlayStore"
      :mode="modelDialog.mode"
      :model="modelDialog.model"
      @close="modelDialogOpen = false"
      @save="saveModel"
    />

    <ProviderLoginDialog
      v-if="login"
      :is-open="loginOpen"
      :overlay-store="appOverlayStore"
      :vendor-name="login.provider.name"
      :phase="login.phase"
      @close="closeLogin"
    />
  </SettingsPage>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import { Check, Copy, ExternalLink, KeyRound, Plus, RefreshCw, Search, Sparkles, UserRound } from '@lucide/vue'
import { appOverlayStore } from '@demicodes/web-ui/overlay/appOverlay'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuDivider from '@demicodes/web-ui/ui/MenuDivider.vue'
import MenuGroup from '@demicodes/web-ui/ui/MenuGroup.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Meter from '@demicodes/web-ui/ui/Meter.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import SettingsGroup from '@demicodes/web-ui/settings/SettingsGroup.vue'
import SettingsListItem from '@demicodes/web-ui/settings/SettingsListItem.vue'
import SettingsPage from '@demicodes/web-ui/settings/SettingsPage.vue'
import SettingsRow from '@demicodes/web-ui/settings/SettingsRow.vue'
import SettingsSplit from '@demicodes/web-ui/settings/SettingsSplit.vue'
import type { MockModel, MockProvider, SettingsState, WireApi } from '../fixtures/settings'

/**
 * Models & providers as a list beside the selected provider. An API-key entry edits
 * its endpoint, key and model list in place; a subscription entry manages accounts
 * and logins. Nothing here opens a further dialog: a model expands into its own rows.
 */
const props = defineProps<{
  state: SettingsState
}>()

const s = computed(() => props.state)
const subscriptions = computed(() => s.value.providers.filter((p) => p.kind === 'subscription'))
const apiKeys = computed(() => s.value.providers.filter((p) => p.kind === 'api_key'))
const selected = computed(() => s.value.providers.find((p) => p.id === s.value.selectedProviderId) ?? null)
/** A draft for "Add provider"; null while an existing entry is shown. */
const draft = ref<{ vendorId: string | null; name: string; baseUrl: string; wireApi: WireApi } | null>(null)

// Narrow hosts open the detail only when there is one to show; the flag itself is shared state.
const detailOpen = computed({
  get: () => s.value.providerDetailOpen && (draft.value !== null || selected.value !== null),
  set: (value: boolean) => {
    s.value.providerDetailOpen = value
  },
})

const expandedModelId = ref<string | null>('kimi-k2-turbo-preview')
const modelFilter = ref('')
const newModelId = ref('')
const testing = ref<string | null>(null)

const stateTone = { ready: 'success', error: 'danger', unreachable: 'danger', 'signed-out': 'warning', disabled: 'neutral' } as const
const stateWord = { ready: 'Connected', error: 'Key rejected', unreachable: 'Unreachable', 'signed-out': 'Signed out', disabled: 'Off' } as const
const wireOptions: { value: WireApi; label: string }[] = [
  { value: 'anthropic-messages', label: 'Anthropic Messages' },
  { value: 'openai-responses', label: 'OpenAI Responses' },
  { value: 'openai-chat', label: 'OpenAI Chat Completions' },
]
const vendors = [
  { id: 'anthropic', name: 'Anthropic', baseUrl: 'https://api.anthropic.com', wireApi: 'anthropic-messages' as WireApi },
  { id: 'openai', name: 'OpenAI', baseUrl: 'https://api.openai.com/v1', wireApi: 'openai-responses' as WireApi },
  { id: 'google', name: 'Google AI Studio', baseUrl: 'https://generativelanguage.googleapis.com', wireApi: 'openai-chat' as WireApi },
  { id: 'deepseek', name: 'DeepSeek', baseUrl: 'https://api.deepseek.com', wireApi: 'openai-chat' as WireApi },
  { id: 'openrouter', name: 'OpenRouter', baseUrl: 'https://openrouter.ai/api/v1', wireApi: 'openai-chat' as WireApi },
  { id: 'zhipu', name: 'Zhipu BigModel', baseUrl: 'https://open.bigmodel.cn/api/paas/v4', wireApi: 'openai-chat' as WireApi },
]

function dotOf(p: MockProvider) {
  return p.state === 'disabled' ? 'neutral' : stateTone[p.state]
}

function listDetail(p: MockProvider): string {
  if (p.kind === 'subscription') {
    const active = p.accounts.find((a) => a.active)
    return active ? `${active.plan} · ${active.label}` : p.login ? 'Signing in…' : 'Not signed in'
  }
  const on = p.models.filter((m) => m.enabled).length
  return p.vendorId ? `${on} of ${p.models.length} models` : `Custom · ${p.models.length} models`
}

function select(id: string) {
  draft.value = null
  s.value.selectedProviderId = id
  s.value.providerDetailOpen = true
  expandedModelId.value = null
}

function startDraft() {
  draft.value = { vendorId: null, name: '', baseUrl: '', wireApi: 'openai-chat' }
  s.value.selectedProviderId = null
  s.value.providerDetailOpen = true
}

function pickVendor(id: string | null) {
  if (!draft.value) return
  const vendor = vendors.find((v) => v.id === id)
  draft.value.vendorId = id
  draft.value.name = vendor?.name ?? ''
  draft.value.baseUrl = vendor?.baseUrl ?? ''
  draft.value.wireApi = vendor?.wireApi ?? 'openai-chat'
}

function formatTokens(n: number | null): string {
  if (n === null) return '—'
  return n >= 1_000_000 ? `${n / 1_000_000}M` : `${Math.round(n / 1000)}k`
}

function capabilityTags(m: MockModel): { text: string; tone: 'neutral' | 'warning' }[] {
  if (m.tools === null && m.attachments === null && m.contextWindow === null) return [{ text: 'Capabilities unknown', tone: 'warning' }]
  const tags: { text: string; tone: 'neutral' | 'warning' }[] = []
  if (m.contextWindow !== null) tags.push({ text: `${formatTokens(m.contextWindow)} ctx`, tone: 'neutral' })
  if (m.tools) tags.push({ text: 'tools', tone: 'neutral' })
  if (m.attachments) tags.push({ text: 'files', tone: 'neutral' })
  if (m.efforts.length) tags.push({ text: 'reasoning', tone: 'neutral' })
  if (m.fastTier) tags.push({ text: 'fast', tone: 'neutral' })
  return tags
}

function visibleModels(p: MockProvider): MockModel[] {
  const q = modelFilter.value.trim().toLowerCase()
  return q ? p.models.filter((m) => m.id.includes(q) || m.name.toLowerCase().includes(q)) : p.models
}

function addModel(p: MockProvider) {
  const id = newModelId.value.trim()
  if (!id || p.models.some((m) => m.id === id)) return
  p.models.push({ id, name: '', contextWindow: null, outputLimit: null, tools: null, attachments: null, efforts: [], defaultEffort: null, fastTier: null, enabled: true })
  newModelId.value = ''
  expandedModelId.value = id
}

function toggleEffort(m: MockModel, effort: string) {
  m.efforts = m.efforts.includes(effort) ? m.efforts.filter((e) => e !== effort) : [...m.efforts, effort]
  if (m.defaultEffort && !m.efforts.includes(m.defaultEffort)) m.defaultEffort = m.efforts[0] ?? null
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

const EFFORTS = ['minimal', 'low', 'medium', 'high', 'max']
</script>

<template>
  <SettingsPage wide title="Models & providers" description="Where conversations get their models. Pick a provider to edit its connection and the models it offers.">
    <SettingsSplit v-model:detail-open="detailOpen" :detail-title="draft ? 'New provider' : selected?.name">
      <template #list>
        <div class="select-none px-2 pb-1 pt-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">Subscriptions</div>
        <SettingsListItem
          v-for="p in subscriptions"
          :key="p.id"
          :label="p.name"
          :detail="listDetail(p)"
          :icon="UserRound"
          :selected="!draft && p.id === s.selectedProviderId"
          :dot="dotOf(p)"
          :muted="!p.enabled"
          @select="select(p.id)"
        />
        <div class="select-none px-2 pb-1 pt-3 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle">API keys</div>
        <SettingsListItem
          v-for="p in apiKeys"
          :key="p.id"
          :label="p.name"
          :detail="listDetail(p)"
          :icon="KeyRound"
          :selected="!draft && p.id === s.selectedProviderId"
          :dot="dotOf(p)"
          :muted="!p.enabled"
          @select="select(p.id)"
        />
        <div class="mt-2 border-t border-line-subtle pt-2">
          <SettingsListItem label="Add provider" :icon="Plus" :selected="!!draft" @select="startDraft" />
        </div>
      </template>

      <template #detail>
        <!-- New provider -->
        <div v-if="draft" class="flex flex-col gap-6 p-5">
          <header class="select-none">
            <h3 class="text-[15px] font-medium text-fg-emphasis">New provider</h3>
            <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">Pick a vendor to prefill its endpoint, or describe a custom one. Subscriptions sign in instead.</p>
          </header>
          <div class="settings-card overflow-hidden rounded-xl border border-line">
            <SettingsRow label="Vendor" description="Known vendors bring their model catalog along.">
              <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Vendor">
                <template #trigger>{{ vendors.find((v) => v.id === draft?.vendorId)?.name ?? 'Custom endpoint' }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem label="Custom endpoint" choice :is-selected="draft.vendorId === null" @select="pickVendor(null); close()" />
                    <MenuDivider />
                    <MenuItem v-for="v in vendors" :key="v.id" :label="v.name" choice :is-selected="draft.vendorId === v.id" @select="pickVendor(v.id); close()" />
                    <MenuGroup label="Subscriptions">
                      <MenuItem label="Claude Code" />
                      <MenuItem label="Codex" />
                      <MenuItem label="Grok Build" />
                    </MenuGroup>
                  </Menu>
                </template>
              </Dropdown>
            </SettingsRow>
            <SettingsRow label="Name" description="How it appears in the model picker.">
              <TextInput v-model="draft.name" placeholder="My provider" class="w-56 max-w-full" />
            </SettingsRow>
            <SettingsRow label="Base URL">
              <TextInput v-model="draft.baseUrl" placeholder="https://api.example.com/v1" class="w-72 max-w-full" />
            </SettingsRow>
            <SettingsRow v-if="!draft.vendorId" label="Protocol" description="What the endpoint speaks.">
              <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Protocol">
                <template #trigger>{{ wireOptions.find((w) => w.value === draft?.wireApi)?.label }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem v-for="w in wireOptions" :key="w.value" :label="w.label" choice :is-selected="draft.wireApi === w.value" @select="draft!.wireApi = w.value; close()" />
                  </Menu>
                </template>
              </Dropdown>
            </SettingsRow>
            <SettingsRow label="API key" description="Kept in this device's keychain.">
              <TextInput placeholder="sk-…" class="w-56 max-w-full" />
            </SettingsRow>
          </div>
          <div class="flex items-center justify-end gap-2">
            <Button variant="ghost" @click="draft = null; s.providerDetailOpen = false">Cancel</Button>
            <Button variant="primary" :disabled="!draft.name.trim() || !draft.baseUrl.trim()">Add provider</Button>
          </div>
        </div>

        <!-- Existing provider -->
        <div v-else-if="selected" class="flex flex-col gap-6 p-5">
          <header class="flex flex-wrap items-center gap-x-3 gap-y-2">
            <div class="flex min-w-0 flex-1 flex-wrap items-center gap-2">
              <h3 class="max-w-full truncate text-[15px] font-medium text-fg-emphasis">{{ selected.name }}</h3>
              <Tag :tone="stateTone[selected.state]">{{ stateWord[selected.state] }}</Tag>
              <Tag v-if="selected.kind === 'subscription'">Subscription</Tag>
            </div>
            <div class="ml-auto flex items-center gap-2">
              <Button variant="ghost" size="sm">Rename</Button>
              <Button variant="ghost" size="sm">Remove</Button>
              <Switch v-model="selected.enabled" size="sm" class="ml-1" />
            </div>
          </header>
          <p v-if="selected.detail" class="-mt-3 select-none text-[12px]" :class="selected.state === 'unreachable' ? 'font-mono text-on-danger' : 'text-on-danger'">{{ selected.detail }}</p>

          <!-- Accounts (subscription) -->
          <SettingsGroup v-if="selected.kind === 'subscription'" title="Accounts" description="One account is active at a time; every conversation on this provider uses it.">
            <SettingsRow
              v-for="account in selected.accounts"
              :key="account.id"
              :label="account.label"
              :description="account.quota ? `${account.plan} · ${account.quota.used}% of the 5-hour window · resets ${account.quota.resets}` : account.plan"
            >
              <template #tags><Tag v-if="account.active" tone="accent">Active</Tag><Tag v-if="account.quota && account.quota.used >= 100" tone="danger">Limit reached</Tag></template>
              <Button v-if="!account.active" variant="ghost" size="sm" @click="setActive(selected!, account.id)">Use</Button>
              <Button variant="ghost" size="sm">Sign out</Button>
            </SettingsRow>
            <div v-for="account in selected.accounts.filter((a) => a.active && a.quota)" :key="`m-${account.id}`" class="px-4 pb-3">
              <Meter :value="account.quota!.used" :max="account.quota!.max" label="Rate window" />
            </div>
            <div v-if="selected.login" class="flex flex-col gap-3 px-4 py-4">
              <div class="flex items-center gap-2 text-chrome text-fg">
                <IndeterminateSpinner :size="ICON_PX.in24" />
                Waiting for you to confirm in the browser
              </div>
              <div class="flex flex-wrap items-center gap-3">
                <span class="font-mono text-[18px] tracking-[0.2em] text-fg-emphasis">{{ selected.login.code }}</span>
                <IconButton :icon="Copy" variant="ghost" aria-label="Copy code" />
                <Button size="sm">
                  Open {{ selected.login.url.replace('https://', '') }}
                  <ExternalLink :size="ICON_PX.in24" />
                </Button>
                <span class="text-[12px] text-fg-subtle">Code expires in {{ selected.login.expires }}</span>
              </div>
              <Button variant="ghost" size="sm" class="self-start" @click="selected!.login = null">Cancel</Button>
            </div>
            <SettingsRow v-else label="Add an account" description="Signs in with the vendor's own login. Also imports a login the CLI already has.">
              <Button variant="ghost" size="sm">Import from CLI</Button>
              <Button @click="selected!.login = { url: 'https://auth.openai.com/codex/device', code: 'HXRV-7K2M', expires: '10 min' }">Sign in…</Button>
            </SettingsRow>
          </SettingsGroup>

          <!-- Connection (API key) -->
          <SettingsGroup v-else title="Connection">
            <SettingsRow label="Base URL" :description="selected.vendorId ? `${selected.vendorId} on models.dev · protocol comes with the vendor` : 'A custom endpoint. Protocol below.'">
              <TextInput v-model="selected.baseUrl" class="w-72 max-w-full" />
            </SettingsRow>
            <SettingsRow v-if="!selected.vendorId" label="Protocol">
              <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Protocol">
                <template #trigger>{{ wireOptions.find((w) => w.value === selected?.wireApi)?.label }}</template>
                <template #content="{ close }">
                  <Menu>
                    <MenuItem v-for="w in wireOptions" :key="w.value" :label="w.label" choice :is-selected="selected.wireApi === w.value" @select="selected!.wireApi = w.value; close()" />
                  </Menu>
                </template>
              </Dropdown>
            </SettingsRow>
            <SettingsRow label="API key" :description="selected.keyHint ? 'Stored in the keychain of this device, never in the config file.' : 'No key. Local endpoints usually need none.'">
              <span v-if="selected.keyHint" class="font-mono text-[12px] text-fg-muted">{{ selected.keyHint }}</span>
              <Button variant="ghost" size="sm">{{ selected.keyHint ? 'Replace' : 'Add key' }}</Button>
            </SettingsRow>
            <SettingsRow label="Test connection" description="Sends one tiny request with the key above.">
              <span v-if="testing === selected.id" class="flex items-center gap-1.5 text-[12px] text-fg-subtle"><IndeterminateSpinner :size="ICON_PX.in24" /> Testing…</span>
              <span v-else-if="selected.state === 'ready'" class="flex items-center gap-1 text-[12px] text-on-success"><Check :size="ICON_PX.in24" /> OK · 412 ms</span>
              <Button variant="ghost" size="sm" @click="test(selected!)">Test</Button>
            </SettingsRow>
          </SettingsGroup>

          <!-- Models -->
          <SettingsGroup title="Models" :description="selected.kind === 'subscription' ? 'What the vendor offers this account. Off hides a model from the picker.' : undefined">
            <SettingsRow
              v-if="selected.kind === 'api_key'"
              label="Model list"
              :description="selected.modelSource === 'catalog' ? `From the vendor catalog · fetched ${selected.catalogFetched}` : 'Ids you enter. Fill in what a model can do so the composer offers the right controls.'"
            >
              <template #tags><Tag v-if="selected.stale" tone="warning">Stale</Tag></template>
              <Button v-if="selected.modelSource === 'catalog'" variant="ghost" size="sm"><RefreshCw :size="ICON_PX.in24" /> Refresh</Button>
              <Segmented v-model="selected.modelSource" size="sm" :options="[{ value: 'catalog', label: 'Catalog' }, { value: 'manual', label: 'Manual' }]" />
            </SettingsRow>
            <div v-if="selected.models.length > 3" class="flex items-center gap-2 px-4 py-2">
              <Search :size="ICON_PX.in24" class="shrink-0 text-fg-subtle" />
              <TextInput v-model="modelFilter" placeholder="Filter models" class="w-56 max-w-full" />
            </div>
            <template v-for="m in visibleModels(selected)" :key="m.id">
              <SettingsRow :label="m.name || m.id" :description="m.name ? m.id : undefined" :class="m.enabled ? '' : 'opacity-60'">
                <template #leading><Checkbox v-model="m.enabled" label="" /></template>
                <template #tags>
                  <Tag v-if="selected.defaultModelId === m.id" tone="accent">Default</Tag>
                  <Tag v-for="t in capabilityTags(m)" :key="t.text" :tone="t.tone">{{ t.text }}</Tag>
                </template>
                <Button v-if="selected.defaultModelId !== m.id" variant="ghost" size="sm" @click="selected!.defaultModelId = m.id">Make default</Button>
                <Button v-if="selected.modelSource === 'manual'" variant="ghost" size="sm" @click="expandedModelId = expandedModelId === m.id ? null : m.id">{{ expandedModelId === m.id ? 'Done' : 'Edit' }}</Button>
                <Button v-else variant="ghost" size="sm" @click="expandedModelId = expandedModelId === m.id ? null : m.id">{{ expandedModelId === m.id ? 'Less' : 'Details' }}</Button>
              </SettingsRow>
              <template v-if="expandedModelId === m.id && selected.modelSource === 'manual'">
                <SettingsRow inset label="Display name" description="Shown in the picker instead of the id.">
                  <TextInput v-model="m.name" :placeholder="m.id" class="w-56 max-w-full" />
                </SettingsRow>
                <SettingsRow inset label="Context window" description="Tokens. Compaction triggers near this.">
                  <TextInput :model-value="m.contextWindow === null ? '' : String(m.contextWindow)" placeholder="128000" class="w-32" @update:model-value="(v) => (m.contextWindow = v ? Number(v) : null)" />
                </SettingsRow>
                <SettingsRow inset label="Max output" description="Tokens per reply. Leave empty to use the endpoint's default.">
                  <TextInput :model-value="m.outputLimit === null ? '' : String(m.outputLimit)" placeholder="8192" class="w-32" @update:model-value="(v) => (m.outputLimit = v ? Number(v) : null)" />
                </SettingsRow>
                <SettingsRow inset label="Tool calling" description="Off keeps the agent from offering it tools.">
                  <Switch :model-value="m.tools === true" size="sm" @update:model-value="(v) => (m.tools = v)" />
                </SettingsRow>
                <SettingsRow inset label="Images and files" description="Attachments in the composer.">
                  <Switch :model-value="m.attachments === true" size="sm" @update:model-value="(v) => (m.attachments = v)" />
                </SettingsRow>
                <SettingsRow inset label="Reasoning efforts" description="Which levels the composer offers. None means no reasoning control.">
                  <button
                    v-for="effort in EFFORTS"
                    :key="effort"
                    type="button"
                    class="h-6 cursor-default select-none rounded-md px-2 text-[12px] transition-colors duration-200 ease-out"
                    :class="m.efforts.includes(effort) ? 'bg-tint-accent text-on-accent' : 'bg-hover text-fg-muted hover:text-fg'"
                    @click="toggleEffort(m, effort)"
                  >
                    {{ effort }}
                  </button>
                </SettingsRow>
                <SettingsRow v-if="m.efforts.length" inset label="Default effort">
                  <Segmented :model-value="m.defaultEffort ?? m.efforts[0]!" size="sm" :options="m.efforts.map((e) => ({ value: e, label: e }))" @update:model-value="(v) => (m.defaultEffort = v)" />
                </SettingsRow>
                <SettingsRow inset label="Fast tier" description="A service tier id the Fast switch selects, if the endpoint has one.">
                  <TextInput :model-value="m.fastTier ?? ''" placeholder="priority" class="w-32" @update:model-value="(v) => (m.fastTier = v || null)" />
                </SettingsRow>
                <SettingsRow inset label="Remove this model">
                  <Button variant="ghost" size="sm" @click="selected!.models = selected!.models.filter((x) => x.id !== m.id); expandedModelId = null">Remove</Button>
                </SettingsRow>
              </template>
              <template v-else-if="expandedModelId === m.id">
                <SettingsRow inset label="Context · output"><span class="text-[12px] tabular-nums text-fg-muted">{{ formatTokens(m.contextWindow) }} · {{ formatTokens(m.outputLimit) }}</span></SettingsRow>
                <SettingsRow inset label="Reasoning"><span class="text-[12px] text-fg-muted">{{ m.efforts.length ? `${m.efforts.join(' · ')} · default ${m.defaultEffort}` : 'None' }}</span></SettingsRow>
                <SettingsRow v-if="m.fastTier" inset label="Fast tier"><span class="font-mono text-[12px] text-fg-muted">{{ m.fastTier }}</span></SettingsRow>
              </template>
            </template>
            <div v-if="!visibleModels(selected).length" class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle">
              {{ modelFilter ? 'No model matches.' : 'No models yet.' }}
            </div>
            <SettingsRow v-if="selected.kind === 'api_key' && selected.modelSource === 'manual'" label="Add a model" description="The id the endpoint expects. You can fill in its capabilities after.">
              <Button variant="ghost" size="sm"><Sparkles :size="ICON_PX.in24" /> Fetch from endpoint</Button>
              <TextInput v-model="newModelId" placeholder="model-id" class="w-48 max-w-full" @keydown.enter="addModel(selected!)" />
              <Button :disabled="!newModelId.trim()" @click="addModel(selected!)">Add</Button>
            </SettingsRow>
          </SettingsGroup>
        </div>
      </template>
    </SettingsSplit>

    <SettingsGroup title="Defaults" description="New conversations start here. Each conversation can still switch.">
      <SettingsRow label="Chat model" description="The primary agent's model.">
        <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Chat model">
          <template #trigger>{{ s.defaults.chat }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="m in ['Claude Sonnet 4.5', 'Claude Opus 4.1', 'GPT-5', 'Kimi K2 Thinking']" :key="m" :label="m" choice :is-selected="s.defaults.chat === m" @select="s.defaults.chat = m; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Fast model" description="Titles, summaries and quick lookups.">
        <Dropdown :overlay-store="appOverlayStore" variant="default" trigger-label="Fast model">
          <template #trigger>{{ s.defaults.fast }}</template>
          <template #content="{ close }">
            <Menu>
              <MenuItem v-for="m in ['Claude Haiku 4.5', 'GPT-5 mini']" :key="m" :label="m" choice :is-selected="s.defaults.fast === m" @select="s.defaults.fast = m; close()" />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow label="Reasoning" description="Default thinking effort for models that support it.">
        <Segmented v-model="s.defaults.reasoning" :options="[{ value: 'off', label: 'Off' }, { value: 'low', label: 'Low' }, { value: 'medium', label: 'Medium' }, { value: 'high', label: 'High' }]" />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>

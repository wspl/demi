<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import {
  Brain,
  Check,
  Image,
  Info,
  Plug,
  Plus,
  RefreshCw,
  Search,
  SlidersHorizontal,
  TextCursorInput,
  Trash2,
  TriangleAlert,
  Zap,
} from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import Meter from '@demicodes/web-ui/ui/Meter.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import AddProviderDialog from './AddProviderDialog.vue'
import ModelDialog from './ModelDialog.vue'
import SettingsGroup from './SettingsGroup.vue'
import SettingsListItem from './SettingsListItem.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSplit from './SettingsSplit.vue'
import {
  WIRE_API_LABELS,
  type SettingsModelDraft,
  type SettingsProviderEntry,
  type SettingsProviderModel,
  type SettingsVendor,
  type SettingsWireApi,
} from './types'

/**
 * Models & providers: a rail of providers beside the selected one. Every supported
 * subscription is always listed and manages its accounts; an API key is added from
 * the rail and edits its endpoint, key and model list on its page. Field edits land
 * on the entry itself; everything that needs the host is emitted.
 */
const props = defineProps<{
  saveModel: (
    provider: SettingsProviderEntry,
    draft: SettingsModelDraft,
    original: SettingsProviderModel | null,
  ) => Promise<void>
  providers: SettingsProviderEntry[]
  vendors: SettingsVendor[]
  overlayStore: OverlayStore
  /** The provider whose connection test is running. */
  testing?: string | null
  /** The provider whose model list is being fetched. */
  refreshing?: string | null
}>()

const emit = defineEmits<{
  change: [provider: SettingsProviderEntry, patch: Partial<SettingsProviderEntry>]
  toggleModel: [
    provider: SettingsProviderEntry,
    model: SettingsProviderModel,
    enabled: boolean,
  ]
  add: [vendor: SettingsVendor]
  addEndpoint: [wireApi: SettingsWireApi]
  remove: [id: string]
  signIn: [provider: SettingsProviderEntry]
  test: [provider: SettingsProviderEntry]
  refresh: [provider: SettingsProviderEntry]
  activateAccount: [provider: SettingsProviderEntry, accountId: string]
  removeAccount: [provider: SettingsProviderEntry, accountId: string]
  removeModel: [provider: SettingsProviderEntry, model: SettingsProviderModel]
}>()

const selectedId = defineModel<string | null>('selectedId', { default: null })
const detailOpen = defineModel<boolean>('detailOpen', { default: false })

const subscriptions = computed(() =>
  props.providers.filter((p) => p.kind === 'subscription'),
)
const apiKeys = computed(() => props.providers.filter((p) => p.kind === 'api_key'))
const selected = computed(
  () => props.providers.find((p) => p.id === selectedId.value) ?? null,
)
const splitOpen = computed({
  get: () => detailOpen.value && selected.value !== null,
  set: (value: boolean) => {
    detailOpen.value = value
  },
})

// Match the visible rail order, including catalogs that arrive after mount.
watch(
  () => JSON.stringify([
    subscriptions.value.map((provider) => provider.id),
    apiKeys.value.map((provider) => provider.id),
    selectedId.value,
  ]),
  () => {
    if (selected.value) {
      return
    }
    const first = subscriptions.value[0] ?? apiKeys.value[0]
    selectedId.value = first?.id ?? null
    detailOpen.value = !!first
  },
  { immediate: true },
)
onMounted(() => {
  if (selected.value) {
    detailOpen.value = true
  }
})

/**
 * Rail dots. A subscription is always listed, so its dot says whether it is set up:
 * none until an account exists, green once one works, red when it stopped working.
 * An API key is listed because it was added, so it is dotted only when it needs
 * attention: yellow until configured, red when its test failed.
 */
const subscriptionBadge = {
  ready: 'success',
  unconfigured: undefined,
  error: 'danger',
  unreachable: 'danger',
  'signed-out': undefined,
  disabled: undefined,
} as const
const apiKeyBadge = {
  ready: undefined,
  unconfigured: 'warning',
  error: 'danger',
  unreachable: 'danger',
  'signed-out': 'warning',
  disabled: undefined,
} as const
const wireOptions = (Object.keys(WIRE_API_LABELS) as SettingsWireApi[]).map(
  (value) => ({
    value,
    label: WIRE_API_LABELS[value],
  }),
)

function select(id: string) {
  selectedId.value = id
  detailOpen.value = true
}

// Adding happens in its own dialog; the host appends the entry and selects it.
const addOpen = ref(false)

// The name edits in place: Enter or blur commits, Escape cancels.
const renaming = ref(false)
const renameDraft = ref('')
const renameInput = ref<InstanceType<typeof TextInput>>()

function beginRename() {
  if (!selected.value) {
    return
  }
  renameDraft.value = selected.value.name
  renaming.value = true
  void nextTick(() => {
    renameInput.value?.focus()
    renameInput.value?.select()
  })
}

function commitRename() {
  if (!renaming.value) {
    return
  }
  renaming.value = false
  const name = renameDraft.value.trim()
  if (name && selected.value) {
    emit('change', selected.value, { name })
  }
}

const modelFilter = ref('')
const cap = (word: string) => word.charAt(0).toUpperCase() + word.slice(1)

function formatTokens(n: number | null): string {
  if (n === null) {
    return '—'
  }
  return n >= 1_000_000 ? `${n / 1_000_000}M` : `${Math.round(n / 1000)}K`
}

const isUnknown = (m: SettingsProviderModel) => m.contextWindow === null

function visibleModels(p: SettingsProviderEntry): SettingsProviderModel[] {
  const q = modelFilter.value.trim().toLowerCase()
  return q
    ? p.models.filter((m) => m.id.includes(q) || m.name.toLowerCase().includes(q))
    : p.models
}

/** The model dialog: create or edit a manual model, or view a catalog one. */
const modelDialog = ref<{
  mode: 'create' | 'edit' | 'view'
  model: SettingsModelDraft
  original: SettingsProviderModel | null
} | null>(null)
const modelDialogOpen = ref(false)

function toDraft(m: SettingsProviderModel): SettingsModelDraft {
  return {
    id: m.id,
    name: m.name,
    contextWindow: m.contextWindow,
    outputLimit: m.outputLimit,
    efforts: [...m.efforts],
    extensions: m.extensions === null ? null : [...m.extensions],
    fastTier: m.fastTier,
  }
}

function openModel(
  mode: 'create' | 'edit' | 'view',
  m: SettingsProviderModel | null,
) {
  modelError.value = null
  modelDialogOpen.value = true
  modelDialog.value = {
    mode,
    original: m,
    model: m
      ? toDraft(m)
      : {
          id: '',
          name: '',
          contextWindow: null,
          outputLimit: null,
          efforts: [],
          extensions: [],
          fastTier: null,
        },
  }
}

const modelSave = ref<'idle' | 'pending'>('idle')
const modelError = ref<string | null>(null)
async function saveModel(draft: SettingsModelDraft) {
  if (!selected.value || !modelDialog.value || modelSave.value === 'pending') {
    return
  }
  modelSave.value = 'pending'
  modelError.value = null
  try {
    await props.saveModel(selected.value, draft, modelDialog.value.original)
    modelDialogOpen.value = false
  } catch (error) {
    modelError.value = error instanceof Error ? error.message : String(error)
  } finally {
    modelSave.value = 'idle'
  }
}
function selectWire(wireApi: SettingsWireApi, close: () => void): void {
  if (selected.value) {
    emit('change', selected.value, { wireApi })
  }
  close()
}
</script>

<template>
  <SettingsPage
    wide
    fill
    title="Models & providers"
  >
    <SettingsSplit
      v-model:detail-open="splitOpen"
      :detail-title="selected?.name"
    >
      <template #list>
        <div
          class="select-none px-1 pb-1 pt-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
        >
          Subscriptions
        </div>
        <SettingsListItem
          v-for="p in subscriptions"
          :key="p.id"
          :label="p.name"
          :selected="p.id === selectedId"
          :badge="subscriptionBadge[p.state]"
          :muted="!p.enabled"
          @select="select(p.id)"
        >
          <template #leading
            ><VendorMark
              :label="p.name"
              :src="p.logo"
              size="sm"
          /></template>
        </SettingsListItem>
        <!-- Adding belongs to this group alone, so its button sits in the group's caption. -->
        <div class="flex h-8 select-none items-center pl-1 pt-2">
          <span
            class="text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
            >API keys</span
          >
          <Button
            size="xs"
            class="ml-auto"
            @click="addOpen = true"
            ><Plus :size="12" /> Add</Button
          >
        </div>
        <SettingsListItem
          v-for="p in apiKeys"
          :key="p.id"
          :label="p.name"
          :selected="p.id === selectedId"
          :badge="apiKeyBadge[p.state]"
          :muted="!p.enabled"
          removable
          @select="select(p.id)"
          @remove="emit('remove', p.id)"
        >
          <template #leading
            ><VendorMark
              :label="p.name"
              :src="p.logo"
              size="sm"
          /></template>
        </SettingsListItem>
      </template>

      <template #detail>
        <div
          v-if="selected"
          class="flex flex-col gap-6"
        >
          <!-- The rail carries the mark; the first card's header names the provider and holds its
               actions. An API key can be renamed and removed; a subscription is a fixture. -->
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
                    <h3
                      class="min-w-0 truncate text-[15px] font-medium text-fg-emphasis"
                    >
                      {{ selected.name }}
                    </h3>
                    <Tooltip
                      v-if="selected.kind === 'api_key'"
                      content="Rename"
                      ><IconButton
                        size="sm"
                        :icon="TextCursorInput"
                        aria-label="Rename provider"
                        @click="beginRename"
                    /></Tooltip>
                  </template>
                  <div class="ml-auto flex items-center gap-1.5">
                    <Switch
                      :model-value="selected.enabled"
                      @update:model-value="
                        emit('change', selected, { enabled: $event })
                      "
                      size="sm"
                      class="ml-2"
                    />
                  </div>
                </div>
              </header>
            </template>

            <!-- Accounts (subscription) -->
            <template v-if="selected.kind === 'subscription'">
              <SettingsRow
                v-for="account in selected.accounts"
                :key="account.id"
                :label="account.label"
                compact
              >
                <template #tags>
                  <Tag>{{ account.plan }}</Tag>
                  <Tag
                    v-if="account.active"
                    tone="accent"
                    >Active</Tag
                  >
                  <Tag
                    v-if="account.quota.some((window) => window.used >= window.max)"
                    tone="danger"
                    >Limit reached</Tag
                  >
                </template>
                <template
                  v-if="account.quota.length"
                  #description
                >
                  <div
                    class="mt-1 grid max-w-72 grid-cols-[3.25rem_minmax(0,1fr)_auto] items-center gap-x-2 gap-y-1 text-[11px] tabular-nums"
                  >
                    <template
                      v-for="window in account.quota"
                      :key="window.id"
                    >
                      <span>{{ window.label }}</span>
                      <Meter
                        :value="window.used"
                        :max="window.max"
                        :label="window.label"
                      />
                      <span
                        >{{ window.used }}%<template v-if="window.resets">
                          · resets {{ window.resets }}</template
                        ></span
                      >
                    </template>
                  </div>
                </template>
                <Button
                  size="sm"
                  v-if="!account.active"
                  @click="emit('activateAccount', selected, account.id)"
                  >Activate</Button
                >
                <Tooltip content="Remove account"
                  ><IconButton
                    size="sm"
                    :icon="Trash2"
                    variant="danger"
                    aria-label="Remove account"
                    @click="emit('removeAccount', selected, account.id)"
                /></Tooltip>
              </SettingsRow>
              <div
                v-if="!selected.accounts.length"
                class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
              >
                No account yet. Sign in to use this provider.
              </div>
              <SettingsRow label="Add account">
                <Button
                  size="sm"
                  variant="primary"
                  @click="emit('signIn', selected)"
                  >Sign in</Button
                >
              </SettingsRow>
            </template>

            <!-- Connection (API key) -->
            <template v-else>
              <SettingsRow label="Base URL">
                <CommitTextInput
                  :model-value="selected.baseUrl"
                  aria-label="Base URL"
                  class="w-72 max-w-full"
                  @commit="emit('change', selected, { baseUrl: $event })"
                />
              </SettingsRow>
              <SettingsRow
                v-if="!selected.vendorId"
                label="Protocol"
              >
                <Dropdown
                  size="sm"
                  :overlay-store="overlayStore"
                  variant="default"
                  trigger-label="Protocol"
                  :disabled="selected.configured !== false"
                >
                  <template #trigger>{{
                    WIRE_API_LABELS[selected.wireApi]
                  }}</template>
                  <template #content="{ close }">
                    <Menu>
                      <MenuItem
                        v-for="w in wireOptions"
                        :key="w.value"
                        :label="w.label"
                        choice
                        :is-selected="selected.wireApi === w.value"
                        @select="selectWire(w.value, close)"
                      />
                    </Menu>
                  </template>
                </Dropdown>
              </SettingsRow>
              <SettingsRow
                label="API key"
                :description="
                  selected.keyConfigured
                    ? 'Key saved. Enter a new key to replace it.'
                    : undefined
                "
              >
                <CommitTextInput
                  :model-value="selected.apiKey"
                  aria-label="API key"
                  @commit="emit('change', selected, { apiKey: $event })"
                  secret
                  placeholder="sk-…"
                  class="w-72 max-w-full"
                />
              </SettingsRow>
              <SettingsRow label="Test connection">
                <span
                  v-if="testing === selected.id"
                  class="flex items-center gap-1.5 text-[12px] text-fg-subtle"
                  ><IndeterminateSpinner :size="ICON_PX.in24" /> Testing…</span
                >
                <span
                  v-else-if="selected.testPassed || selected.testedIn"
                  class="flex items-center gap-1 text-[12px] text-on-success"
                  ><Check :size="ICON_PX.in24" /> OK<template
                    v-if="selected.testedIn"
                  >
                    · {{ selected.testedIn }}</template
                  ></span
                >
                <span
                  v-else-if="selected.detail"
                  class="min-w-0 truncate font-mono text-[12px] text-on-danger"
                  >{{ selected.detail }}</span
                >
                <Tooltip content="Test connection"
                  ><IconButton
                    size="sm"
                    :icon="Plug"
                    aria-label="Test connection"
                    :disabled="selected.configured === false"
                    @click="emit('test', selected)"
                /></Tooltip>
              </SettingsRow>
            </template>
          </SettingsGroup>

          <!-- Models. A subscription's list is whatever the vendor serves, so its only control is a
               refresh; a vendor key chooses between the catalog and a manual list; a bare endpoint
               only has the manual list. -->
          <SettingsGroup>
            <template #header>
              <header class="flex h-7 select-none items-center gap-2">
                <h3 class="text-[15px] font-medium leading-5 text-fg-emphasis">
                  Models
                </h3>
                <span
                  v-if="
                    selected.modelSource === 'catalog' && selected.catalogFetched
                  "
                  class="text-[12px] text-fg-subtle"
                  >fetched {{ selected.catalogFetched }}</span
                >
                <Tag
                  v-if="selected.stale"
                  tone="warning"
                  >Stale</Tag
                >
                <span
                  v-if="selected.kind === 'subscription'"
                  class="ml-auto"
                  ><Tooltip content="Refresh the list"
                    ><IconButton
                      size="sm"
                      :icon="RefreshCw"
                      spin-on-click
                      :spinning="refreshing === selected.id"
                      :disabled="selected.configured === false || refreshing === selected.id"
                      aria-label="Refresh models"
                      @click="emit('refresh', selected)" /></Tooltip
                ></span>
                <Segmented
                  size="sm"
                  v-else-if="selected.vendorId"
                  :model-value="selected.modelSource"
                  @update:model-value="
                    emit('change', selected, { modelSource: $event })
                  "
                  class="ml-auto"
                  :options="[
                    { value: 'catalog', label: 'Catalog' },
                    { value: 'manual', label: 'Manual' },
                  ]"
                />
              </header>
            </template>
            <!-- The filter is bare text; its hit area is the whole row height. -->
            <div
              v-if="selected.kind === 'api_key'"
              class="flex min-h-10 items-center gap-3 px-3 py-1"
            >
              <TextInput
                size="sm"
                v-model="modelFilter"
                placeholder="Filter models"
                bare
                class="min-w-0 flex-1"
              >
                <template #prefix><Search :size="ICON_PX.in24" /></template>
              </TextInput>
              <Tooltip
                v-if="selected.modelSource === 'manual'"
                content="Add model"
                ><IconButton
                  size="sm"
                  :icon="Plus"
                  aria-label="Add model"
                  @click="openModel('create', null)"
              /></Tooltip>
              <Tooltip
                v-else
                content="Refresh the catalog"
                ><IconButton
                  size="sm"
                  :icon="RefreshCw"
                  spin-on-click
                  :spinning="refreshing === selected.id"
                  :disabled="selected.configured === false || refreshing === selected.id"
                  aria-label="Refresh models"
                  @click="emit('refresh', selected)"
              /></Tooltip>
            </div>
            <SettingsRow
              v-for="m in visibleModels(selected)"
              :key="m.id"
              :label="m.name || m.id"
              compact
              :interactive="selected.kind === 'api_key'"
              isolate-controls
              :class="m.enabled ? '' : 'opacity-60'"
              @click="emit('toggleModel', selected, m, !m.enabled)"
            >
              <template #tags>
                <Tooltip
                  v-if="isUnknown(m)"
                  content="Capabilities unknown. Edit to fill them in."
                  ><Tag tone="warning"><TriangleAlert :size="12" /></Tag
                ></Tooltip>
                <Tag v-if="m.contextWindow !== null">{{
                  formatTokens(m.contextWindow)
                }}</Tag>
                <Tooltip
                  v-if="m.extensions?.length"
                  :content="`Accepts ${m.extensions?.join(' ')}`"
                  ><Tag><Image :size="12" /></Tag
                ></Tooltip>
                <Tooltip
                  v-if="m.efforts.length"
                  :content="`Reasoning · ${m.efforts.map(cap).join(', ')}`"
                  ><Tag><Brain :size="12" /></Tag
                ></Tooltip>
                <Tooltip
                  v-if="m.fastTier"
                  content="Has a fast tier"
                  ><Tag><Zap :size="12" /></Tag
                ></Tooltip>
              </template>
              <template
                v-if="
                  selected.kind === 'api_key' && selected.modelSource === 'manual'
                "
              >
                <Tooltip content="Edit"
                  ><IconButton
                    size="sm"
                    :icon="SlidersHorizontal"
                    aria-label="Edit model"
                    @click="openModel('edit', m)"
                /></Tooltip>
                <Tooltip content="Remove"
                  ><IconButton
                    size="sm"
                    :icon="Trash2"
                    variant="danger"
                    aria-label="Remove model"
                    :disabled="
                      selected.configured !== false && selected.models.length === 1
                    "
                    @click="emit('removeModel', selected, m)"
                /></Tooltip>
              </template>
              <Tooltip
                v-else
                content="Details"
                ><IconButton
                  size="sm"
                  :icon="Info"
                  aria-label="Model details"
                  @click="openModel('view', m)"
              /></Tooltip>
              <Switch
                v-if="selected.kind === 'api_key'"
                :model-value="m.enabled"
                @update:model-value="emit('toggleModel', selected, m, $event)"
                size="sm"
                :aria-label="`Enable ${m.name || m.id}`"
              />
            </SettingsRow>
            <div
              v-if="!visibleModels(selected).length"
              class="select-none px-4 py-6 text-center text-[13px] text-fg-subtle"
            >
              {{
                modelFilter
                  ? 'No model matches.'
                  : selected.kind === 'subscription'
                    ? 'Sign in to see what this account can use.'
                    : 'No models yet.'
              }}
            </div>
          </SettingsGroup>
        </div>
      </template>
    </SettingsSplit>

    <AddProviderDialog
      :is-open="addOpen"
      :overlay-store="overlayStore"
      :vendors="vendors"
      @close="addOpen = false"
      @add="
        (v) => {
          addOpen = false
          emit('add', v)
        }
      "
      @add-endpoint="
        (w) => {
          addOpen = false
          emit('addEndpoint', w)
        }
      "
    />
    <ModelDialog
      v-if="modelDialog"
      :is-open="modelDialogOpen"
      :overlay-store="overlayStore"
      :mode="modelDialog.mode"
      :pending="modelSave === 'pending'"
      :error="modelError"
      :model="modelDialog.model"
      @close="modelDialogOpen = false"
      @save="saveModel"
    />
  </SettingsPage>
</template>

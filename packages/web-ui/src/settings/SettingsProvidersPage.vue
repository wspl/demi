<script setup lang="ts">
import { formError, type ShownError } from '../infra/errors'
import { computed, reactive, ref, watch, type Component } from 'vue'
import { effortLabel } from './efforts'
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { showToast } from '../infra/toast'
import {
  Brain,
  Check,
  GripVertical,
  Image,
  Hand,
  Info,
  ListOrdered,
  Plug,
  Plus,
  RefreshCw,
  Scale,
  Search,
  SlidersHorizontal,
  TextCursorInput,
  Trash2,
  TriangleAlert,
  Zap,
} from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import AsyncRegion from '../ui/AsyncRegion.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import IconButton from '@demicodes/web-ui/ui/IconButton.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import ProviderQuota from './ProviderQuota.vue'
import RelativeTime from '../ui/RelativeTime.vue'
import Segmented from '@demicodes/web-ui/ui/Segmented.vue'
import Switch from '@demicodes/web-ui/ui/Switch.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import CommitTextInput from '../ui/CommitTextInput.vue'
import TitleInput from '../ui/TitleInput.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import TruncatedText from '@demicodes/web-ui/ui/TruncatedText.vue'
import VendorMark from '@demicodes/web-ui/ui/VendorMark.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import Dropdown from '@demicodes/web-ui/ui/Dropdown.vue'
import Menu from '@demicodes/web-ui/ui/Menu.vue'
import MenuItem from '@demicodes/web-ui/ui/MenuItem.vue'
import AddModelDialog from './AddModelDialog.vue'
import AddProviderDialog from './AddProviderDialog.vue'
import ModelDialog from './ModelDialog.vue'
import { freeModelId } from './model-ids'
import { endpointProblem } from './provider-defaults'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import { operations, type Action, type Operation } from '../ui/operations'
import { answerInDialog } from './dialog-work'
import { providerOperation, providerTarget } from './provider-operations'
import SettingsGroup from './SettingsGroup.vue'
import SettingsListItem from './SettingsListItem.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import SettingsSplit from './SettingsSplit.vue'
import { useListReorder } from '../ui/useListReorder'
import {
  COMPATIBLE_WIRE_APIS,
  ROTATIONS,
  ROTATION_HINTS,
  ROTATION_LABELS,
  WIRE_API_LABELS,
  type SettingsModelDraft,
  type SettingsModelSearch,
  type SettingsQuotaRefresh,
  type SettingsModelEditor,
  type SettingsProviderEntry,
  type SettingsProviderOperation,
  type SettingsProviderModel,
  type SettingsRotation,
  type SettingsVendor,
} from './types'
import { formatTokens } from '../ui/token-count'
import type { TitleText } from '../ui/ui-text'

/**
 * Providers: a rail of providers beside the selected one. Every supported
 * subscription is always listed and manages its accounts; an API key is added from
 * the rail. A vendor entry shows its vendor's endpoint as plain text and lists the
 * vendor's catalog; a compatible API sets its protocol, its endpoint and, when its
 * server takes one, its key, and lists only the models the user adds (`product.md`
 * § Models & providers). Adding creates the entry at once, selected, on the page
 * every entry has, and what it still lacks is a plain wait, the rail's yellow dot,
 * an empty field and the words *No models yet*; red marks a failure only. Add Model
 * adds the model it is given at once and opens it in Edit Model, the dialog every
 * model of the list opens (`models.md` § Adding a model from models.dev). Removing
 * an entry, an account or a model asks first. Field edits land on the entry itself;
 * everything that needs the host is emitted. A subscription with several accounts
 * offers its rotation and orders its accounts by dragging; adding its second account
 * and choosing In Order or Balanced first warn, once, that several personal
 * subscriptions may be against the vendor's terms (`providers.md` § Account rotation).
 */
/** Each rotation's mark: the one by hand, the one down the list, the one weighed by room. */
const ROTATION_ICONS: Record<SettingsRotation, Component> = { manual: Hand, in_order: ListOrdered, balanced: Scale }

const props = defineProps<{
  /**
   * Saves a model of the entry's own list: the one `original` was, or a new
   * one, which joins the list as it is given, its id already free.
   */
  saveModel: (
    provider: SettingsProviderEntry,
    draft: SettingsModelDraft,
    original: SettingsProviderModel | null,
  ) => Promise<void>
  /** The saved providers, which the list shows. */
  providers: SettingsProviderEntry[]
  /** Searches models.dev for the Add Model dialog of the entry `providerId`, filled from that entry's vendor. */
  searchModels?: (providerId: string, query: string, signal: AbortSignal) => ReturnType<SettingsModelSearch>
  vendors: SettingsVendor[]
  load?: 'loading' | 'ready' | 'failed'
  vendorLoad?: 'loading' | 'ready' | 'failed'
  modelLoad?: 'loading' | 'ready' | 'failed'
  overlayStore: OverlayStore
  refreshingUsage?: Record<string, Record<string, SettingsQuotaRefresh>>
  /** The vendor's words for each account's last manual usage refresh that failed, by provider and account. */
  usageFailures?: Record<string, Record<string, ShownError>>
  /**
   * What each question's Remove carries out, as the dialog's button shows it
   * until the promise settles: an entry, one of its accounts, one of its models.
   */
  onRemove?: Action<[id: string]>
  onRemoveAccount?: Action<[provider: SettingsProviderEntry, accountId: string]>
  onRemoveModel?: Action<[provider: SettingsProviderEntry, model: SettingsProviderModel]>
  /** Installs the CLI on the user's Cloud again; Install works until the backend took it. */
  onInstallCli?: Action<[provider: SettingsProviderEntry]>
  /** Whether the user went on past the warning about several accounts, which then never shows again. */
  multiAccountWarned?: boolean
}>()

const emit = defineEmits<{
  retry: []
  retryVendors: []
  /** The model list's Retry: the catalog loads again, outside the provider's writes. */
  retryModels: []
  change: [
    provider: SettingsProviderEntry,
    patch: Partial<SettingsProviderEntry>,
  ]
  toggleModel: [
    provider: SettingsProviderEntry,
    model: SettingsProviderModel,
    enabled: boolean,
  ]
  add: [vendor: SettingsVendor]
  /** A compatible API, created at once with its defaults. */
  addCompatible: []
  signIn: [provider: SettingsProviderEntry]
  /** Test the provider; a subscription names the account to test with. */
  test: [provider: SettingsProviderEntry, accountId?: string]
  refresh: [provider: SettingsProviderEntry]
  /** Ask the vendor for one account's usage again. */
  refreshUsage: [provider: SettingsProviderEntry, accountId: string, automatic?: boolean]
  /** Read the CLI's newest version and the machines again. */
  checkCli: [provider: SettingsProviderEntry]
  activateAccount: [provider: SettingsProviderEntry, accountId: string]
  /** The accounts in the order the user dragged them to, every one once. */
  reorderAccounts: [provider: SettingsProviderEntry, accountIds: string[]]
  /** The user went on past the warning about several accounts. */
  multiAccountWarned: []
}>()

const selectedId = defineModel<string | null>('selectedId', { default: null })
const detailOpen = defineModel<boolean>('detailOpen', { default: false })

const subscriptions = computed(() =>
  props.providers.filter((p) => p.kind === 'subscription'),
)
const apiKeys = computed(() =>
  props.providers.filter((p) => p.kind === 'api_key'),
)
const selected = computed(
  () => props.providers.find((p) => p.id === selectedId.value) ?? null,
)
const splitOpen = computed({
  get: () => detailOpen.value && selected.value !== null,
  set: (value: boolean) => {
    detailOpen.value = value
  },
})

// Match the visible rail order, including catalogs that arrive after mount. Only the
// selection follows: a narrow page opens on its list, never straight into a detail.
watch(
  () =>
    JSON.stringify([
      subscriptions.value.map((provider) => provider.id),
      apiKeys.value.map((provider) => provider.id),
      selectedId.value,
      props.load,
    ]),
  () => {
    if (props.load && props.load !== 'ready') {
      return
    }
    if (selected.value) {
      return
    }
    const first = subscriptions.value[0] ?? apiKeys.value[0]
    selectedId.value = first?.id ?? null
  },
  { immediate: true },
)

/**
 * Rail dots say what needs attention, for every kind of provider alike: none
 * while it works, the sidebar's needs-you yellow while it is not set up (no
 * account, no key, no model), the warning yellow while it is signed out, red
 * when it failed or cannot be reached. A working provider is the normal case
 * and carries no mark.
 */
const providerBadge = {
  ready: undefined,
  setup: 'attention',
  'signed-out': 'warning',
  error: 'danger',
  unreachable: 'danger',
  disabled: undefined,
} as const
/** What each dot says when the pointer is over the mark it sits on. */
const providerBadgeLabel = {
  ready: undefined,
  get setup() {
    return t('providers.state.setup')
  },
  get 'signed-out'() {
    return t('providers.state.signedOut')
  },
  get error() {
    return t('providers.state.error')
  },
  get unreachable() {
    return t('providers.state.unreachable')
  },
  disabled: undefined,
} as const

/**
 * Editing waits only for a write: a test or a refresh only reads, so the
 * connection, the model source and the manual models stay editable during
 * one, and the host saves an edit once it ends.
 */
function fieldsLocked(provider: SettingsProviderEntry): boolean {
  return runs(provider.id, { kind: 'saving' }) || runs(provider.id, { kind: 'removing' })
}

/** Whether the provider's operation runs (`operations.ts`, `provider-operations.ts`). */
function runs(id: string, operation: SettingsProviderOperation): boolean {
  return operations.runs(providerOperation(id, operation))
}

/** Whether the provider does something now: one thing at a time, so its other operations wait. */
function busy(id: string): boolean {
  return operations.blocked(providerOperation(id, { kind: 'saving' }))
}

/**
 * The question before something the user set up goes (`ConfirmDialog`):
 * its title, what it says, and what runs on the answer.
 */
const confirming = ref<{
  title: string
  action: TitleText
  body: string[]
  /** What the answer does, to what: the question shows it working, asked again included. */
  operation: Operation
  run: () => unknown
} | null>(null)
const confirmOpen = ref(false)

function ask(question: NonNullable<typeof confirming.value>): void {
  confirming.value = question
  confirmOpen.value = true
}

/** The answer: the question closes once its work is done, if it still asks the same of the same thing. */
async function confirm(): Promise<void> {
  const question = confirming.value
  if (!question) {
    return
  }
  try {
    await question.run()
  } finally {
    const shown = confirming.value
    if (shown?.operation.action === question.operation.action && shown.operation.target === question.operation.target) {
      confirmOpen.value = false
    }
  }
}

function askRemoveProvider(provider: SettingsProviderEntry): void {
  ask({
    title: t('providers.removeProvider.title', { name: provider.name }),
    action: uiText('common.remove'),
    body: [
      provider.models.length === 0
        ? t('providers.removeProvider.keyOnly')
        : t('providers.removeProvider.keyAndModels', { count: provider.models.length }, provider.models.length),
      t('providers.removeProvider.history'),
    ],
    operation: providerOperation(provider.id, { kind: 'removing' }),
    run: () => props.onRemove?.(provider.id),
  })
}

/**
 * Removing the active account switches the provider to its first other account in the list,
 * as the backend does, or leaves it with none (`providers.md` § Login and publication); the
 * question says which.
 */
function askRemoveAccount(
  provider: SettingsProviderEntry,
  account: { id: string; label: string; active: boolean },
): void {
  const next = provider.accounts.find((other) => other.id !== account.id)
  const effect = !account.active
    ? t('providers.removeAccount.keeps', { name: provider.name })
    : next
      ? t('providers.removeAccount.switches', { name: provider.name, next: next.label })
      : t('providers.removeAccount.none', { name: provider.name })
  ask({
    title: t('providers.removeAccount.title', { label: account.label }),
    action: uiText('common.remove'),
    body: [effect, t('providers.removeAccount.forgets')],
    operation: providerOperation(provider.id, { kind: 'account', accountId: account.id, action: 'remove' }),
    run: () => props.onRemoveAccount?.(provider, account.id),
  })
}

// The warning about several accounts (`providers.md` § Account rotation): what
// Continue goes on with, which Cancel drops.
const warning = ref<(() => void) | null>(null)
const warningOpen = ref(false)

function afterWarning(then: () => void): void {
  if (props.multiAccountWarned) {
    then()
    return
  }
  warning.value = then
  warningOpen.value = true
}

function continuePastWarning(): void {
  const then = warning.value
  warningOpen.value = false
  warning.value = null
  emit('multiAccountWarned')
  then?.()
}

function addAccount(provider: SettingsProviderEntry): void {
  if (provider.accounts.length === 0) {
    emit('signIn', provider)
    return
  }
  afterWarning(() => emit('signIn', provider))
}

function chooseRotation(provider: SettingsProviderEntry, rotation: SettingsRotation): void {
  if (rotation === (provider.rotation ?? 'manual')) {
    return
  }
  if (rotation === 'manual') {
    emit('change', provider, { rotation })
    return
  }
  afterWarning(() => emit('change', provider, { rotation }))
}

const accountList = ref<HTMLElement>()
const accountOrder = useListReorder(
  accountList,
  () => selected.value?.accounts.map((account) => account.id) ?? [],
  (ids) => {
    if (selected.value) {
      emit('reorderAccounts', selected.value, ids)
    }
  },
)

function askRemoveModel(provider: SettingsProviderEntry, model: SettingsProviderModel): void {
  ask({
    title: t('providers.removeModel.title', { name: model.name || model.id }),
    action: uiText('common.remove'),
    body: [t('providers.removeModel.body', { provider: provider.name })],
    // Of its own group: the list's save waits for what the provider does, rather than being refused.
    operation: { action: `removeModel:${model.id}`, target: providerTarget(provider.id) },
    run: async () => {
      await props.onRemoveModel?.(provider, model)
      // Delete Model asked from Edit Model: the model is gone, and its dialog with it.
      const editor = modelEditor.value
      if (editor?.providerId === provider.id && editor.original?.id === model.id) {
        editor.open = false
      }
    },
  })
}

function select(id: string) {
  selectedId.value = id
  detailOpen.value = true
}

// Adding happens in its own dialog; the host appends the entry and selects it.
const addOpen = ref(false)

// The name edits in place (`TitleInput`).
const renaming = ref(false)

function rename(name: string) {
  renaming.value = false
  if (selected.value) {
    emit('change', selected.value, { name })
  }
}

function isTesting(provider: SettingsProviderEntry): boolean {
  return runs(provider.id, { kind: 'testing' })
}

// An account's test ends in a toast: its button has no row to show the answer in.
const testingAccounts = computed(() =>
  props.providers.flatMap((provider) =>
    (provider.accounts ?? [])
      .filter((account) => accountPending(provider.id, account.id, 'test'))
      .map((account) => ({ providerId: provider.id, accountId: account.id })),
  ),
)
watch(testingAccounts, (now, before) => {
  for (const tested of before) {
    if (now.some((candidate) => candidate.providerId === tested.providerId && candidate.accountId === tested.accountId)) {
      continue
    }
    const provider = props.providers.find((candidate) => candidate.id === tested.providerId)
    const account = provider?.accounts?.find((candidate) => candidate.id === tested.accountId)
    if (!provider || !account) {
      continue
    }
    const model = provider.testedWith ? ` · ${provider.testedWith}` : ''
    // A test that failed left the provider's own words; one that passed left none.
    if (!provider.detail) {
      showToast({ title: `${t('providers.connected', { account: account.label })}${model}`, tone: 'success' })
    } else {
      showToast({ title: `${t('providers.testFailed', { account: account.label })}${model}`, message: provider.detail, tone: 'danger' })
    }
  }
})

// Why the selected provider cannot be tested right now, for the button's tooltip.
const testBlockReason = computed(() => {
  return selected.value?.models.some((model) => model.enabled)
    ? undefined
    : t('providers.connection.enableModel')
})

const modelFilter = ref('')

const isUnknown = (m: SettingsProviderModel) => m.contextWindow === null

function visibleModels(p: SettingsProviderEntry): SettingsProviderModel[] {
  const q = modelFilter.value.trim().toLowerCase()
  return q
    ? p.models.filter(
        (m) => m.id.includes(q) || m.name.toLowerCase().includes(q),
      )
    : p.models
}

/**
 * The model dialog: Edit Model for a model of the entry's own list, or a
 * catalog model's facts. A host that does not keep it leaves it to the
 * model's local value, which holds what it is given as it is, so each editor
 * is made reactive: closing it is a change of the object.
 */
const modelEditor = defineModel<SettingsModelEditor | null>('modelEditor', {
  default: null,
})

/** The ids Edit Model's model may not take: those of the other models of its entry's list. */
const takenIds = computed(() => {
  const editor = modelEditor.value
  const provider = props.providers.find((entry) => entry.id === editor?.providerId)
  return provider?.models.map((m) => m.id).filter((id) => id !== editor?.original?.id) ?? []
})

/** Add Model, a search over models.dev, for the entry `providerId`. */
const modelSearch = ref<{
  providerId: string
  open: boolean
  /** Whether the model chosen is being added. */
  adding: boolean
  error: ShownError | null
} | null>(null)
const searchModelsOf = computed<SettingsModelSearch | undefined>(() => {
  const search = props.searchModels
  const shown = modelSearch.value
  return search && shown ? (query, signal) => search(shown.providerId, query, signal) : undefined
})

function openAddModel(): void {
  if (selected.value) {
    modelSearch.value = { providerId: selected.value.id, open: true, adding: false, error: null }
  }
}

/**
 * Adds the model chosen in Add Model at once and opens it in Edit Model. A
 * taken id takes the first free suffix; on an entry that still follows its
 * vendor's list, the vendor's copy of the model gives way to it instead.
 */
async function addModel(draft: SettingsModelDraft): Promise<void> {
  const search = modelSearch.value
  const provider = props.providers.find((entry) => entry.id === search?.providerId)
  if (!search || !provider || search.adding) {
    return
  }
  const model: SettingsModelDraft = {
    ...draft,
    id: provider.modelSource === 'manual' ? freeModelId(provider.models.map((m) => m.id), draft.id) : draft.id,
  }
  search.adding = true
  let added = false
  try {
    await answerInDialog(
      () => (modelSearch.value === search && search.open ? search : null),
      t('providers.addModelFailed'),
      (error) => formError(error, { invalid_body: null, duplicate_model: t('providers.duplicateModel') }),
      async () => {
        await props.saveModel(provider, model, null)
        added = true
      },
    )
  } finally {
    search.adding = false
  }
  if (added) {
    modelEditor.value = reactive<SettingsModelEditor>({
      providerId: provider.id,
      open: true,
      error: null,
      mode: 'edit',
      original: { ...model, enabled: true },
      model,
    })
  }
}

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

function openModel(mode: 'edit' | 'view', m: SettingsProviderModel) {
  if (!selected.value) {
    return
  }
  // A refused model opens again as the user left it, to correct.
  const refused = modelEditor.value
  if (refused?.error && refused.providerId === selected.value.id && refused.original?.id === m.id) {
    refused.open = true
    return
  }
  modelEditor.value = reactive<SettingsModelEditor>({
    providerId: selected.value.id,
    open: true,
    error: null,
    mode,
    original: m,
    model: toDraft(m),
  })
}

/** Saving a model, of its own group: it waits for what the provider does, rather than being refused. */
function modelSaving(editor: SettingsModelEditor): Operation {
  return { action: `saveModel:${editor.original?.id ?? 'new'}`, target: providerTarget(editor.providerId) }
}

/** The model dialog of `editor`'s model, if it is the one open: the answer to its save goes there. */
function shownModelEditor(providerId: string, modelId: string | null): SettingsModelEditor | null {
  const shown = modelEditor.value
  return shown?.open && shown.providerId === providerId && (shown.original?.id ?? null) === modelId ? shown : null
}

async function saveModel(draft: SettingsModelDraft): Promise<void> {
  const editor = modelEditor.value
  const provider = props.providers.find(
    (entry) => entry.id === editor?.providerId,
  )
  if (!editor || !provider) {
    return
  }
  editor.model = draft
  const original = editor.original
  let saved = false
  // A refused field of the model is the user's to correct in the dialog.
  await answerInDialog(
    () => shownModelEditor(provider.id, original?.id ?? null),
    t('providers.saveModelFailed'),
    (error) => formError(error, { invalid_body: null, duplicate_model: t('providers.duplicateModel') }),
    async () => {
      await props.saveModel(provider, draft, original)
      saved = true
    },
  )
  // The model is now what was saved, its id included, for the next edit.
  if (saved && original) {
    editor.original = { ...draft, enabled: original.enabled }
  }
}

/** Delete Model in Edit Model: asked first, as removing a model always is. */
function deleteEditedModel(): void {
  const editor = modelEditor.value
  const provider = props.providers.find((entry) => entry.id === editor?.providerId)
  if (editor?.original && provider) {
    askRemoveModel(provider, editor.original)
  }
}

function accountPending(providerId: string, accountId: string, action: 'remove' | 'test'): boolean {
  return runs(providerId, { kind: 'account', accountId, action })
}

/**
 * A compatible API's endpoint, which it cannot work without: one typed in is
 * an http or https address. An empty field is no fault: left empty, it
 * removes a saved endpoint, and the entry waits for one.
 */
const endpointFault = ref<string | null>(null)
watch(selectedId, () => {
  endpointFault.value = null
})
function endpointCheck(value: string) {
  return value.trim() === '' ? null : endpointProblem(value)
}
</script>

<template>
  <SettingsPage wide fill :title="t('providers.title')">
    <AsyncRegion
      :state="load"
      :label="t('providers.loading')"
      @retry="emit('retry')"
    >
      <SettingsSplit
        v-model:detail-open="splitOpen"
        :detail-title="selected?.name"
      >
        <template #list>
          <div
            class="select-none px-1 pb-1 pt-1.5 text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
          >
            {{ t('providers.subscriptions') }}
          </div>
          <SettingsListItem
            v-for="p in subscriptions"
            :key="p.id"
            :label="p.name"
            :selected="p.id === selectedId"
            :badge="providerBadge[p.state]"
            :badge-label="providerBadgeLabel[p.state]"
            :muted="!p.enabled"
            @select="select(p.id)"
          >
            <template #leading
              ><VendorMark :label="p.name" :src="p.logo" size="sm"
            /></template>
          </SettingsListItem>
          <!-- Adding belongs to this group alone, so its button sits in the group's caption. -->
          <div class="flex h-8 select-none items-center pl-1 pt-2">
            <span
              class="text-[11px] font-medium uppercase tracking-[0.04em] text-fg-subtle"
              >{{ t('providers.apiKeys') }}</span
            >
            <Button size="xs" class="ml-auto" @click="addOpen = true"
              ><Plus :size="12" /> {{ uiText('common.add') }}</Button
            >
          </div>
          <SettingsListItem
            v-for="p in apiKeys"
            :key="p.id"
            :label="p.name"
            :selected="p.id === selectedId"
            :badge="providerBadge[p.state]"
            :badge-label="providerBadgeLabel[p.state]"
            :muted="!p.enabled"
            removable
            :removing="runs(p.id, { kind: 'removing' })"
            :remove-disabled="busy(p.id)"
            @select="select(p.id)"
            @remove="askRemoveProvider(p)"
          >
            <template #leading
              ><VendorMark :label="p.name" :src="p.logo" size="sm"
            /></template>
          </SettingsListItem>
        </template>

        <template #detail>
          <div v-if="selected" class="flex flex-col gap-6">
            <!-- The rail carries the mark; the first card's header names the provider and holds its
               actions. An API key can be renamed and removed; a subscription is a fixture. -->
            <SettingsGroup>
              <template #header>
                <header class="select-none">
                  <!-- At least a control's height; where the actions do not fit beside the name they
                       wrap under it, and the header grows rather than running over the card. -->
                  <div
                    class="flex min-h-8 flex-wrap items-center gap-x-1.5 gap-y-2"
                  >
                    <TitleInput
                      v-if="renaming"
                      class="w-56 max-w-full"
                      :label="t('providers.name')"
                      :title="selected.name"
                      @submit="rename"
                      @cancel="renaming = false"
                    />
                    <template v-else>
                      <!-- A name that can be changed is also changed by a double-click on it. -->
                      <!-- On a phone the provider is the page, and its name the page's title. -->
                      <h3
                        class="min-w-0 text-[15px] font-medium text-fg-emphasis @max-md:text-[20px] @max-md:leading-7"
                        @dblclick="selected.kind === 'api_key' && (renaming = true)"
                      >
                        <TruncatedText :text="selected.name" />
                      </h3>
                      <Tooltip
                        v-if="selected.kind === 'api_key'"
                        :content="t('providers.rename')"
                        ><IconButton
                          size="sm"
                          variant="ghost"
                          :icon="TextCursorInput"
                          :aria-label="t('providers.rename')"
                          @click="renaming = true"
                      /></Tooltip>
                    </template>
                    <div class="ml-auto flex items-center gap-1.5">
                      <span
                        v-if="runs(selected.id, { kind: 'saving' })"
                        class="flex items-center gap-1 text-[12px] text-fg-subtle"
                        role="status"
                        ><IndeterminateSpinner :size="12" /> {{ t('providers.saving') }}</span
                      >
                      <!-- A subscription grows by accounts; adding one is the header's action. -->
                      <Button
                        v-if="selected.kind === 'subscription'"
                        size="sm"
                        @click="addAccount(selected)"
                        ><Plus :size="ICON_PX.in24" /> {{ t('providers.addAccount') }}</Button
                      >
                      <Switch
                        :model-value="selected.enabled"
                        :aria-label="t('providers.showInMenu')"
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
                <!-- Rotation means something with two accounts or more. -->
                <SettingsRow
                  v-if="selected.accounts.length >= 2"
                  :label="t('providers.rotation.title')"
                  :description="ROTATION_HINTS[selected.rotation ?? 'manual']"
                >
                  <Dropdown
                    size="sm"
                    :overlay-store="overlayStore"
                    variant="default"
                    :trigger-label="t('providers.rotation.title')"
                    :disabled="busy(selected.id)"
                  >
                    <template #trigger>{{ ROTATION_LABELS[selected.rotation ?? 'manual'] }}</template>
                    <template #content="{ close }">
                      <Menu>
                        <MenuItem
                          v-for="rotation in ROTATIONS"
                          :key="rotation"
                          :icon="ROTATION_ICONS[rotation]"
                          :label="ROTATION_LABELS[rotation]"
                          :hint="ROTATION_HINTS[rotation]"
                          choice
                          :is-selected="(selected.rotation ?? 'manual') === rotation"
                          @select="
                            close();
                            chooseRotation(selected, rotation)
                          "
                        />
                      </Menu>
                    </template>
                  </Dropdown>
                </SettingsRow>
                <div ref="accountList" class="contents">
                <SettingsRow
                  v-for="account in selected.accounts"
                  :key="account.id"
                  :data-reorder-id="account.id"
                  :class="{ 'bg-surface-float shadow-md': accountOrder.dragging.value === account.id }"
                  :label="account.label"
                  compact
                >
                  <template #tags>
                    <Tag v-if="account.plan">{{ account.plan }}</Tag>
                    <Tag v-if="account.active" tone="accent">{{ t('providers.account.active') }}</Tag>
                    <Tag
                      v-if="
                        account.quota.some(
                          (window) => window.used >= window.max,
                        )
                      "
                      tone="danger"
                      >{{ t('providers.account.limitReached') }}</Tag
                    >
                  </template>
                  <template #below>
                    <ProviderQuota
                      :key="`${selected.id}:${account.id}`"
                      :windows="account.quota"
                      :auto-refresh="selected.autoRefreshUsage === true"
                      :error="usageFailures?.[selected.id]?.[account.id]"
                      @refresh="emit('refreshUsage', selected, account.id, true)"
                    />
                  </template>
                  <Button
                    size="sm"
                    v-if="!account.active"
                    :disabled="busy(selected.id)"
                    @click="emit('activateAccount', selected, account.id)"
                    >{{ t('providers.account.activate') }}</Button
                  >
                  <!-- Automatic refresh shares the same account request with this explicit retry. -->
                  <Tooltip :content="t('providers.account.refreshUsage')"
                    ><IconButton
                      size="sm"
                      :icon="RefreshCw"
                      :aria-label="t('providers.account.refreshUsage')"
                      spin-on-click
                      :spinning="refreshingUsage?.[selected.id]?.[account.id] === 'manual'"
                      :disabled="busy(selected.id) || refreshingUsage?.[selected.id]?.[account.id] === 'manual'"
                      @click="emit('refreshUsage', selected, account.id)"
                  /></Tooltip>
                  <!-- The test asks with this account, in use or not; its answer is a toast,
                       since the row has no room to keep it. -->
                  <Tooltip :content="t('providers.account.test')"
                    ><IconButton
                      size="sm"
                      :icon="Plug"
                      :aria-label="t('providers.account.test')"
                      :operation="providerOperation(selected.id, { kind: 'account', accountId: account.id, action: 'test' })"
                      :disabled="!selected.models.some((model) => model.enabled)"
                      :disabled-reason="testBlockReason"
                      @click="emit('test', selected, account.id)"
                  /></Tooltip>
                  <Tooltip :content="t('providers.account.remove')"
                    ><IconButton
                      size="sm"
                      :icon="Trash2"
                      variant="danger"
                      :aria-label="t('providers.account.remove')"
                      :operation="providerOperation(selected.id, { kind: 'account', accountId: account.id, action: 'remove' })"
                      @click="askRemoveAccount(selected, account)"
                  /></Tooltip>
                  <!-- The list's order is the one In Order follows: drag a row by its handle,
                       or move it with the arrow keys. -->
                  <Tooltip v-if="selected.accounts.length >= 2" :content="t('providers.account.reorder')"
                    ><IconButton
                      size="sm"
                      variant="ghost"
                      class="cursor-grab touch-none"
                      :icon="GripVertical"
                      :aria-label="t('providers.account.move', { label: account.label })"
                      @pointerdown="accountOrder.start($event, account.id)"
                      @keydown="accountOrder.keydown($event, account.id)"
                  /></Tooltip>
                </SettingsRow>
                </div>
                <div
                  v-if="!selected.accounts.length"
                  class="flex select-none flex-col items-center gap-1.5 px-4 py-6 text-center text-[13px] text-fg-subtle"
                >
                  {{ t('providers.account.none') }}
                </div>
              </template>

              <!-- Connection (API key) -->
              <template v-else>
                <!-- A vendor's endpoint is its vendor's: shown, never edited. A compatible API's
                     protocol and endpoint are the user's, changed any time; what it still lacks is
                     an empty field, never red. -->
                <SettingsRow v-if="!selected.compatible" :label="t('providers.connection.endpoint')">
                  <span class="min-w-0 select-text break-all text-[13px] text-fg-muted">{{
                    selected.baseUrl
                  }}</span>
                </SettingsRow>
                <template v-else>
                  <SettingsRow :label="t('providers.connection.protocol')">
                    <Dropdown
                      size="sm"
                      :overlay-store="overlayStore"
                      variant="default"
                      :trigger-label="t('providers.connection.protocol')"
                      :disabled="fieldsLocked(selected)"
                    >
                      <template #trigger>{{ WIRE_API_LABELS[selected.wireApi] }}</template>
                      <template #content="{ close }">
                        <Menu>
                          <MenuItem
                            v-for="wire in COMPATIBLE_WIRE_APIS"
                            :key="wire"
                            :label="WIRE_API_LABELS[wire]"
                            choice
                            :is-selected="selected.wireApi === wire"
                            @select="
                              close();
                              wire !== selected.wireApi && emit('change', selected, { wireApi: wire })
                            "
                          />
                        </Menu>
                      </template>
                    </Dropdown>
                  </SettingsRow>
                  <SettingsRow :label="t('providers.connection.endpoint')">
                    <template v-if="endpointFault" #detail>
                      <InlineError :message="endpointFault" />
                    </template>
                    <CommitTextInput
                      :disabled="fieldsLocked(selected)"
                      :model-value="selected.baseUrl"
                      :aria-label="t('providers.connection.endpoint')"
                      placeholder="https://api.example.com/v1"
                      literal
                      trim
                      :validate="endpointCheck"
                      class="w-72 max-w-full"
                      @problem="endpointFault = $event"
                      @commit="emit('change', selected, { baseUrl: $event })"
                    />
                  </SettingsRow>
                </template>
                <SettingsRow
                  :label="t('providers.connection.apiKey')"
                  :description="
                    selected.compatible
                      ? t('providers.connection.keyOptional')
                      : t('providers.connection.keyStored')
                  "
                >
                  <CommitTextInput
                    :disabled="fieldsLocked(selected)"
                    :model-value="selected.apiKey"
                    :aria-label="t('providers.connection.apiKey')"
                    @commit="emit('change', selected, { apiKey: $event })"
                    masked="secret"
                    trim
                    :stored="selected.keyConfigured"
                    :placeholder="selected.keyConfigured ? t('providers.connection.keyReplace') : t('providers.connection.keyPaste')"
                    class="w-72 max-w-full"
                  />
                </SettingsRow>
              </template>
              <!-- A key's test has a row of its own; a subscription's sits on its active account. -->
              <SettingsRow
                v-if="selected.kind === 'api_key'"
                :label="t('providers.connection.test')"
                :description="t('providers.connection.testDescription')"
              >
                <!-- The provider's own words in full, under the row; the page does not paraphrase them. -->
                <template
                  v-if="
                    selected.detail &&
                    !isTesting(selected)
                  "
                  #detail
                >
                  <p class="select-text break-words text-[12px] leading-4 text-on-danger">
                    {{ selected.detail }}
                  </p>
                </template>
                <span
                  v-if="isTesting(selected)"
                  class="flex items-center gap-1.5 text-[12px] text-fg-subtle"
                  ><IndeterminateSpinner :size="ICON_PX.in24" />
                  {{ t('providers.connection.testing') }}</span
                >
                <span
                  v-else-if="selected.testPassed || selected.testedIn"
                  class="flex items-center gap-1 text-[12px] text-on-success"
                  ><Check :size="ICON_PX.in24" /> {{ t('providers.connection.connected') }}<template
                    v-if="selected.testedWith"
                  >
                    · {{ selected.testedWith }}</template
                  ><template
                    v-if="selected.testedIn"
                  >
                    · {{ selected.testedIn }}</template
                  ></span
                >
                <span
                  v-else-if="selected.detail"
                  class="min-w-0 truncate text-[12px] text-on-danger"
                  >{{ t('providers.connection.failed') }}<template
                    v-if="selected.testedWith"
                  >
                    · {{ selected.testedWith }}</template
                  ></span
                >
                <Tooltip :content="t('providers.account.test')"
                  ><IconButton
                    size="sm"
                    :icon="Plug"
                    :aria-label="t('providers.account.test')"
                    :disabled="
                      selected.configured === false ||
                      busy(selected.id) ||
                      !selected.models.some((model) => model.enabled)
                    "
                    :disabled-reason="testBlockReason"
                    @click="emit('test', selected)"
                /></Tooltip>
              </SettingsRow>
            </SettingsGroup>

            <!-- The CLI of a provider whose requests are a process: Demi installs the vendor's newest
               on the machines that run it, so this shows what is wanted and what each machine has,
               and only a failure asks for anything. -->
            <SettingsGroup v-if="selected.cli" :title="t('providers.cli.title')">
              <SettingsRow
                :label="t('providers.cli.version')"
                :description="t('providers.cli.versionDescription')"
              >
                <span
                  v-if="'version' in selected.cli.newest"
                  class="text-[13px] tabular-nums text-fg-muted"
                  >{{ selected.cli.newest.version }}</span
                >
                <Tooltip :content="t('providers.cli.check')"
                  ><IconButton
                    size="sm"
                    :icon="RefreshCw"
                    :aria-label="t('providers.cli.check')"
                    spin-on-click
                    :spinning="runs(selected.id, { kind: 'cli' })"
                    :disabled="busy(selected.id) && !runs(selected.id, { kind: 'cli' })"
                    @click="emit('checkCli', selected)"
                /></Tooltip>
              </SettingsRow>
              <SettingsRow
                v-if="'error' in selected.cli.newest"
                compact
                :label="t('providers.cli.unreadable')"
                :description="selected.cli.newest.error"
              />
              <SettingsRow
                v-for="machine in selected.cli.machines"
                :key="machine.id"
                compact
                :label="machine.name"
              >
                <span class="text-[13px] tabular-nums text-fg-muted">{{
                  machine.versions === null
                    ? t('providers.cli.noAnswer')
                    : machine.versions.length
                      ? machine.versions.join(', ')
                      : t('providers.cli.notInstalled')
                }}</span>
              </SettingsRow>
              <SettingsRow
                v-if="selected.cli.install && selected.cli.install.state !== 'installed'"
                compact
                :label="selected.cli.install.state === 'installing' ? t('providers.cli.installing') : t('providers.cli.installFailed')"
                :description="selected.cli.install.state === 'failed' ? selected.cli.install.message : undefined"
              >
                <IndeterminateSpinner v-if="selected.cli.install.state === 'installing'" />
                <Button
                  v-else
                  size="sm"
                  :operation="providerOperation(selected.id, { kind: 'cli' })"
                  @click="onInstallCli?.(selected)"
                  >{{ t('providers.cli.install') }}</Button
                >
              </SettingsRow>
            </SettingsGroup>

            <!-- Models. A subscription's list is whatever the vendor serves, so it is refreshed, never
               edited; a vendor key chooses between the catalog and a manual list; a bare endpoint only
               has the manual list. Every kind chooses which of its models the pickers offer. -->
            <SettingsGroup>
              <template #header>
                <header class="flex h-7 select-none items-center gap-2">
                  <h3
                    class="text-[15px] font-medium leading-5 text-fg-emphasis"
                  >
                    {{ t('providers.models.title') }}
                  </h3>
                  <span
                    v-if="
                      selected.modelSource === 'catalog' &&
                      selected.catalogFetched
                    "
                    class="text-[12px] text-fg-subtle"
                    >{{ t('providers.models.updated') }} <RelativeTime :timestamp="selected.catalogFetched"
                  /></span>
                  <span v-if="selected.kind === 'subscription'" class="ml-auto"
                    ><Tooltip :content="t('providers.models.refresh')"
                      ><IconButton
                        size="sm"
                        :icon="RefreshCw"
                        spin-on-click
                        :spinning="runs(selected.id, { kind: 'refreshing' })"
                        :disabled="
                          selected.configured === false ||
                          runs(selected.id, { kind: 'refreshing' })
                        "
                        :aria-label="t('providers.models.refresh')"
                        @click="emit('refresh', selected)" /></Tooltip
                  ></span>
                  <Segmented
                    size="sm"
                    v-else-if="!selected.compatible"
                    :model-value="selected.modelSource"
                    :disabled="fieldsLocked(selected)"
                    @update:model-value="
                      emit('change', selected, { modelSource: $event })
                    "
                    class="ml-auto"
                    :options="[
                      { value: 'catalog', label: t('providers.models.catalog') },
                      { value: 'manual', label: t('providers.models.manual') },
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
                  search
                  :placeholder="t('providers.models.filterModels')"
                  bare
                  class="min-w-0 flex-1"
                >
                  <template #prefix><Search :size="ICON_PX.in24" /></template>
                </TextInput>
                <Tooltip :content="t('providers.models.add')"
                  ><IconButton
                    size="sm"
                    :icon="Plus"
                    :aria-label="t('providers.models.add')"
                    :disabled="fieldsLocked(selected)"
                    @click="openAddModel"
                /></Tooltip>
                <!-- A compatible API has no catalog to read again: its models are the ones added. -->
                <Tooltip v-if="!selected.compatible && selected.modelSource === 'catalog'" :content="t('providers.models.refresh')"
                  ><IconButton
                    size="sm"
                    :icon="RefreshCw"
                    spin-on-click
                    :spinning="runs(selected.id, { kind: 'refreshing' })"
                    :disabled="
                      selected.configured === false ||
                      runs(selected.id, { kind: 'refreshing' })
                    "
                    :aria-label="t('providers.models.refresh')"
                    @click="emit('refresh', selected)"
                /></Tooltip>
              </div>
              <AsyncRegion
                :state="
                  selected.models.length || selected.modelSource === 'manual'
                    ? 'ready'
                    : modelLoad
                "
                :label="t('providers.models.loading')"
                @retry="emit('retryModels')"
              >
                <SettingsRow
                  v-for="m in visibleModels(selected)"
                  :key="m.id"
                  :label="m.name || m.id"
                  compact
                  interactive
                  isolate-controls
                  :muted="!m.enabled"
                  @click="emit('toggleModel', selected, m, !m.enabled)"
                >
                  <template #tags>
                    <Tooltip
                      v-if="isUnknown(m)"
                      :content="t('providers.models.unknown')"
                      ><Tag tone="warning"><TriangleAlert :size="12" /></Tag
                    ></Tooltip>
                    <Tag v-if="m.contextWindow !== null">{{
                      formatTokens(m.contextWindow)
                    }}</Tag>
                    <Tooltip
                      v-if="m.extensions?.length"
                      :content="t('providers.models.accepts', { types: m.extensions?.join(' ') ?? '' })"
                      ><Tag><Image :size="12" /></Tag
                    ></Tooltip>
                    <Tooltip
                      v-if="m.efforts.length"
                      :content="t('providers.models.reasoning', { efforts: m.efforts.map(effortLabel).join(', ') })"
                      ><Tag><Brain :size="12" /></Tag
                    ></Tooltip>
                    <Tooltip v-if="m.fastTier" :content="t('providers.models.fast')"
                      ><Tag><Zap :size="12" /></Tag
                    ></Tooltip>
                  </template>
                  <template
                    v-if="
                      selected.kind === 'api_key' &&
                      selected.modelSource === 'manual'
                    "
                  >
                    <Tooltip :content="t('providers.models.edit')"
                      ><IconButton
                        size="sm"
                        :icon="SlidersHorizontal"
                        :aria-label="t('providers.models.edit')"
                        :disabled="fieldsLocked(selected)"
                        @click="openModel('edit', m)"
                    /></Tooltip>
                    <Tooltip :content="t('providers.account.remove')"
                      ><IconButton
                        size="sm"
                        :icon="Trash2"
                        variant="danger"
                        :aria-label="t('providers.account.remove')"
                        @click="askRemoveModel(selected, m)"
                    /></Tooltip>
                  </template>
                  <Tooltip v-else :content="t('providers.models.details')"
                    ><IconButton
                      size="sm"
                      :icon="Info"
                      :aria-label="t('providers.models.details')"
                      @click="openModel('view', m)"
                  /></Tooltip>
                  <Switch
                    :model-value="m.enabled"
                    @update:model-value="
                      emit('toggleModel', selected, m, $event)
                    "
                    size="sm"
                    :aria-label="t('providers.models.enable', { name: m.name || m.id })"
                  />
                </SettingsRow>
                <div
                  v-if="!visibleModels(selected).length"
                  class="flex select-none justify-center px-4 py-6 text-center text-[13px] text-fg-subtle"
                >
                  <!-- An empty list is a plain wait, never red: nothing has failed. -->
                  {{
                    modelFilter
                      ? t('providers.models.noResults')
                      : selected.kind === 'subscription' &&
                          !selected.accounts.length
                        ? t('providers.models.signInToSee')
                        : t('providers.models.none')
                  }}
                </div>
              </AsyncRegion>
            </SettingsGroup>
          </div>
        </template>
      </SettingsSplit>
    </AsyncRegion>

    <AddProviderDialog
      :is-open="addOpen"
      :overlay-store="overlayStore"
      :vendors="vendors"
      :load="vendorLoad"
      @retry="emit('retryVendors')"
      @close="addOpen = false"
      @add="
        (v) => {
          addOpen = false
          emit('add', v)
        }
      "
      @add-compatible="
        () => {
          addOpen = false
          emit('addCompatible')
        }
      "
    />
    <ConfirmDialog
      :is-open="confirmOpen"
      :overlay-store="overlayStore"
      :title="confirming?.title ?? ''"
      :action="confirming?.action ?? uiText('common.remove')"
      :operation="confirming?.operation"
      @close="confirmOpen = false"
      @confirm="confirm"
    >
      <p v-for="(line, index) in confirming?.body" :key="index">{{ line }}</p>
    </ConfirmDialog>
    <ConfirmDialog
      :is-open="warningOpen"
      :overlay-store="overlayStore"
      :title="t('providers.warning.title')"
      :action="uiText('common.continue')"
      :on-confirm="continuePastWarning"
      @close="warningOpen = false; warning = null"
    >
      <p>{{ t('providers.warning.body') }}</p>
      <p>{{ t('providers.warning.once') }}</p>
    </ConfirmDialog>
    <ModelDialog
      v-if="modelEditor"
      :is-open="modelEditor.open"
      :overlay-store="overlayStore"
      :mode="modelEditor.mode"
      :error="modelEditor.error"
      :operation="modelSaving(modelEditor)"
      :model="modelEditor.model"
      :taken-ids="takenIds"
      @close="modelEditor.open = false"
      @save="saveModel"
      @delete="deleteEditedModel"
    />
    <AddModelDialog
      v-if="modelSearch"
      :is-open="modelSearch.open"
      :overlay-store="overlayStore"
      :search="searchModelsOf"
      :adding="modelSearch.adding"
      :error="modelSearch.error"
      @close="modelSearch.open = false"
      @add="addModel"
    />
  </SettingsPage>
</template>

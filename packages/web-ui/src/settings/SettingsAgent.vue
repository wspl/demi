<script setup lang="ts">
import { computed } from 'vue'
import { Feather, Plus } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { t } from '../locales/text'
import ModelMenu from '../agent/ModelMenu.vue'
import { applyModelChange, initialModelSettings, type ModelSettings, type ModelSettingsChange } from '../agent/model-selection'
import { otherWayKeys, sendWayLabel, type SendWhileRunning } from '../agent/send-way'
import Dropdown from '../ui/Dropdown.vue'
import Menu from '../ui/Menu.vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import Switch from '../ui/Switch.vue'
import ModelChoiceTrigger from './ModelChoiceTrigger.vue'
import { modelChoiceLabel, type ModelChoiceLabel } from './model-choice'
import { profileMissing } from './subagent-profiles'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { ImageEntryChoices, SmallModelChoice } from './types'
import type { ImageEntryOption, ImageVendorRow } from './image-entries'
import type { InlineDetailText } from '../ui/inline-detail'

/**
 * How the agent works for the user (`models.md` § Small model,
 * `turns.md`): the small model that writes conversation titles, Automatic
 * or one the user picks with the composer's model menu, and whether small
 * requests stay on the conversation's provider; the entry that makes each
 * vendor's images (`images.md` § Which entry makes an image), Automatic or
 * one the user picks; then what Enter does with a message while the agent
 * works, and whether a stopped turn goes on by itself. Every row is a model.
 */
const props = defineProps<{
  overlayStore: OverlayStore
  /** The providers and models the menu offers, as the composer's does. */
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  /** Every model each provider's catalog lists, hidden ones included, which name a chosen model. */
  catalogModels: Record<string, ModelInfo[]>
  /**
   * The name of the model Automatic resolves to, which the backend answers;
   * null when it resolves none for the page, so Automatic shows alone.
   */
  automaticModel: string | null
  /** A row for each vendor Demi makes images with, always all three. */
  imageVendors: ImageVendorRow[]
}>()

const emit = defineEmits<{
  /** Add Provider… in a vendor's menu that no entry makes images for: the host opens Settings › Providers. */
  addProvider: []
}>()

const smallModel = defineModel<SmallModelChoice | null>('smallModel', { required: true })
const sameProvider = defineModel<boolean>('sameProvider', { required: true })
const sendWhileRunning = defineModel<SendWhileRunning>('sendWhileRunning', { required: true })
const resumeAfterRestart = defineModel<boolean>('resumeAfterRestart', { required: true })
const resumeAtRetry = defineModel<boolean>('resumeAtRetry', { required: true })
const imageEntries = defineModel<ImageEntryChoices>('imageEntries', { required: true })

/** The small model as the menu holds model settings: always the default tier. */
const settings = computed<ModelSettings | null>(() =>
  smallModel.value ? { ...smallModel.value, serviceTierId: null } : null,
)

/**
 * What Automatic resolves to, after it in the language's brackets,
 * `Automatic (Codex)`: the brackets are cut from the language's own
 * pattern, so the text inside can truncate while they stay whole.
 */
function resolution(text: string): InlineDetailText {
  const marker = '\u0000'
  const [open = '', close = ''] = t('settings.agent.automaticResolution', { detail: marker }).split(marker)
  return { text, open, close }
}

/** Automatic, with the model it resolves to after it when the backend says. */
const automaticChoice = computed(() => ({
  label: t('settings.agent.automatic'),
  icon: Feather,
  detail: props.automaticModel ? resolution(props.automaticModel) : undefined,
}))

const label = computed<ModelChoiceLabel>(() =>
  settings.value
    ? modelChoiceLabel(settings.value, props.providers, props.catalogModels)
    : { name: t('settings.agent.automatic'), effort: '', fast: false },
)

/**
 * Why the chosen model cannot be used now, which resolves it as Automatic
 * until it is back: its entry is gone or unusable, or it or its effort is
 * gone or hidden.
 */
const missing = computed(() => profileMissing(
  settings.value,
  props.providers.filter((provider) => provider.isAvailable),
  props.models,
))

function chooseAutomatic(): void {
  smallModel.value = null
}

function change(next: ModelSettingsChange): void {
  const { providerId, modelId, thinkingEffort } = applyModelChange(settings.value ?? initialModelSettings(), next)
  smallModel.value = { providerId, modelId, thinkingEffort }
}

/** The entry chosen for `row`'s vendor while it can be chosen; otherwise the backend resolves Automatic. */
function chosenEntry(row: ImageVendorRow): ImageEntryOption | undefined {
  return row.entries.find((entry) => entry.id === imageEntries.value[row.vendor])
}

/**
 * What Automatic resolves to, after it: the name of the entry it takes now,
 * never its account; Not available for a vendor none of whose entries
 * makes images; nothing while its entries do but none can now.
 */
function automaticDetail(row: ImageVendorRow): InlineDetailText | undefined {
  if (row.automatic) {
    return resolution(row.automatic.name)
  }
  return row.entries.length ? undefined : resolution(t('settings.agent.notAvailable'))
}

/** Chooses `entry` for `row`'s vendor, or Automatic for null. */
function chooseEntry(row: ImageVendorRow, entry: string | null): void {
  const next = { ...imageEntries.value }
  if (entry === null) {
    delete next[row.vendor]
  } else {
    next[row.vendor] = entry
  }
  imageEntries.value = next
}

const sendOptions = computed<readonly SegmentedOption<SendWhileRunning>[]>(() => [
  { value: 'steer', label: sendWayLabel('steer') },
  { value: 'queue', label: sendWayLabel('queue') },
])
</script>

<template>
  <SettingsPage :title="t('settings.agent.title')" :description="t('settings.agent.description')">
    <SettingsGroup :title="t('settings.agent.smallModel')">
      <SettingsRow :label="t('settings.agent.model')" :description="t('settings.agent.modelDescription')">
        <Dropdown
          :overlay-store="overlayStore"
          variant="field"
          size="sm"
          width="shrink"
          :trigger-label="t('settings.agent.model')"
          :detail="settings ? undefined : automaticChoice.detail"
        >
          <template #trigger>
            <ModelChoiceTrigger :label="label" :missing="missing" />
          </template>
          <template #content>
            <ModelMenu
              :providers="providers"
              :models="models"
              :settings="settings"
              :stand-in="automaticChoice"
              without-context
              without-fast
              @stand-in="chooseAutomatic"
              @change="change"
            />
          </template>
        </Dropdown>
      </SettingsRow>
      <SettingsRow
        :label="t('settings.agent.sameProvider')"
        :description="t('settings.agent.sameProviderDescription')"
      >
        <Switch v-model="sameProvider" size="sm" :aria-label="t('settings.agent.sameProvider')" />
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup
      :title="t('settings.agent.imageGeneration')"
      :description="t('settings.agent.imageGenerationDescription')"
    >
      <SettingsRow v-for="row in imageVendors" :key="row.vendor" :label="row.name">
        <Dropdown
          :overlay-store="overlayStore"
          variant="field"
          size="sm"
          width="shrink"
          :trigger-label="row.name"
          :detail="chosenEntry(row) ? undefined : automaticDetail(row)"
        >
          <template #trigger>
            <span class="min-w-0 overflow-clip text-ellipsis whitespace-nowrap">{{ chosenEntry(row)?.name ?? t('settings.agent.automatic') }}</span>
          </template>
          <template #content="{ close }">
            <Menu>
              <MenuItem
                :label="t('settings.agent.automatic')"
                :detail="automaticDetail(row)"
                choice
                :is-selected="!chosenEntry(row)"
                @select="chooseEntry(row, null); close()"
              />
              <MenuDivider />
              <MenuItem
                v-if="!row.entries.length"
                :icon="Plus"
                :label="t('settings.agent.addProvider')"
                @select="emit('addProvider'); close()"
              />
              <MenuItem
                v-for="entry in row.entries"
                :key="entry.id"
                :label="entry.name"
                :detail="entry.account ?? undefined"
                choice
                :is-selected="chosenEntry(row)?.id === entry.id"
                @select="chooseEntry(row, entry.id); close()"
              />
            </Menu>
          </template>
        </Dropdown>
      </SettingsRow>
    </SettingsGroup>
    <SettingsGroup :title="t('settings.agent.messages')">
      <SettingsRow
        :label="t('settings.agent.sendWhileRunning')"
        :description="t('settings.agent.sendWhileRunningDescription', { keys: otherWayKeys() })"
      >
        <Segmented
          v-model="sendWhileRunning"
          size="sm"
          :options="sendOptions"
        />
      </SettingsRow>
      <SettingsRow
        :label="t('settings.agent.resumeAfterRestart')"
        :description="t('settings.agent.resumeAfterRestartDescription')"
      >
        <Switch
          v-model="resumeAfterRestart"
          size="sm"
          :aria-label="t('settings.agent.resumeAfterRestart')"
        />
      </SettingsRow>
      <SettingsRow
        :label="t('settings.agent.resumeAtRetry')"
        :description="t('settings.agent.resumeAtRetryDescription')"
      >
        <Switch v-model="resumeAtRetry" size="sm" :aria-label="t('settings.agent.resumeAtRetry')" />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>

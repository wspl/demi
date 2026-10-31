<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { Feather } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import { t } from '../locales/text'
import ModelMenu from '../agent/ModelMenu.vue'
import { applyModelChange, initialModelSettings, type ModelSettings, type ModelSettingsChange } from '../agent/model-selection'
import { otherWayKeys, sendWayLabel, type SendWhileRunning } from '../agent/send-way'
import Dropdown from '../ui/Dropdown.vue'
import Segmented, { type SegmentedOption } from '../ui/Segmented.vue'
import { IDLE_COMPACTION_CONTEXT_FLOOR } from '@demicodes/protocol'
import InlineError from '../ui/InlineError.vue'
import Switch from '../ui/Switch.vue'
import TokenInput from '../ui/TokenInput.vue'
import { formatTokens } from '../ui/token-count'
import type { SentenceText } from '../ui/ui-text'
import ModelChoiceTrigger from './ModelChoiceTrigger.vue'
import { modelChoiceLabel, type ModelChoiceLabel } from './model-choice'
import { profileMissing } from './subagent-profiles'
import SettingsGroup from './SettingsGroup.vue'
import SettingsPage from './SettingsPage.vue'
import SettingsRow from './SettingsRow.vue'
import type { IdleLifetime, SmallModelChoice } from './types'
import type { InlineDetailText } from '../ui/inline-detail'

/**
 * How the agent works for the user (`crates/provider-registry/src/small_model.rs`):
 *
 * - The small model that writes conversation titles: Automatic or one the user
 *   picks with the composer's model menu.
 * - Whether small requests stay on the conversation's provider.
 * - What Enter does with a message while the agent works.
 * - Whether a stopped turn goes on by itself.
 * - Whether an idle conversation compacts before its prompt cache expires,
 *   for which least cache lifetime and from which least context; both keep
 *   their values while the switch is off. The context commits on Enter or
 *   when the field loses focus; one below the floor the backend takes
 *   (`IDLE_COMPACTION_CONTEXT_FLOOR`) stays in the field with what to fix,
 *   and Escape gives the field back the value the setting holds.
 *
 * Every row is a model. Which entry makes generated media is the order of
 * Settings › Providers and each entry's switches.
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
}>()

const smallModel = defineModel<SmallModelChoice | null>('smallModel', { required: true })
const sameProvider = defineModel<boolean>('sameProvider', { required: true })
const suggestedReplies = defineModel<boolean>('suggestedReplies', { required: true })
const outputSpeed = defineModel<boolean>('outputSpeed', { required: true })
const sendWhileRunning = defineModel<SendWhileRunning>('sendWhileRunning', { required: true })
const resumeAfterRestart = defineModel<boolean>('resumeAfterRestart', { required: true })
const resumeAtRetry = defineModel<boolean>('resumeAtRetry', { required: true })
const compactIdle = defineModel<boolean>('compactIdle', { required: true })
const idleLifetime = defineModel<IdleLifetime>('idleLifetime', { required: true })
/** The least context, in tokens, of a conversation that compacts while idle. */
const idleContext = defineModel<number>('idleContext', { required: true })

const contextDraft = ref<number | null>(idleContext.value)
const contextProblem = ref<SentenceText | null>(null)
watch(idleContext, (tokens) => {
  contextDraft.value = tokens
  contextProblem.value = null
})

function commitContext() {
  const tokens = contextDraft.value
  if (tokens === null || tokens < IDLE_COMPACTION_CONTEXT_FLOOR) {
    contextProblem.value = t('settings.agent.idleContextFloor', { floor: formatTokens(IDLE_COMPACTION_CONTEXT_FLOOR) })
    return
  }
  contextProblem.value = null
  if (tokens !== idleContext.value)
    idleContext.value = tokens
}

/**
 * Enter commits and Escape gives the field back the value the setting holds;
 * either ends the edit, and the dialog around stays open.
 */
function finishContext(event: KeyboardEvent) {
  event.preventDefault()
  if (event.key === 'Escape') {
    contextDraft.value = idleContext.value
    contextProblem.value = null
  }
  ;(event.target as HTMLElement).blur()
}

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

const lifetimeOptions = computed<readonly SegmentedOption<IdleLifetime>[]>(() => [
  { value: '30m', label: t('settings.agent.halfHour') },
  { value: '1h', label: t('settings.agent.hour') },
])

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
        :label="t('settings.agent.suggestedReplies')"
        :description="t('settings.agent.suggestedRepliesDescription')"
      >
        <Switch v-model="suggestedReplies" size="sm" :aria-label="t('settings.agent.suggestedReplies')" />
      </SettingsRow>
      <SettingsRow
        :label="t('settings.agent.outputSpeed')"
        :description="t('settings.agent.outputSpeedDescription')"
      >
        <Switch v-model="outputSpeed" size="sm" :aria-label="t('settings.agent.outputSpeed')" />
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
    <SettingsGroup :title="t('settings.agent.context')">
      <SettingsRow
        :label="t('settings.agent.compactIdle')"
        :description="t('settings.agent.compactIdleDescription')"
      >
        <Switch v-model="compactIdle" size="sm" :aria-label="t('settings.agent.compactIdle')" />
      </SettingsRow>
      <SettingsRow
        :label="t('settings.agent.idleLifetime')"
        :description="t('settings.agent.idleLifetimeDescription')"
        :disabled="!compactIdle"
      >
        <Segmented
          v-model="idleLifetime"
          size="sm"
          :options="lifetimeOptions"
          :disabled="!compactIdle"
        />
      </SettingsRow>
      <SettingsRow
        :label="t('settings.agent.idleContext')"
        :description="t('settings.agent.idleContextDescription')"
        :disabled="!compactIdle"
      >
        <template v-if="contextProblem" #problem>
          <InlineError :message="contextProblem" />
        </template>
        <TokenInput
          v-model="contextDraft"
          size="sm"
          class="w-32"
          :disabled="!compactIdle"
          :aria-label="t('settings.agent.idleContext')"
          @focusout="commitContext"
          @keydown.enter="finishContext"
          @keydown.escape="finishContext"
        />
      </SettingsRow>
    </SettingsGroup>
  </SettingsPage>
</template>

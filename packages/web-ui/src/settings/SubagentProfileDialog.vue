<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { t as agentText } from '../agent/locales/text'
import type { ShownError } from '../infra/errors'
import { computed, ref, watch } from 'vue'
import { CornerLeftUp } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import ModelMenu from '../agent/ModelMenu.vue'
import ContextMenu from '../agent/ContextMenu.vue'
import { profileContextChoice, profileContextOptions } from '../agent/context-limit'
import {
  applyModelChange,
  initialModelSettings,
  type ModelSettings,
  type ModelSettingsChange,
} from '../agent/model-selection'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import type { Action, Operation } from '../ui/operations'
import ScrollArea from '../ui/ScrollArea.vue'
import InlineError from '../ui/InlineError.vue'
import Switch from '../ui/Switch.vue'
import TextArea from '../ui/TextArea.vue'
import TextInput from '../ui/TextInput.vue'
import SettingsRow from './SettingsRow.vue'
import ModelChoiceTrigger from './ModelChoiceTrigger.vue'
import { modelChoiceLabel, type ModelChoiceLabel } from './model-choice'
import { profileMissing, profileNameProblem } from './subagent-profiles'
import type { SettingsSubagentDraft } from './types'

/**
 * One subagent profile, created or edited in its own dialog:
 *
 * - its name and when to use it,
 * - its model, chosen with the composer's own model menu with Parent’s Model at
 *   its top,
 * - its Context, while the model is its own and offers a choice,
 * - the instructions it adds to Demi's own, and
 * - whether its children may spawn.
 *
 * The host saves; a refusal shows under the fields, and the draft stays.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  mode: 'create' | 'edit'
  profile: SettingsSubagentDraft
  /** The providers and models the model menu offers, as the composer's does. */
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  /**
   * Every model each provider's catalog lists, the ones the menu hides
   * included, which name the profile's model and tell whether it is gone.
   */
  catalogModels: Record<string, ModelInfo[]>
  /** Saves the profile; Save works until it settles. */
  onSave?: Action<[draft: SettingsSubagentDraft]>
  /** What Save does, and what Delete does, one group on the profile (`operations.ts`): while one works, the other cannot be pressed. */
  saveOperation?: Operation
  deleteOperation?: Operation
  /** Why the last save failed, with the request it came from. */
  error?: ShownError | null
}>()

const emit = defineEmits<{
  close: []
  delete: []
}>()

/** The draft as the dialog edits it: blank instructions stand for none. */
function fields(profile: SettingsSubagentDraft) {
  return {
    name: profile.name,
    description: profile.description,
    model: profile.model,
    contextLimit: profile.contextLimit,
    instructions: profile.instructions ?? '',
    canSpawn: profile.canSpawn,
  }
}

const draft = ref(fields(props.profile))

watch(
  () => [props.isOpen, props.profile] as const,
  ([open]) => {
    if (open) {
      draft.value = fields(props.profile)
    }
  },
)

/**
 * What the menu's trigger reads, as the composer's chip does: Parent’s
 * Model, or the model with its provider, then its effort a step quieter,
 * then the bolt while Fast is on.
 */
const modelLabel = computed<ModelChoiceLabel>(() => {
  const settings = draft.value.model
  return settings
    ? modelChoiceLabel(settings, props.providers, props.catalogModels)
    : { name: agentText('models.parent'), effort: '', fast: false }
})
const missing = computed(() => profileMissing(draft.value.model, props.providers, props.catalogModels))

/** The catalog's entry for the profile's own model; none for the parent's or a model that is gone. */
function ownModel(settings: ModelSettings | null): ModelInfo | null {
  if (!settings) {
    return null
  }
  return (props.catalogModels[settings.providerId] ?? []).find((model) => model.id === settings.modelId) ?? null
}

/** The Context control, while the profile's own model offers a choice: its options, the chosen one marked. */
const context = computed(() => {
  const model = ownModel(draft.value.model)
  const options = model ? profileContextOptions(model) : []
  if (!model || options.length === 0) {
    return null
  }
  const chosen = profileContextChoice(model, draft.value.contextLimit)
  const marked = options.map((option) => ({ ...option, selected: option.tokens === chosen }))
  return { options: marked, label: marked.find((option) => option.selected)?.label ?? '' }
})

/** Another model keeps the profile's Context while it offers it, as the backend does; the parent's keeps none. */
function keepContext(): void {
  const model = ownModel(draft.value.model)
  const offered = model && profileContextOptions(model).some((option) => option.tokens === draft.value.contextLimit)
  if (!offered) {
    draft.value.contextLimit = null
  }
}

function changeModel(change: ModelSettingsChange): void {
  draft.value.model = applyModelChange(draft.value.model ?? initialModelSettings(), change)
  keepContext()
}

/** The model menu's choice at its top: the parent's model, which a profile without one of its own infers with. */
const parentChoice = computed(() => ({ label: agentText('models.parent'), icon: CornerLeftUp }))

function chooseParentModel(): void {
  draft.value.model = null
  keepContext()
}

/** What breaks the name rule, said under the field as the name is typed. */
const nameProblem = computed(() => profileNameProblem(draft.value.name.trim()))

const canSave = computed(() =>
  draft.value.name.trim() !== '' &&
  nameProblem.value === null &&
  draft.value.description.trim() !== '',
)

function save(): unknown {
  if (!canSave.value) {
    return
  }
  const model: ModelSettings | null = draft.value.model ? { ...draft.value.model } : null
  return props.onSave?.({
    name: draft.value.name.trim(),
    description: draft.value.description.trim(),
    model,
    contextLimit: model ? draft.value.contextLimit : null,
    instructions: draft.value.instructions.trim() === '' ? null : draft.value.instructions,
    canSpawn: draft.value.canSpawn,
  })
}

const title = computed(() => (props.mode === 'create' ? t('profile.new') : t('profile.edit')))
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="lg"
    :label="title"
    @close="emit('close')"
  >
    <div class="flex min-h-0 flex-col">
      <header class="min-w-0 select-none p-5 pb-4 pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
        <p class="mt-0.5 text-explanation leading-5 text-fg-muted">
          {{ t('profile.hint') }}
        </p>
      </header>

      <!-- Only the fields scroll; the title and the buttons stay in place. -->
      <ScrollArea class="min-h-0" viewport-class="flex flex-col gap-3 px-5">
        <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
          <SettingsRow :label="t('profile.name')" :description="t('profile.nameDescription')">
            <template v-if="nameProblem" #detail>
              <InlineError :message="nameProblem" />
            </template>
            <TextInput
              v-model="draft.name"
              :focused="mode === 'create'"
              class="w-48 max-w-full"
              literal
              :aria-label="t('profile.name')"
            />
          </SettingsRow>
          <SettingsRow :label="t('profile.description')" :description="t('profile.descriptionDescription')">
            <TextInput
              v-model="draft.description"
              class="w-80 max-w-full"
              :aria-label="t('profile.description')"
            />
          </SettingsRow>
          <SettingsRow :label="t('profile.model')">
            <Dropdown
              :overlay-store="overlayStore"
              variant="field"
              size="sm"
              width="shrink"
              :trigger-label="t('profile.model')"
            >
              <template #trigger>
                <ModelChoiceTrigger :label="modelLabel" :missing="missing" />
              </template>
              <template #content>
                <ModelMenu
                  :providers="providers"
                  :models="models"
                  :settings="draft.model"
                  :stand-in="parentChoice"
                  without-context
                  @stand-in="chooseParentModel"
                  @change="changeModel"
                />
              </template>
            </Dropdown>
          </SettingsRow>
          <SettingsRow
            v-if="context"
            :label="t('profile.context')"
            :description="t('profile.contextDescription')"
          >
            <Dropdown
              :overlay-store="overlayStore"
              variant="field"
              size="sm"
              width="shrink"
              :trigger-label="t('profile.context')"
            >
              <template #trigger>
                <span class="min-w-0 truncate">{{ context.label }}</span>
              </template>
              <template #content>
                <ContextMenu
                  :options="context.options"
                  @select="draft.contextLimit = context.options[$event].tokens"
                />
              </template>
            </Dropdown>
          </SettingsRow>
          <SettingsRow :label="t('profile.instructions')" :description="t('profile.instructionsDescription')" />
          <div class="px-4 pb-3">
            <TextArea
              v-model="draft.instructions"
              mono
              :rows="7"
              :aria-label="t('profile.instructions')"
            />
          </div>
          <SettingsRow :label="t('profile.mayStart')" :description="t('profile.mayStartDescription')">
            <Switch v-model="draft.canSpawn" size="sm" class="ml-1" :aria-label="t('profile.mayStart')" />
          </SettingsRow>
        </div>

        <InlineError v-if="missing" :message="missing" />
      </ScrollArea>
      <InlineError v-if="error" class="px-5 pt-3" :message="error.message" :request-id="error.requestId" />
    </div>
    <template v-if="mode === 'edit'" #footer-leading>
      <Button variant="danger" :operation="deleteOperation" @click="emit('delete')">{{ uiText('common.delete') }}</Button>
    </template>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button variant="primary" :disabled="!canSave" :operation="saveOperation" @click="save">
        {{ mode === 'create' ? t('profile.create') : uiText('common.save') }}
      </Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { TriangleAlert } from '@lucide/vue'
import type { OverlayStore } from '../overlay/overlayStore'
import type { ModelInfo, ProviderInfo } from '../transport/protocol'
import ModelMenu from '../agent/ModelMenu.vue'
import { isFastMode } from '../agent/fast-mode'
import {
  applyModelChange,
  composerModel,
  firstEffort,
  initialModelSettings,
  offersEffort,
  type ModelSettingsChange,
} from '../agent/model-selection'
import { buildReasoningState, reasoningOptionLabel } from '../agent/reasoning'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'
import Dropdown from '../ui/Dropdown.vue'
import ScrollArea from '../ui/ScrollArea.vue'
import InlineError from '../ui/InlineError.vue'
import Segmented from '../ui/Segmented.vue'
import Switch from '../ui/Switch.vue'
import TextArea from '../ui/TextArea.vue'
import TextInput from '../ui/TextInput.vue'
import { ICON_PX } from '../ui/icon-metrics'
import SettingsRow from './SettingsRow.vue'
import { profileMissing, profileModelLabel, profileNameProblem } from './subagent-profiles'
import type { SettingsSubagentDraft } from './types'

/**
 * One subagent profile, created or edited in its own dialog: its name and
 * when to use it, its model (the parent's, or any provider's with its effort
 * and tier), its system prompt (the parent's, or text that replaces it), and
 * whether its children may spawn. The host saves; a refusal shows under the
 * fields, and the draft stays.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  mode: 'create' | 'edit'
  profile: SettingsSubagentDraft
  /** The providers and models the model menu offers. */
  providers: ProviderInfo[]
  models: Record<string, ModelInfo[]>
  pending?: boolean
  error?: string | null
}>()

const emit = defineEmits<{
  close: []
  save: [draft: SettingsSubagentDraft]
  delete: []
}>()

type Source = 'parent' | 'own'

/** The parts edited in place; the model and the prompt follow their source. */
function fields(profile: SettingsSubagentDraft) {
  return {
    name: profile.name,
    description: profile.description,
    canSpawn: profile.canSpawn,
  }
}

const draft = ref(fields(props.profile))
const modelSource = ref<Source>(props.profile.model ? 'own' : 'parent')
const promptSource = ref<Source>(props.profile.instructions === null ? 'parent' : 'own')
/** What each part held before the user chose the parent's, so choosing it back restores it. */
const ownModel = ref(props.profile.model ?? initialModelSettings())
const ownPrompt = ref(props.profile.instructions ?? '')

watch(
  () => [props.isOpen, props.profile] as const,
  ([open]) => {
    if (!open) {
      return
    }
    draft.value = fields(props.profile)
    modelSource.value = props.profile.model ? 'own' : 'parent'
    promptSource.value = props.profile.instructions === null ? 'parent' : 'own'
    ownModel.value = props.profile.model ?? initialModelSettings()
    ownPrompt.value = props.profile.instructions ?? ''
  },
)

const sources = [
  { value: 'parent', label: 'Parent’s' },
  { value: 'own', label: 'Own' },
] as const

const modelChosen = computed(() => ownModel.value.providerId !== '' && ownModel.value.modelId !== '')
const modelLabel = computed(() => {
  if (!modelChosen.value) {
    return 'Choose a model'
  }
  const settings = ownModel.value
  const listed = (props.models[settings.providerId] ?? []).find((model) => model.id === settings.modelId)
  const parts = [profileModelLabel(settings, props.providers, props.models)]
  const effort = settings.thinkingEffort
  const reasoning = buildReasoningState(listed)
  // An effort the model no longer offers reads as itself, never as the first one a menu would show.
  if (listed && effort !== null && !offersEffort(listed, effort)) {
    parts.push(effort)
  } else if (reasoning) {
    parts.push(reasoningOptionLabel(reasoning, effort))
  }
  if (isFastMode(listed, settings.serviceTierId)) {
    parts.push('Fast')
  }
  return parts.join(' · ')
})
const missing = computed(() =>
  modelSource.value === 'own' && modelChosen.value
    ? profileMissing(ownModel.value, props.providers, props.models)
    : null,
)

// Choosing an own model starts from the model a menu would show first, so the menu and the
// field never disagree about what is chosen.
watch(modelSource, (source) => {
  if (source !== 'own' || modelChosen.value) {
    return
  }
  const first = composerModel(props.providers, props.models, null, null).selected
  if (first) {
    ownModel.value = {
      providerId: first.providerId,
      modelId: first.modelId,
      thinkingEffort: firstEffort(first.model),
      serviceTierId: null,
    }
  }
})

function changeModel(change: ModelSettingsChange): void {
  ownModel.value = applyModelChange(ownModel.value, change)
}

/** What breaks the name rule, said under the field as the name is typed. */
const nameProblem = computed(() => profileNameProblem(draft.value.name.trim()))

const canSave = computed(() =>
  !props.pending &&
  draft.value.name.trim() !== '' &&
  nameProblem.value === null &&
  draft.value.description.trim() !== '' &&
  (modelSource.value === 'parent' || modelChosen.value) &&
  (promptSource.value === 'parent' || ownPrompt.value.trim() !== ''),
)

function save(): void {
  if (!canSave.value) {
    return
  }
  emit('save', {
    name: draft.value.name.trim(),
    description: draft.value.description.trim(),
    model: modelSource.value === 'own' ? { ...ownModel.value } : null,
    instructions: promptSource.value === 'own' ? ownPrompt.value : null,
    canSpawn: draft.value.canSpawn,
  })
}

const title = computed(() => (props.mode === 'create' ? 'New Profile' : 'Edit Profile'))
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
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          An agent names the profile with <span class="font-mono">demi agent spawn --profile</span>; its description tells the agent when to.
        </p>
      </header>

      <!-- Only the fields scroll; the title and the buttons stay in place. -->
      <ScrollArea :inert="pending" class="min-h-0" viewport-class="flex flex-col gap-3 px-5">
        <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
          <SettingsRow label="Name" description="Required. Lowercase letters, digits and hyphens, starting with a letter.">
            <template v-if="nameProblem" #detail>
              <InlineError :message="nameProblem" />
            </template>
            <TextInput
              v-model="draft.name"
              :focused="mode === 'create'"
              class="w-48 max-w-full"
              placeholder="explore"
              literal
              aria-label="Name"
            />
          </SettingsRow>
          <SettingsRow label="Description" description="Required. When the agent should use it, in one line.">
            <TextInput
              v-model="draft.description"
              class="w-80 max-w-full"
              placeholder="Use for finding code; it reports and changes nothing."
              aria-label="Description"
            />
          </SettingsRow>
          <SettingsRow label="Model" description="The parent’s, or one of any provider with its effort and tier.">
            <Segmented v-model="modelSource" size="sm" :options="sources" />
          </SettingsRow>
          <SettingsRow v-if="modelSource === 'own'" label="Chosen model" inset>
            <Dropdown
              :overlay-store="overlayStore"
              variant="field"
              size="sm"
              width="shrink"
              trigger-label="Model"
              placement="bottom-end"
            >
              <template #trigger>
                <span class="flex min-w-0 items-center gap-1.5">
                  <TriangleAlert v-if="missing" :size="ICON_PX.in24" class="shrink-0 text-on-warning" />
                  <span class="min-w-0 truncate">{{ modelLabel }}</span>
                </span>
              </template>
              <template #content>
                <ModelMenu
                  :providers="providers"
                  :models="models"
                  :settings="modelChosen ? ownModel : null"
                  without-context
                  @change="changeModel"
                />
              </template>
            </Dropdown>
          </SettingsRow>
          <SettingsRow label="System prompt" description="The parent’s instructions, or text that replaces them.">
            <Segmented v-model="promptSource" size="sm" :options="sources" />
          </SettingsRow>
          <div v-if="promptSource === 'own'" class="px-4 py-3">
            <TextArea
              v-model="ownPrompt"
              mono
              :rows="7"
              aria-label="System prompt"
              placeholder="You find code and report where it is. You change nothing."
            />
          </div>
          <SettingsRow label="Children may spawn" description="Off removes spawn, abort and resume from its children’s demi agent commands.">
            <Switch v-model="draft.canSpawn" size="sm" class="ml-1" aria-label="Children may spawn" />
          </SettingsRow>
        </div>

        <InlineError v-if="missing" :message="missing" />
      </ScrollArea>
      <InlineError v-if="error" class="px-5 pt-3" :message="error" />
    </div>
    <template v-if="mode === 'edit'" #footer-leading>
      <Button variant="danger" :disabled="pending" @click="emit('delete')">Delete</Button>
    </template>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button variant="primary" :disabled="!canSave" @click="save">
        {{ mode === 'create' ? 'Create Profile' : 'Save' }}
      </Button>
    </template>
  </Dialog>
</template>

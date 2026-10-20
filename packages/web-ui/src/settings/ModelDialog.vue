<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import type { ShownError } from '../infra/errors'
import { computed, ref, watch } from 'vue'
import { effortLabel } from './efforts'
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import type { Action, Operation } from '@demicodes/web-ui/ui/operations'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import TokenInput from '@demicodes/web-ui/ui/TokenInput.vue'
import SettingsRow from './SettingsRow.vue'
import {
  EXTENSION_PRESETS,
  THINKING_EFFORTS,
  type SettingsModelDraft,
} from './types'
import { formatTokens } from '../ui/token-count'

/**
 * One model, in its own dialog: Edit Model, which every model of an entry's
 * own list opens, the one just added from Add Model included, or the facts
 * of a catalog's model to read. Its fields come in one order, the id, the
 * name, the context window, the output limit, the thinking efforts, the Fast
 * tier and the files it reads, and every one is editable, the id too
 * (`crates/backend-user-shard/src/provider_settings.rs`). An id another model of the
 * list has is refused here, before anything is sent. Save keeps the change,
 * Cancel drops it, Delete Model removes the model. Tool calling is not a
 * setting: a model without it cannot drive the agent, so it is never offered.
 */
const props = defineProps<{
  /** Saves the model; Save works until it settles. */
  onSave?: Action<[model: SettingsModelDraft]>
  /** What Save does, to which model (`operations.ts`). */
  operation?: Operation
  /** Why the last save failed, with the request it came from. */
  error?: ShownError | null
  isOpen: boolean
  overlayStore: OverlayStore
  mode: 'edit' | 'view'
  model: SettingsModelDraft
  /** The ids of the entry's other models, which this one's id may not take. */
  takenIds?: readonly string[]
}>()

const emit = defineEmits<{
  close: []
  /** Delete Model: the host asks first, then removes it. */
  delete: []
}>()

const draft = ref<SettingsModelDraft>(clone(props.model))
watch(
  () => props.model,
  (next) => {
    draft.value = clone(next)
  },
)
// Each opening starts from the model as it is: Cancel dropped the edit.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      draft.value = clone(props.model)
    }
  },
)

function clone(m: SettingsModelDraft): SettingsModelDraft {
  return {
    ...m,
    efforts: [...m.efforts],
    extensions: m.extensions === null ? null : [...m.extensions],
  }
}

const editable = computed(() => props.mode === 'edit')
/** Whether the model is text, the model menu's: only a text model has a window, efforts and files. */
const text = computed(() => (draft.value.kind ?? 'text') === 'text')
const title = computed(() =>
  props.mode === 'edit' ? t('modelDialog.edit') : draft.value.name || draft.value.id,
)

/** Why the id cannot be saved, said as the user reads it; null when it can. */
const idProblem = computed(() =>
  props.takenIds?.includes(draft.value.id.trim()) ? t('modelDialog.duplicateId') : null,
)

function toggleEffort(effort: string) {
  const list = draft.value.efforts
  draft.value.efforts = list.includes(effort)
    ? list.filter((e) => e !== effort)
    : [...list, effort]
}

/** A preset is a set to pick from, so it is a checkbox: checked when every extension is in, mixed when some are. */
function presetState(extensions: string[]): {
  checked: boolean
  partial: boolean
} {
  const have = extensions.filter((e) =>
    draft.value.extensions?.includes(e),
  ).length
  return {
    checked: have === extensions.length,
    partial: have > 0 && have < extensions.length,
  }
}

function setPreset(extensions: string[], on: boolean) {
  const rest = (draft.value.extensions ?? []).filter(
    (e) => !extensions.includes(e),
  )
  draft.value.extensions = on ? [...rest, ...extensions] : rest
}

const canSave = computed(() => {
  const model = draft.value
  return (
    model.id.trim().length > 0 &&
    idProblem.value === null &&
    // An unknown window, as a model added by Custom has, is no fault.
    (model.contextWindow === null ||
      (Number.isInteger(model.contextWindow) && model.contextWindow > 0)) &&
    (model.outputLimit === null ||
      (Number.isInteger(model.outputLimit) &&
        model.outputLimit > 0 &&
        (model.contextWindow === null || model.outputLimit <= model.contextWindow)))
  )
})

function save(): void {
  const model = clone(draft.value)
  model.id = model.id.trim()
  void props.onSave?.(model)
}
</script>

<template>
  <!-- Reading is a short list of facts; editing needs room for descriptions. -->
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :size="editable ? 'lg' : 'md'"
    :label="title"
    @close="emit('close')"
  >
    <div class="flex min-h-0 flex-col">
      <header class="min-w-0 select-none p-5 pb-4 pr-10">
        <h3 class="truncate text-[15px] font-medium text-fg-emphasis">
          {{ title }}
        </h3>
        <p
          v-if="mode === 'view'"
          class="mt-0.5 font-mono text-[12px] text-fg-subtle"
        >
          {{ draft.id }}
        </p>
      </header>
      <ScrollArea
        class="min-h-0"
        :viewport-class="`flex flex-col gap-4 px-5 ${editable ? '' : 'pb-5'}`"
      >
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow v-if="editable" :label="t('modelDialog.id')">
            <template v-if="idProblem" #detail>
              <InlineError :message="idProblem" />
            </template>
            <TextInput
              v-model="draft.id"
              :aria-label="t('modelDialog.id')"
              literal
              class="w-64 max-w-full font-mono"
            />
          </SettingsRow>
          <SettingsRow
            :label="t('modelDialog.name')"
            :description="editable ? (text ? t('modelDialog.nameDescription') : t('modelDialog.nameDefault')) : undefined"
            :compact="!editable"
          >
            <TextInput
              v-if="editable"
              v-model="draft.name"
              :aria-label="t('modelDialog.name')"
              :placeholder="draft.id"
              class="w-64 max-w-full"
            />
            <span v-else class="text-chrome text-fg">{{
              draft.name || '—'
            }}</span>
          </SettingsRow>
          <!-- A model that is not text is only its ID and name (`packages/web-ui/src/settings/SettingsProvidersPage.vue`). -->
          <template v-if="text">
          <SettingsRow :label="t('modelDialog.contextWindow')" :compact="!editable">
            <TokenInput
              v-if="editable"
              v-model="draft.contextWindow"
              class="w-32"
            />
            <span v-else class="text-chrome tabular-nums text-fg">{{
              formatTokens(draft.contextWindow)
            }}</span>
          </SettingsRow>
          <SettingsRow
            :label="t('modelDialog.maxOutput')"
            :description="editable ? t('modelDialog.optional') : undefined"
            :compact="!editable"
          >
            <TokenInput
              v-if="editable"
              v-model="draft.outputLimit"
              class="w-32"
            />
            <span v-else class="text-chrome tabular-nums text-fg">{{
              formatTokens(draft.outputLimit)
            }}</span>
          </SettingsRow>
          <SettingsRow
            :label="t('modelDialog.reasoning')"
            :description="editable ? t('modelDialog.reasoningDescription') : undefined"
            :compact="!editable"
          >
            <!-- On a phone the efforts wrap rather than run past the row's start. -->
            <span v-if="editable" class="flex justify-end gap-2 phone:flex-wrap">
              <button
                v-for="effort in THINKING_EFFORTS"
                :key="effort"
                type="button"
                class="h-6 cursor-default select-none rounded-md px-2 text-control-sm transition-colors duration-200 ease-out phone:h-hit-sm phone:px-3"
                data-drawn-target
                :class="
                  draft.efforts.includes(effort)
                    ? 'bg-tint-accent text-on-accent'
                    : 'bg-hover text-fg-muted hover:text-fg'
                "
                @click="toggleEffort(effort)"
              >
                {{ effortLabel(effort) }}
              </button>
            </span>
            <span v-else class="text-chrome text-fg">{{
              draft.efforts.length ? draft.efforts.map(effortLabel).join(' · ') : t('modelDialog.none')
            }}</span>
          </SettingsRow>
          <SettingsRow
            :label="t('modelDialog.fastTier')"
            :description="editable ? t('modelDialog.fastTierDescription') : undefined"
            :compact="!editable"
          >
            <TextInput
              v-if="editable"
              :model-value="draft.fastTier ?? ''"
              literal
              class="w-32"
              @update:model-value="(v) => (draft.fastTier = v.trim() || null)"
            />
            <span v-else class="font-mono text-[12px] text-fg-muted">{{
              draft.fastTier ?? '—'
            }}</span>
          </SettingsRow>
          </template>
        </div>

        <div
          v-if="text"
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow
            :label="t('modelDialog.acceptedFiles')"
            :description="editable ? t('modelDialog.textAlways') : undefined"
            :compact="!editable"
          >
            <span
              v-if="!editable && !draft.extensions?.length"
              class="text-chrome text-fg"
              >{{ draft.extensions === null ? t('modelDialog.unknown') : t('modelDialog.textOnly') }}</span
            >
            <span v-else-if="!editable" class="flex flex-wrap justify-end gap-1"
              ><Tag v-for="ext in draft.extensions" :key="ext">{{
                ext
              }}</Tag></span
            >
          </SettingsRow>
          <template v-if="editable">
            <SettingsRow
              v-for="preset in EXTENSION_PRESETS"
              :key="preset.id"
              inset
              :label="preset.label"
              :description="preset.extensions.join(' ')"
            >
              <Checkbox
                :model-value="presetState(preset.extensions).checked"
                :partial="presetState(preset.extensions).partial"
                label=""
                :aria-label="t('modelDialog.accept', { label: preset.label.toLowerCase() })"
                @update:model-value="(on) => setPreset(preset.extensions, on)"
              />
            </SettingsRow>
          </template>
          <div
            v-if="editable && draft.extensions?.length"
            class="flex flex-wrap gap-1 px-4 py-3 phone:gap-2"
          >
            <Tag v-for="ext in draft.extensions" :key="ext">
              {{ ext }}
              <!-- On a phone the × is a small icon button's 28 px, inside a tag a little taller than its text. -->
              <button
                v-if="editable"
                type="button"
                class="ml-1 text-fg-subtle hover:text-fg phone:-my-1.5 phone:-mr-2 phone:ml-0 phone:size-hit-xs"
                :aria-label="t('modelDialog.removeExtension', { ext })"
                @click="
                  draft.extensions = (draft.extensions ?? []).filter(
                    (e) => e !== ext,
                  )
                "
              >
                ×
              </button>
            </Tag>
          </div>
        </div>
      </ScrollArea>
      <InlineError v-if="error" class="px-5 pt-3" :message="error.message" :request-id="error.requestId" />
    </div>
    <template v-if="editable" #footer-leading>
      <Button variant="danger" @click="emit('delete')">{{ t('modelDialog.delete') }}</Button>
    </template>
    <template v-if="editable" #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
      <Button
        variant="primary"
        :disabled="!canSave"
        :operation="operation"
        @click="save"
        >{{ uiText('common.save') }}</Button
      >
    </template>
  </Dialog>
</template>

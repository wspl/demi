<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import { computed, ref, watch } from 'vue'
import { upperFirst } from '@demicodes/utils'
import type { OverlayStore } from '../overlay/overlayStore'
import Autocomplete from '@demicodes/web-ui/ui/Autocomplete.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Checkbox from '@demicodes/web-ui/ui/Checkbox.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import TokenInput from '@demicodes/web-ui/ui/TokenInput.vue'
import SettingsRow from './SettingsRow.vue'
import { useModelSearch } from './model-search'
import {
  EXTENSION_PRESETS,
  THINKING_EFFORTS,
  type SettingsModelDraft,
  type SettingsModelMatch,
  type SettingsModelSearch,
} from './types'
import { formatTokens } from '../ui/token-count'

/**
 * One model, in its own dialog: added, edited, or viewed as the catalog
 * describes it. Adding asks only for the model's name at first: typing
 * searches models.dev (`search`), the list under the field offers *Custom*
 * first and then the matches, and choosing one shows every field of the model,
 * filled from the match or empty but for the typed name, each still editable
 * (`models.md` § Adding a model from models.dev). Tool calling is not a
 * setting: a model without it cannot drive the agent, so it is never offered.
 */
const props = defineProps<{
  pending?: boolean
  /** Why the last save failed, with the request it came from. */
  error?: ShownError | null
  isOpen: boolean
  overlayStore: OverlayStore
  mode: 'create' | 'edit' | 'view'
  model: SettingsModelDraft
  /** Asks models.dev for the matches of the typed name; without it only *Custom* is offered. */
  search?: SettingsModelSearch
}>()

const emit = defineEmits<{
  close: []
  save: [model: SettingsModelDraft]
}>()

const draft = ref<SettingsModelDraft>(clone(props.model))
watch(
  () => props.model,
  (next) => {
    draft.value = clone(next)
  },
)

function clone(m: SettingsModelDraft): SettingsModelDraft {
  return {
    ...m,
    efforts: [...m.efforts],
    extensions: m.extensions === null ? null : [...m.extensions],
  }
}

const editable = computed(() => props.mode !== 'view')
/**
 * Adding shows the fields once a match or *Custom* was chosen, or when the
 * dialog opens on a model already filled in (a save that failed); the other
 * modes show them at once.
 */
const startsChosen = () => props.mode !== 'create' || props.model.id.trim() !== ''
const chosen = ref(startsChosen())
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      chosen.value = startsChosen()
    }
  },
)
const { results: matches, failed: searchFailed } = useModelSearch(
  computed(() => draft.value.name),
  () => props.search,
)

/** A row of the name field's list: *Custom* first, then the matches. */
type Choice = { kind: 'custom' } | { kind: 'match'; match: SettingsModelMatch }
const choices = computed<Choice[]>(() => [
  { kind: 'custom' },
  ...matches.value.map((match): Choice => ({ kind: 'match', match })),
])

function choiceKey(choice: Choice): string {
  return choice.kind === 'custom' ? 'custom' : choice.match.ref
}

function choose(choice: Choice): void {
  if (choice.kind === 'match') {
    draft.value = clone(choice.match.model)
  } else {
    draft.value = clone({
      ...props.model,
      name: draft.value.name,
    })
  }
  chosen.value = true
}

const title = computed(() =>
  props.mode === 'create'
    ? 'Add Model'
    : props.mode === 'edit'
      ? 'Edit Model'
      : draft.value.name || draft.value.id,
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
    chosen.value &&
    model.id.trim().length > 0 &&
    model.contextWindow !== null &&
    Number.isInteger(model.contextWindow) &&
    model.contextWindow > 0 &&
    (model.outputLimit === null ||
      (Number.isInteger(model.outputLimit) &&
        model.outputLimit > 0 &&
        model.outputLimit <= model.contextWindow))
  )
})
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
        :inert="pending"
        class="min-h-0"
        :viewport-class="`flex flex-col gap-4 px-5 ${editable ? '' : 'pb-5'}`"
      >
        <div
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow v-if="mode === 'create'" label="Name">
            <template v-if="searchFailed" #detail>
              <InlineError message="Could not search models.dev." />
            </template>
            <Autocomplete
              v-model="draft.name"
              :items="choices"
              :item-key="choiceKey"
              list-label="Models"
              placeholder="Gemini 3.8 Flash"
              :focused="!chosen"
              class="w-80 max-w-full"
              @choose="choose"
            >
              <template #item="{ item }">
                <span v-if="item.kind === 'custom'" class="min-w-0 flex-1 truncate">Custom</span>
                <template v-else>
                  <span class="min-w-0 flex-1 truncate">{{ item.match.model.name }}</span>
                  <span class="shrink-0 text-fg-subtle">{{ item.match.vendor }}</span>
                  <span class="w-12 shrink-0 text-right tabular-nums text-fg-subtle">{{
                    formatTokens(item.match.model.contextWindow)
                  }}</span>
                </template>
              </template>
            </Autocomplete>
          </SettingsRow>
          <SettingsRow v-if="editable && chosen" label="Model ID">
            <TextInput
              v-model="draft.id"
              placeholder="model-id"
              literal
              class="w-64 max-w-full font-mono"
              :readonly="mode === 'edit'"
            />
          </SettingsRow>
          <SettingsRow
            v-if="mode !== 'create'"
            label="Display name"
            :description="editable ? 'Optional' : undefined"
            :compact="!editable"
          >
            <TextInput
              v-if="editable"
              v-model="draft.name"
              :placeholder="draft.id"
              class="w-64 max-w-full"
            />
            <span v-else class="text-chrome text-fg">{{
              draft.name || '—'
            }}</span>
          </SettingsRow>
          <SettingsRow v-if="chosen" label="Context window" :compact="!editable">
            <TokenInput
              v-if="editable"
              v-model="draft.contextWindow"
              placeholder="128"
              class="w-32"
            />
            <span v-else class="text-chrome tabular-nums text-fg">{{
              formatTokens(draft.contextWindow)
            }}</span>
          </SettingsRow>
          <SettingsRow
            v-if="chosen"
            label="Max output"
            :description="editable ? 'Optional' : undefined"
            :compact="!editable"
          >
            <TokenInput
              v-if="editable"
              v-model="draft.outputLimit"
              placeholder="8"
              class="w-32"
            />
            <span v-else class="text-chrome tabular-nums text-fg">{{
              formatTokens(draft.outputLimit)
            }}</span>
          </SettingsRow>
          <SettingsRow
            v-if="chosen"
            label="Reasoning"
            :description="editable ? 'First is the default.' : undefined"
            :compact="!editable"
          >
            <template v-if="editable">
              <button
                v-for="effort in THINKING_EFFORTS"
                :key="effort"
                type="button"
                class="h-6 cursor-default select-none rounded-md px-2 text-[12px] transition-colors duration-200 ease-out"
                :class="
                  draft.efforts.includes(effort)
                    ? 'bg-tint-accent text-on-accent'
                    : 'bg-hover text-fg-muted hover:text-fg'
                "
                @click="toggleEffort(effort)"
              >
                {{ upperFirst(effort) }}
              </button>
            </template>
            <span v-else class="text-chrome text-fg">{{
              draft.efforts.length ? draft.efforts.map(upperFirst).join(' · ') : 'None'
            }}</span>
          </SettingsRow>
          <SettingsRow
            v-if="chosen"
            label="Fast tier"
            :description="editable ? 'Service tier ID. Optional' : undefined"
            :compact="!editable"
          >
            <TextInput
              v-if="editable"
              :model-value="draft.fastTier ?? ''"
              placeholder="fast"
              literal
              class="w-32"
              @update:model-value="(v) => (draft.fastTier = v.trim() || null)"
            />
            <span v-else class="font-mono text-[12px] text-fg-muted">{{
              draft.fastTier ?? '—'
            }}</span>
          </SettingsRow>
        </div>

        <div
          v-if="chosen"
          class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float"
        >
          <SettingsRow
            label="Accepted files"
            :description="editable ? 'Text is always accepted.' : undefined"
            :compact="!editable"
          >
            <span
              v-if="!editable && !draft.extensions?.length"
              class="text-chrome text-fg"
              >{{ draft.extensions === null ? 'Unknown' : 'Text only' }}</span
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
                :aria-label="`Accept ${preset.label.toLowerCase()}`"
                @update:model-value="(on) => setPreset(preset.extensions, on)"
              />
            </SettingsRow>
          </template>
          <div
            v-if="editable && draft.extensions?.length"
            class="flex flex-wrap gap-1 px-4 py-3"
          >
            <Tag v-for="ext in draft.extensions" :key="ext">
              {{ ext }}
              <button
                v-if="editable"
                type="button"
                class="ml-1 text-fg-subtle hover:text-fg"
                :aria-label="`Remove ${ext}`"
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
    <template v-if="editable" #footer>
      <Button @click="emit('close')">Cancel</Button>
      <Button
        variant="primary"
        :disabled="!canSave"
        :loading="pending"
        @click="emit('save', clone(draft))"
        >{{ mode === 'create' ? 'Add' : 'Save' }}</Button
      >
    </template>
  </Dialog>
</template>

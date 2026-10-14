<script setup lang="ts">
import type { ShownError } from '../infra/errors'
import { computed, ref, watch } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import Autocomplete from '@demicodes/web-ui/ui/Autocomplete.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import Dialog from '@demicodes/web-ui/ui/Dialog.vue'
import InlineError from '@demicodes/web-ui/ui/InlineError.vue'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import SettingsRow from './SettingsRow.vue'
import { useModelSearch } from './model-search'
import type { SettingsModelDraft, SettingsModelMatch, SettingsModelSearch } from './types'
import { formatTokens } from '../ui/token-count'

/**
 * Add Model is a search and nothing else (`models.md` § Adding a model from
 * models.dev): typing searches models.dev, and the list under the field
 * offers *Custom* first and then the matches with their vendor and context
 * window. Choosing a row adds the model at once, filled from the match, or
 * with the typed text as its id and name for *Custom*; the host then opens it
 * in Edit Model, the dialog every model opens.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** Asks models.dev for the matches of the typed text; without it only *Custom* is offered. */
  search?: SettingsModelSearch
  /** Whether a chosen model is being added: the field waits for it. */
  adding?: boolean
  /** Why the last add failed, with the request it came from. */
  error?: ShownError | null
}>()

const emit = defineEmits<{
  close: []
  /** The model chosen, to add as it is. */
  add: [model: SettingsModelDraft]
}>()

const text = ref('')
// Each opening starts with an empty field.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      text.value = ''
    }
  },
  { immediate: true },
)

const { results: matches, failed } = useModelSearch(text, () => props.search)

/** A row of the list: *Custom* first, then the matches. */
type Choice = { kind: 'custom' } | { kind: 'match'; match: SettingsModelMatch }
const choices = computed<Choice[]>(() => [
  ...(text.value.trim() ? [{ kind: 'custom' } as const] : []),
  ...matches.value.map((match): Choice => ({ kind: 'match', match })),
])

function choiceKey(choice: Choice): string {
  return choice.kind === 'custom' ? 'custom' : choice.match.ref
}

function choose(choice: Choice): void {
  if (props.adding) {
    return
  }
  if (choice.kind === 'match') {
    emit('add', { ...choice.match.model, efforts: [...choice.match.model.efforts] })
    return
  }
  const typed = text.value.trim()
  emit('add', {
    id: typed,
    name: typed,
    contextWindow: null,
    outputLimit: null,
    efforts: [],
    extensions: [],
    fastTier: null,
  })
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="lg" label="Add Model" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">Add Model</h3>
        <p class="mt-0.5 text-[13px] leading-5 text-fg-muted">
          Search models.dev, or choose Custom to add a model by the name you type.
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow label="Model">
          <template v-if="failed" #detail>
            <InlineError message="Could not search models.dev." />
          </template>
          <span v-if="adding" class="flex items-center gap-1.5 text-[12px] text-fg-subtle" role="status"
            ><IndeterminateSpinner :size="12" /> Adding…</span
          >
          <Autocomplete
            v-model="text"
            :items="choices"
            :item-key="choiceKey"
            list-label="Models"
            focused
            placeholder="Search"
            :disabled="adding"
            class="w-80 max-w-full"
            @choose="choose"
          >
            <template #item="{ item }">
              <template v-if="item.kind === 'custom'">
                <span class="shrink-0">Custom</span>
                <span class="min-w-0 flex-1 truncate text-fg-subtle">{{ text.trim() }}</span>
              </template>
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
      </div>
      <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
    </div>
    <template #footer>
      <Button @click="emit('close')">Cancel</Button>
    </template>
  </Dialog>
</template>

<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
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
 * Add Model is a search and nothing else
 * (`crates/provider-registry/src/vendors.rs`): typing searches models.dev.
 *
 * The list under the field offers *Custom* first and then the matches with their
 * vendor and context window. Choosing a row adds the model at once, filled from
 * the match, or with the typed text as its id and name for *Custom*. The host
 * then opens it in Edit Model, the dialog every model opens.
 *
 * Several IDs, pasted from a server's model list or typed apart by commas or
 * spaces, make *Custom* add them all at once, each with its ID as its name; the
 * dialog then closes without opening any of them.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** Asks models.dev for the matches of the typed text; without it only *Custom* is offered. */
  search?: SettingsModelSearch
  /** The text the field starts with on each opening; a specimen's way to show a search under way. */
  startText?: string
  /** Whether a chosen model is being added: the field waits for it. */
  adding?: boolean
  /** Why the last add failed, with the request it came from. */
  error?: ShownError | null
  /** The title, the name of the button that opened it, such as Add Image Model; Add Model without it. */
  title?: string
}>()

const emit = defineEmits<{
  close: []
  /** The model chosen, to add as it is. */
  add: [model: SettingsModelDraft]
  /** Several custom models typed or pasted at once, to add as they are. */
  addSeveral: [models: SettingsModelDraft[]]
}>()

const text = ref('')
const field = ref<{ show(): void } | null>(null)

/** The IDs the field holds, split at lines, spaces and commas. */
const ids = computed(() => [...new Set(text.value.split(/[\s,]+/).filter(Boolean))])

/** A paste of several lines keeps each line an ID, which a single-line field would join. */
function paste(event: ClipboardEvent): void {
  const lines = (event.clipboardData?.getData('text') ?? '').split(/\r?\n/).map((line) => line.trim()).filter(Boolean)
  if (lines.length < 2) {
    return
  }
  event.preventDefault()
  text.value = lines.join(', ')
  // The paste it replaced would have opened the list; this one does it itself.
  field.value?.show()
}

function customDraft(id: string): SettingsModelDraft {
  return { id, name: id, contextWindow: null, outputLimit: null, efforts: [], extensions: [], fastTier: null }
}

// Each opening starts with the same text, empty in the product.
watch(
  () => props.isOpen,
  (open) => {
    if (open) {
      text.value = props.startText ?? ''
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
  if (ids.value.length > 1) {
    emit('addSeveral', ids.value.map(customDraft))
    return
  }
  emit('add', customDraft(text.value.trim()))
}
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="lg" :label="props.title ?? t('addModel.title')" @close="emit('close')">
    <div class="flex flex-col gap-4 px-5 pt-5">
      <header class="select-none pr-10">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ props.title ?? t('addModel.title') }}</h3>
        <p class="mt-0.5 text-explanation leading-5 text-fg-muted">
          {{ t('addModel.hint') }}
        </p>
      </header>
      <div class="settings-card @container overflow-hidden rounded-xl border border-line bg-surface-float">
        <SettingsRow :label="t('addModel.model')">
          <template v-if="failed" #detail>
            <InlineError :message="t('addModel.searchFailed')" />
          </template>
          <span v-if="adding" class="flex items-center gap-1.5 text-detail text-fg-subtle" role="status"
            ><IndeterminateSpinner :size="12" /> {{ t('addModel.adding') }}</span
          >
          <Autocomplete
            ref="field"
            v-model="text"
            :items="choices"
            :item-key="choiceKey"
            :list-label="t('addModel.list')"
            focused
            :placeholder="t('addModel.search')"
            :disabled="adding"
            class="w-80 max-w-full"
            @choose="choose"
            @paste="paste"
          >
            <template #item="{ item }">
              <span v-if="item.kind === 'custom'" class="flex min-w-0 flex-1 items-center gap-2">
                <span class="shrink-0">{{ ids.length > 1 ? t('addModel.several', { count: ids.length }) : t('addModel.custom') }}</span>
                <span class="min-w-0 flex-1 truncate text-fg-subtle">{{ text.trim() }}</span>
              </span>
              <span v-else class="flex min-w-0 flex-1 items-center gap-2">
                <span class="min-w-0 flex-1 truncate">{{ item.match.model.name }}</span>
                <span class="shrink-0 text-fg-subtle">{{ item.match.vendor }}</span>
                <span class="w-12 shrink-0 text-right tabular-nums text-fg-subtle">{{
                  formatTokens(item.match.model.contextWindow)
                }}</span>
              </span>
            </template>
          </Autocomplete>
        </SettingsRow>
      </div>
      <InlineError v-if="error" :message="error.message" :request-id="error.requestId" />
    </div>
    <template #footer>
      <Button @click="emit('close')">{{ uiText('common.cancel') }}</Button>
    </template>
  </Dialog>
</template>

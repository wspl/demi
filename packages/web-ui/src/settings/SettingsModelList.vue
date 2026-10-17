<script setup lang="ts">
import { Search } from '@lucide/vue'
import AsyncRegion from '../ui/AsyncRegion.vue'
import TextInput from '@demicodes/web-ui/ui/TextInput.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import type { SentenceText } from '../ui/ui-text'

/**
 * A provider's list of models inside its card, as every section of the
 * entry's page shows one, text models and each kind of media alike: a
 * header row with the filter and the list's buttons (`actions`), such as
 * Add Model, then the rows, then, when no row shows, why in plain text,
 * never red, since nothing has failed.
 */
const props = withDefaults(defineProps<{
  /** The filter's placeholder; without it the list has no header row. */
  filterPlaceholder?: string
  /** Shown in place of the rows when none shows; null while some do. */
  empty: string | null
  /** Whether the rows are loaded; a failure offers Retry. */
  state?: 'loading' | 'ready' | 'failed'
  loadingLabel?: SentenceText
}>(), {
  filterPlaceholder: undefined,
  state: 'ready',
  loadingLabel: undefined,
})

const emit = defineEmits<{ retry: [] }>()

const filter = defineModel<string>('filter', { default: '' })
</script>

<template>
  <!-- The filter is bare text; its hit area is the whole row height. -->
  <div v-if="props.filterPlaceholder" class="flex min-h-10 items-center gap-3 px-3 py-1">
    <TextInput
      size="sm"
      v-model="filter"
      search
      :placeholder="props.filterPlaceholder"
      bare
      class="min-w-0 flex-1"
    >
      <template #prefix><Search :size="ICON_PX.in24" /></template>
    </TextInput>
    <slot name="actions" />
  </div>
  <AsyncRegion :state="props.state" :label="props.loadingLabel" @retry="emit('retry')">
    <slot />
    <div
      v-if="props.empty !== null"
      class="flex select-none justify-center px-4 py-6 text-center text-[13px] text-fg-subtle"
    >
      {{ props.empty }}
    </div>
  </AsyncRegion>
</template>

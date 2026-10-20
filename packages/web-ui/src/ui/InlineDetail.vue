<script setup lang="ts">
import { computed } from 'vue'
import type { InlineDetailText } from './inline-detail'

/**
 * A value followed by its detail on one line (`InlineDetailText`), as a field's
 * trigger and a menu row show them: the value in the line's own text, the detail
 * smaller and in the secondary color.
 *
 * The detail truncates first, inside its brackets, which stay; the value only once
 * the detail is gone. The two sit on one text baseline: they clip with
 * `overflow: clip`, which keeps an element's text baseline, where
 * `overflow: hidden` would make a flex item align by its bottom edge.
 */
const props = defineProps<{
  detail: InlineDetailText
}>()

const parts = computed(() =>
  typeof props.detail === 'string' ? { text: props.detail, open: '', close: '' } : props.detail,
)
</script>

<template>
  <span class="flex min-w-0 flex-1 items-baseline gap-1.5">
    <span class="flex min-w-0 max-w-full shrink-0 items-baseline gap-2"><slot /></span>
    <span data-detail class="flex min-w-0 items-baseline whitespace-nowrap text-detail text-fg-subtle">
      <span v-if="parts.open" class="shrink-0">{{ parts.open }}</span>
      <span class="min-w-0 overflow-clip text-ellipsis">{{ parts.text }}</span>
      <span v-if="parts.close" class="shrink-0">{{ parts.close }}</span>
    </span>
  </span>
</template>

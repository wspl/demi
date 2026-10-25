<script setup lang="ts">
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import { t } from './locales/text'

/**
 * The mark of a row that works (`provideRowTarget`): a spinner of the size of
 * an `xs` icon button, in the place where the row's controls stand.
 *
 * A row that can work keeps the mark's room while it rests, empty, so nothing
 * in it moves when its work starts or ends.
 */
withDefaults(defineProps<{
  /** False while the row rests: the room stays, empty. */
  working?: boolean
}>(), { working: true })
</script>

<template>
  <span
    class="flex size-hit-xs shrink-0 items-center justify-center text-fg-muted"
    :role="working ? 'status' : undefined"
    :aria-label="working ? t('rowSpinner.working') : undefined"
  >
    <IndeterminateSpinner v-if="working" :size="14" />
  </span>
</template>

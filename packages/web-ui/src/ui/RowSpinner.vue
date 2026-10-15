<script setup lang="ts">
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import { t } from './locales/text'

/**
 * The mark of a row that works (`provideRowTarget`): a spinner of the size of
 * an `xs` icon button, in the place where the row's controls stand.
 */
</script>

<template>
  <span class="flex size-hit-xs shrink-0 items-center justify-center text-fg-muted" role="status" :aria-label="t('rowSpinner.working')">
    <IndeterminateSpinner :size="14" />
  </span>
</template>

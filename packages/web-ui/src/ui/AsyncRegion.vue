<script setup lang="ts">
import RegionStatus from './RegionStatus.vue'
import type { SentenceText } from './ui-text'

/**
 * A region whose content arrives later. While loading or failed it shows the
 * shared `RegionStatus` pane in place of the slot; a failure always offers
 * Retry, which returns the pane to loading at once (`RegionStatus`).
 */
withDefaults(
  defineProps<{
    state?: 'loading' | 'ready' | 'failed'
    label?: SentenceText
    error?: string
    /** The failure reason under the error line, in the caller's words. */
    detail?: string | null
    /** Retry, as `RegionStatus` takes it. */
    onRetry?: () => unknown
  }>(),
  {
    state: 'ready',
    label: 'Loading…',
    error: 'Could not load this content.',
    detail: null,
    onRetry: () => {},
  },
)
</script>

<template>
  <slot v-if="state === 'ready'" />
  <RegionStatus
    v-else
    class="min-h-24"
    :status="state"
    :label="state === 'loading' ? label : error"
    :loading-label="label"
    :detail="detail"
    :on-retry="onRetry"
  />
</template>

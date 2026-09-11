<script setup lang="ts">
import RegionStatus from './RegionStatus.vue'

/**
 * A region whose content arrives later. While loading or failed it shows the
 * shared `RegionStatus` pane in place of the slot; a failure always offers Retry.
 */
withDefaults(
  defineProps<{
    state?: 'loading' | 'ready' | 'failed'
    label?: string
    error?: string
    /** The failure reason under the error line, in the caller's words. */
    detail?: string | null
  }>(),
  {
    state: 'ready',
    label: 'Loading…',
    error: 'Could not load this content.',
    detail: null,
  },
)
defineEmits<{ retry: [] }>()
</script>

<template>
  <slot v-if="state === 'ready'" />
  <RegionStatus
    v-else
    class="min-h-24"
    :busy="state === 'loading'"
    :failed="state === 'failed'"
    :label="state === 'loading' ? label : error"
    :detail="state === 'failed' ? detail : null"
    :action="state === 'failed' ? 'Retry' : undefined"
    @action="$emit('retry')"
  />
</template>

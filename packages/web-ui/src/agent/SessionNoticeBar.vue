<script setup lang="ts">
import Button from '../ui/Button.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'

/**
 * A state of the whole session that the reader chose or can leave (archived,
 * no model can send), or one that ends by itself (the Cloud is resetting). It
 * replaces the composer input and offers the one action that leaves the
 * state; a state that ends by itself has none and says it is in progress.
 * Failures are not notices; they are `ErrorNotice` records in the transcript.
 */
defineProps<{
  label: string
  action?: string
  /** The state is in progress and ends by itself. */
  busy?: boolean
}>()

const emit = defineEmits<{
  action: []
}>()
</script>

<template>
  <div
    class="session-notice-bar flex w-full items-center gap-3 rounded-lg bg-surface-raised px-3 py-2 text-chrome text-fg-muted"
    role="status"
  >
    <IndeterminateSpinner v-if="busy" :size="14" class="shrink-0 text-fg-subtle" />
    <span class="min-w-0 flex-1 break-words">{{ label }}</span>
    <Button v-if="action" size="sm" class="shrink-0" @click="emit('action')">{{
      action
    }}</Button>
  </div>
</template>

<script setup lang="ts">
import Button from '../ui/Button.vue'

/**
 * A state of the whole session that the reader chose or can leave: archived,
 * no model can send. It replaces the composer input and offers the one action
 * that leaves the state. Failures are not notices; they are `ErrorNotice`
 * records in the transcript.
 */
defineProps<{
  label: string
  action?: string
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
    <span class="min-w-0 flex-1 break-words">{{ label }}</span>
    <Button v-if="action" size="sm" class="shrink-0" @click="emit('action')">{{
      action
    }}</Button>
  </div>
</template>

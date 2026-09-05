<script setup lang="ts">
import { computed } from 'vue'
import type { ConversationStatus } from './conversation-status'

const props = defineProps<{
  status: ConversationStatus
}>()

const dotColor = computed(() => {
  if (props.status === 'active') return 'var(--on-accent)'
  if (props.status === 'error' || props.status === 'aborted') return 'var(--on-danger)'
  if (props.status === 'done') return 'var(--on-success)'
  return null
})
</script>

<template>
  <span
    class="absolute -right-px -top-px size-1.5 rounded-full border transition-[background-color,opacity] duration-300"
    :class="[
      dotColor ? 'opacity-100 border-surface-base' : 'opacity-0 border-transparent',
      status === 'active' && 'animate-pulse',
    ]"
    :style="{ backgroundColor: dotColor ?? 'transparent' }"
  />
</template>

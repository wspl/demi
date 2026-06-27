<script setup lang="ts">
import { computed } from 'vue'
import type { ConversationStatus } from './conversation-status'

const props = defineProps<{
  status: ConversationStatus
}>()

const dotColor = computed(() => {
  if (props.status === 'active') return 'var(--color-blue-400)'
  if (props.status === 'error' || props.status === 'aborted') return 'var(--color-red-400)'
  if (props.status === 'done') return 'var(--color-emerald-400)'
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

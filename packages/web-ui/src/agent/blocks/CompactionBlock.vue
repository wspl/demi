<script setup lang="ts">
import { computed } from 'vue'
import { formatTokens } from '../../ui/token-count'

const props = defineProps<{
  summary: string
  summaryTokens: number
  isCompacting: boolean
  createdAt: string
}>()

const label = computed(() => {
  if (props.isCompacting)
    return 'Compacting context...'
  return `Context compacted to ~${formatTokens(props.summaryTokens)} tokens`
})
</script>

<template>
  <div class="flex items-center gap-3 px-[var(--agent-pad-x,2rem)]">
    <div class="h-px flex-1 bg-line-subtle" />
    <span
      v-if="isCompacting"
      class="thinking-shimmer text-chrome text-fg-subtle"
    >{{ label }}</span>
    <span
      v-else
      class="text-chrome text-fg-subtle"
    >{{ label }}</span>
    <div class="h-px flex-1 bg-line-subtle" />
  </div>
</template>

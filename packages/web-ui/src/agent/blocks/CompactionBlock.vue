<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{
  summary: string
  summaryTokens: number
  isCompacting: boolean
  createdAt: string
}>()

function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}K`
  return String(tokens)
}

const label = computed(() => {
  if (props.isCompacting) return 'Compacting context...'
  return `Context compacted to ~${formatTokens(props.summaryTokens)} tokens`
})
</script>

<template>
  <div class="flex items-center gap-3 px-[var(--agent-pad-x,2rem)]">
    <div class="h-px flex-1 bg-overlay/6" />
    <span
      v-if="isCompacting"
      class="thinking-shimmer text-[11px] font-medium tracking-wide text-fg-subtle"
    >{{ label }}</span>
    <span
      v-else
      class="text-[11px] font-medium tracking-wide text-fg-subtle"
    >{{ label }}</span>
    <div class="h-px flex-1 bg-overlay/6" />
  </div>
</template>

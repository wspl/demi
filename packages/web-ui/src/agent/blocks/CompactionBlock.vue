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
  <div class="flex items-center gap-3 px-8">
    <div class="h-px flex-1 bg-overlay/6" />
    <span
      v-if="isCompacting"
      class="compaction-shimmer text-[11px] font-medium tracking-wide text-fg-subtle"
    >{{ label }}</span>
    <span
      v-else
      class="text-[11px] font-medium tracking-wide text-fg-subtle"
    >{{ label }}</span>
    <div class="h-px flex-1 bg-overlay/6" />
  </div>
</template>

<style scoped>
.compaction-shimmer {
  background: linear-gradient(
    90deg,
    rgba(163, 163, 163, 0.5) 0%,
    rgba(200, 200, 200, 0.7) 40%,
    rgba(220, 220, 220, 0.8) 50%,
    rgba(200, 200, 200, 0.7) 60%,
    rgba(163, 163, 163, 0.5) 100%
  );
  background-size: 200% 100%;
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  animation: shimmer 2s ease-in-out infinite;
}

@keyframes shimmer {
  0% { background-position: 100% 0; }
  100% { background-position: -100% 0; }
}
</style>

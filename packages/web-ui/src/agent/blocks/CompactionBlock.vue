<script setup lang="ts">
import { formatTokens } from '../../ui/token-count'

/** Where the context was compacted: a rule with the summary's size in it. */
defineProps<{
  summaryTokens: number
}>()
</script>

<template>
  <div class="flex items-center gap-3 px-[var(--agent-pad-x,2rem)]">
    <div class="h-px flex-1 bg-line-subtle" />
    <span class="text-chrome text-fg-subtle">Context compacted to ~{{ formatTokens(summaryTokens) }} tokens</span>
    <div class="h-px flex-1 bg-line-subtle" />
  </div>
</template>

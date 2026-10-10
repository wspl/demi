<script setup lang="ts">
import IndeterminateSpinner from '../../ui/IndeterminateSpinner.vue'
import { formatTokens } from '../../ui/token-count'

/**
 * Where the context was compacted: a rule with what happened in it. While
 * the pass runs it says so, at the place the finished rule takes; a failed
 * pass is its error record instead (`runtime.md` § Block types).
 */
defineProps<{
  /** The summary's size; unknown only for a marker whose boundary an edit removed. */
  summaryTokens?: number
  running?: boolean
}>()
</script>

<template>
  <div class="flex items-center gap-3 px-[var(--agent-pad-x,2rem)]" :role="running ? 'status' : undefined">
    <div class="h-px flex-1 bg-line-subtle" />
    <span v-if="running" class="flex items-center gap-2 text-row text-fg-row">
      <IndeterminateSpinner />
      Compacting context…
    </span>
    <span v-else-if="summaryTokens !== undefined" class="text-row text-fg-row">Context compacted to ~{{ formatTokens(summaryTokens) }} tokens</span>
    <span v-else class="text-row text-fg-row">Context compacted</span>
    <div class="h-px flex-1 bg-line-subtle" />
  </div>
</template>

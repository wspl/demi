<script setup lang="ts">
import { t } from '../locales/text'
import Shimmer from '../../ui/Shimmer.vue'
import { formatTokens } from '../../ui/token-count'

/**
 * Where the context was compacted: a rule with what happened in it. While
 * the pass runs it says so, its words shimmering as a running row's do, at
 * the place the finished rule takes, which rolls in over it; a failed pass is
 * its error record instead.
 */
defineProps<{
  /**
   * The summary's size; unknown only for a marker whose boundary an edit
   * removed.
   */
  summaryTokens?: number
  running?: boolean
}>()
</script>

<template>
  <div class="flex items-center gap-3 px-[var(--agent-pad-x,2rem)]" :role="running ? 'status' : undefined">
    <div class="h-px flex-1 bg-line-subtle" />
    <Shimmer v-if="running" active class="items-center text-row text-fg-row">
      <span>{{ t('transcript.compaction.running') }}</span>
    </Shimmer>
    <span v-else-if="summaryTokens !== undefined" class="text-row text-fg-row">{{ t('transcript.compaction.doneTo', { tokens: formatTokens(summaryTokens) }) }}</span>
    <span v-else class="text-row text-fg-row">{{ t('transcript.compaction.done') }}</span>
    <div class="h-px flex-1 bg-line-subtle" />
  </div>
</template>

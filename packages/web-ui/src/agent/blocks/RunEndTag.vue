<script setup lang="ts">
import Tag from '../../ui/Tag.vue'
import Tooltip from '../../ui/Tooltip.vue'
import type { SentenceText } from '../../ui/ui-text'

/**
 * How a row's work went wrong, after its title: red Failed, or grey Stopped,
 * Stopping… while a stop waits for the runner, or Lost. It never shrinks, so a
 * long title gives up its end to it. `hint` is the tooltip that says more (a
 * command's exit code, why it was lost).
 */
export type RunEndKind = 'failed' | 'stopped' | 'stopping' | 'lost'

defineProps<{
  kind: RunEndKind
  hint?: SentenceText
}>()

const LABEL: Record<RunEndKind, string> = {
  failed: 'Failed',
  stopped: 'Stopped',
  stopping: 'Stopping…',
  lost: 'Lost',
}
</script>

<template>
  <Tooltip v-if="hint" :content="hint" class="flex shrink-0 items-center">
    <Tag :tone="kind === 'failed' ? 'danger' : 'neutral'">{{ LABEL[kind] }}</Tag>
  </Tooltip>
  <Tag v-else :tone="kind === 'failed' ? 'danger' : 'neutral'" class="shrink-0">{{ LABEL[kind] }}</Tag>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { t } from '../locales/text'
import Tag from '../../ui/Tag.vue'
import Tooltip from '../../ui/Tooltip.vue'
import type { SentenceText } from '../../ui/ui-text'

/**
 * How a row's work went wrong, after its title:
 *
 * - red Failed
 * - grey Stopped
 * - grey Stopping… while a stop waits for the runner
 * - grey Waiting for MacBook Pro while a command's start waits for its offline
 *   Host, `host`
 * - grey Lost
 *
 * The tag never shrinks, so a long title gives up its end to it. `hint` is the
 * tooltip that says more (a command's exit code, why it was lost).
 */
export type RunEndKind = 'failed' | 'stopped' | 'stopping' | 'waiting' | 'lost'

const props = defineProps<{
  kind: RunEndKind
  hint?: SentenceText
  /** The device a waiting command's start waits for. */
  host?: string
}>()

const label = computed(() => t(`transcript.runEnd.${props.kind}`, { host: props.host ?? '' }))

</script>

<template>
  <Tooltip v-if="hint" :content="hint" class="flex shrink-0 items-center">
    <Tag :tone="kind === 'failed' ? 'danger' : 'neutral'">{{ label }}</Tag>
  </Tooltip>
  <Tag v-else :tone="kind === 'failed' ? 'danger' : 'neutral'" class="shrink-0">{{ label }}</Tag>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import Tag from '@demicodes/web-ui/ui/Tag.vue'
import type { ToolCallBlock } from '../block-types'
import { runningTime } from '../command-end'
import { useLiveCalls } from '../live-calls'
import { shellRowTime } from '../terminals'
import { useElapsedTime } from '../../composables/useElapsedTime'

/**
 * The grey tag of how long a command has run, once past 3 seconds,
 * counting up: of the command of `calls` that still runs, the one that has
 * run longest when several do; nothing once none runs. A shell row wears
 * it for its own command, and an ended run's row for a command still
 * running in it.
 */
const props = defineProps<{ calls: readonly ToolCallBlock[] }>()

const liveCalls = useLiveCalls()
// The earliest start among the commands that still run, by their live frames.
const since = computed(() => {
  let earliest: number | null = null
  for (const call of props.calls) {
    const time = shellRowTime(call, liveCalls(call.toolUseId))
    if (time?.kind === 'running' && (earliest === null || time.since < earliest))
      earliest = time.since
  }
  return earliest
})
const runningFor = useElapsedTime(() => since.value ?? Number.NaN, () => since.value !== null)
const tag = computed(() => (since.value === null ? null : runningTime(runningFor.value)))
</script>

<template>
  <Tag v-if="tag" class="shrink-0 tabular-nums">{{ tag }}</Tag>
</template>

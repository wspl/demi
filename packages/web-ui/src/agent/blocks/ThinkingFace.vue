<script setup lang="ts">
import { computed } from 'vue'
import { thinkingFaceLabel } from '../thinking-label'
import { useElapsedTime } from '../../composables/useElapsedTime'

/**
 * What a thinking row says, wherever it stands: Thinking and for how long
 * while it streams, Thought for how long once it ended, from its start to
 * its end as the block records it.
 */
const props = defineProps<{
  streaming: boolean
  createdAt: string
  /**
   * When the thinking's latest piece came, which is its end once it no
   * longer streams; null for a block that does not record it.
   */
  endedAt: string | null
}>()

const elapsedMs = useElapsedTime(
  () => Date.parse(props.createdAt),
  () => props.streaming,
  // While it streams the clock runs on: its latest piece is not its end.
  () => (!props.streaming && props.endedAt ? Date.parse(props.endedAt) : null),
)
const label = computed(() => thinkingFaceLabel(props.streaming, elapsedMs.value))
</script>

<template>
  <span class="min-w-0 truncate">{{ label }}</span>
</template>

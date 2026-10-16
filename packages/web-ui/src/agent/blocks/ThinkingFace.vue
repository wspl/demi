<script setup lang="ts">
import { computed } from 'vue'
import { thinkingFaceLabel } from '../thinking-label'
import { useElapsedTime } from '../../composables/useElapsedTime'

/**
 * What a thinking row says, wherever it stands (`rendering.md` § Work
 * groups): Thinking and for how long while it streams, Thought for how long
 * once it ended, from its start to the start of the block after it.
 */
const props = defineProps<{
  streaming: boolean
  createdAt: string
  /** The start of the block after the thinking, when it ended; none while it streams or nothing followed. */
  endedAt?: string | null
}>()

const elapsedMs = useElapsedTime(
  () => Date.parse(props.createdAt),
  () => props.streaming,
  () => (props.endedAt ? Date.parse(props.endedAt) : null),
)
const label = computed(() => thinkingFaceLabel(props.streaming, elapsedMs.value))
</script>

<template>
  <span class="min-w-0 truncate">{{ label }}</span>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useElapsedTime } from '../composables/useElapsedTime'
import { agentRunLine, type AgentRun } from './subagents'

/**
 * The line that says what a child runs on and for how long
 * (`subagents.md` § What an agent runs on), its time following the page's
 * clock while the round runs. One line, its end giving way first, unless
 * it `wraps`, as in a tip, which has room for it whole; the caller gives
 * its color and size.
 */
const props = defineProps<{
  run: AgentRun
  wraps?: boolean
}>()

const elapsedMs = useElapsedTime(
  () => Date.parse(props.run.startedAt),
  () => props.run.phase === 'running',
  () => (props.run.endedAt ? Date.parse(props.run.endedAt) : null),
)
const line = computed(() => agentRunLine(props.run, elapsedMs.value ?? 0))
</script>

<template>
  <span class="block min-w-0" :class="wraps ? '' : 'truncate'">{{ line }}</span>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import AgentRunLine from './AgentRunLine.vue'
import { agentPhaseWords } from './agent-links'
import type { AgentRun } from './subagents'

/**
 * What a tip names a subagent with, an agent link's and a receipt name's
 * (`subagents.md` § Linking an agent, § What an agent runs on):
 * *Subagent 4 · Finished*, its description, then what it is doing and runs on, both groups on one line.
 */
const props = defineProps<{
  number: number
  description: string
  run: AgentRun
}>()

const title = computed(() => `Subagent ${props.number} · ${agentPhaseWords(props.run.phase)}`)
</script>

<template>
  <div>
    <div class="font-medium text-fg-emphasis">{{ title }}</div>
    <div class="text-fg-muted">{{ description }}</div>
    <AgentRunLine class="mt-0.5 text-fg-muted" :run="run" plain />
  </div>
</template>

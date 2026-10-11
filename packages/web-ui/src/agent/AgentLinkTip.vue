<script setup lang="ts">
import { computed } from 'vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import AgentMark from './AgentMark.vue'
import { agentPhaseWords, useMessageAgents } from './agent-links'

/**
 * What a message's link to an agent shows beyond its text
 * (`subagents.md` § Linking an agent), for the link `element` a render
 * made: the agent's mark in the place the render left for it, with its live
 * state, and the tip that names it, *Subagent 4 · Finished* and its
 * description; for a number the conversation does not have, the tip that
 * says so.
 */
const props = defineProps<{
  element: HTMLElement
  number: number
}>()

const agents = useMessageAgents()
const agent = computed(() => agents()?.find(props.number) ?? null)
const markPlace = computed(() => props.element.querySelector<HTMLElement>('[data-agent-mark]'))
const title = computed(() => {
  const found = agent.value
  if (!found)
    return `No agent ${props.number} in this conversation`
  return found.phase ? `Subagent ${found.number} · ${agentPhaseWords(found.phase)}` : found.description
})
</script>

<template>
  <Teleport v-if="agent && markPlace" :to="markPlace">
    <AgentMark :phase="agent.phase" :size="ICON_PX.in28" />
  </Teleport>
  <Tooltip :trigger="element" :content="agent?.phase ? undefined : title">
    <template v-if="agent?.phase" #overlay>
      <div class="font-medium text-fg-emphasis">{{ title }}</div>
      <div class="text-fg-muted">{{ agent.description }}</div>
    </template>
  </Tooltip>
</template>

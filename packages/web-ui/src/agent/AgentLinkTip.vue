<script setup lang="ts">
import { computed } from 'vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import AgentMark from './AgentMark.vue'
import AgentTip from './AgentTip.vue'
import { useMessageAgents } from './agent-links'

/**
 * What a message's link to an agent shows beyond its text
 * (`subagents.md` § Linking an agent), for the link `element` a render
 * made: the agent's mark in the place the render left for it, with its live
 * state, and the tip that names it, *Subagent 4 · Finished*, its
 * description and what it runs on; for the main agent, its name; for a
 * number the conversation does not have, the tip that says so.
 */
const props = defineProps<{
  element: HTMLElement
  number: number
}>()

const agents = useMessageAgents()
const agent = computed(() => agents()?.find(props.number) ?? null)
const markPlace = computed(() => props.element.querySelector<HTMLElement>('[data-agent-mark]'))
const plain = computed(() => agent.value?.description ?? `No agent ${props.number} in this conversation`)
</script>

<template>
  <Teleport v-if="agent && markPlace" :to="markPlace">
    <AgentMark :phase="agent.run?.phase ?? null" :size="ICON_PX.in28" />
  </Teleport>
  <Tooltip :trigger="element" :content="agent?.run ? undefined : plain" wide>
    <template v-if="agent?.run" #overlay>
      <AgentTip :number="agent.number" :description="agent.description" :run="agent.run" />
    </template>
  </Tooltip>
</template>

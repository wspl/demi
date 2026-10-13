<script setup lang="ts">
import { computed } from 'vue'
import { ICON_PX } from '../ui/icon-metrics'
import AgentMark from './AgentMark.vue'
import { useMessageAgents } from './agent-links'

/**
 * What a message's link to an agent shows beyond its text
 * (`subagents.md` § Linking an agent), for the link `element` a render
 * made: the agent's mark in the place the render left for it, with its live
 * state. The link has no tip: its text already names the agent.
 */
const props = defineProps<{
  element: HTMLElement
  number: number
}>()

const agents = useMessageAgents()
const agent = computed(() => agents()?.find(props.number) ?? null)
const markPlace = computed(() => props.element.querySelector<HTMLElement>('[data-agent-mark]'))
</script>

<template>
  <Teleport v-if="agent && markPlace" :to="markPlace">
    <AgentMark :phase="agent.run?.phase ?? null" :size="ICON_PX.in28" />
  </Teleport>
</template>

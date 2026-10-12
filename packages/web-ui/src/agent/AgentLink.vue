<script setup lang="ts">
import { computed } from 'vue'
import Tooltip from '../ui/Tooltip.vue'
import { ICON_PX } from '../ui/icon-metrics'
import AgentMark from './AgentMark.vue'
import AgentRunLine from './AgentRunLine.vue'
import { useMessageAgents } from './agent-links'

/**
 * A link to an agent of the conversation outside a message, *agent 7* with
 * the agent's mark, as a message's `agent:7` link reads
 * (`subagents.md` § Linking an agent): the same look (`agent-link` in the
 * stylesheet), the same tip, and a click that opens the Agents window on
 * that agent's tab. A number the conversation does not have is its text
 * alone, which names that in its tip.
 */
const props = defineProps<{ number: number }>()

const agents = useMessageAgents()
const agent = computed(() => agents()?.find(props.number) ?? null)

function open(): void {
  agents()?.open(props.number)
}
</script>

<template>
  <Tooltip
    v-if="agent"
    tag="span"
    wide
  >
    <span
      role="button"
      tabindex="0"
      class="agent-link icon-link"
      @click="open"
      @keydown.enter.prevent="open"
      @keydown.space.prevent="open"
    >
      <span class="agent-link-lead">
        <span class="agent-link-mark"><AgentMark :phase="agent.run?.phase ?? null" :size="ICON_PX.in28" /></span>agent {{ number }}
      </span>
    </span>
    <template #overlay>
      <AgentRunLine v-if="agent.run" :run="agent.run" plain />
      <div v-else>{{ agent.description }}</div>
    </template>
  </Tooltip>
  <Tooltip v-else tag="span" :content="`No agent ${number} in this conversation`">
    <span>agent {{ number }}</span>
  </Tooltip>
</template>

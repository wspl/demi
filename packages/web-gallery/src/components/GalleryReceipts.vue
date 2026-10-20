<script setup lang="ts">
import { ref } from 'vue'
import DockWindowArea from '@demicodes/web-ui/agent/DockWindowArea.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import { dockWindowShare } from '../dock-window-share'
import { provideOriginRevealer } from '@demicodes/web-ui/agent/agent-receipts'
import { agentLinkWindow, findAgent, provideMessageAgents } from '@demicodes/web-ui/agent/agent-links'
import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import { productWould } from '../product-would'
import { useAgentContext } from '../agent-context'

/** What a child's context card in the Agents window shows and does beside its usage. */
const agentContext = useAgentContext()

/**
 * Receipt specimens over the gallery's children, as a session holds them:
 *
 * - A click on an agent's name opens the Agents window over them on that
 *   agent's tab, a finished one as Completed picks it, at the block the
 *   receipt came from, as the product's session does.
 * - The main agent's name says what the product would show in the main
 *   transcript.
 * - A message's link to an agent opens the window on its tab, and leaves it
 *   as it is when it shows that tab, as the product's session does.
 */
const props = defineProps<{ agents: readonly SubagentRecord[] }>()
const emit = defineEmits<{
  abort: []
  abortAgent: [id: string]
}>()

const open = ref(false)
const activeId = ref<string | null>(null)
const reveal = ref<TranscriptReveal | null>(null)
// The main agent's transcript is not among the specimens: the product would bring the block into view there.
provideOriginRevealer(({ node, blockId }) => {
  if (node === null) {
    return blockId === null ? undefined : () => productWould('The Main Transcript Shows the Call That Sent the Message')
  }
  return () => {
    activeId.value = node
    open.value = true
    reveal.value = blockId === null ? null : { node, blockId }
  }
})
provideMessageAgents(() => ({
  find: (number) => findAgent(props.agents, number),
  open: (number) => {
    const next = agentLinkWindow({ open: open.value, activeId: activeId.value }, props.agents, number)
    if (number === 0 && !open.value) {
      productWould('The Main Transcript Comes into View')
    }
    if (next) {
      open.value = next.open
      activeId.value = next.activeId
    }
  },
  // Stop Agent in a link's menu stops it as its tab's does.
  stop: (number) => {
    const agent = props.agents.find((candidate) => candidate.number === number)
    if (agent) {
      emit('abortAgent', agent.id)
    }
  },
}))
</script>

<template>
  <div class="relative">
    <slot />
    <!-- The window stands over the receipts as the product's stands over the session, at a session window's height. -->
    <DockWindowArea v-model:share="dockWindowShare" class="absolute inset-x-0 top-0 h-[40rem]">
      <SubagentPanel
        v-model:open="open"
        v-model:active-id="activeId"
        :agents="agents"
        :reveal="reveal"
        @revealed="reveal = null"
        @abort="emit('abort')"
        @abort-agent="(id) => emit('abortAgent', id)"
        v-bind="agentContext.panel"
      />
    </DockWindowArea>
  </div>
</template>

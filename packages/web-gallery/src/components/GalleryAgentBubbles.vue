<script setup lang="ts">
import { reactive, ref } from 'vue'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import DockWindowArea from '@demicodes/web-ui/agent/DockWindowArea.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { provideOriginRevealer } from '@demicodes/web-ui/agent/agent-receipts'
import { agentLinkWindow, findAgent, provideMessageAgents } from '@demicodes/web-ui/agent/agent-links'
import type { TranscriptReveal } from '@demicodes/web-ui/agent/history'
import { dockWindowShare } from '../dock-window-share'
import { productWould } from '../product-would'
import { useAgentContext } from '../agent-context'
import { BUBBLES_ROOT, bubbleAgents, bubblesMainBlocks } from '../fixtures/subagents'

/**
 * The messages an agent receives from other agents, in its own view: the Agents
 * window on Tidy the release notes, or the main agent's transcript under the
 * window.
 *
 * - `child`: its brief, its work, a bubble from the main agent and one from
 *   its sibling Review the notes, named above it.
 * - `child-report`: the same, with the report of its own child Check the
 *   links, which keeps its row.
 * - `main`: the main agent's view, where a message from its child keeps its
 *   row.
 *
 * The main agent's transcript is under the window in each, as the product's
 * session is.
 *
 * A sender's name opens the window on that agent's tab at the call that sent
 * the message, as the product's session does; the main agent's says what the
 * product would show in the main transcript.
 */
const props = defineProps<{ view: 'child' | 'child-report' | 'main' }>()

const agentContext = useAgentContext()
const agents = reactive(bubbleAgents(props.view === 'child-report'))
const open = ref(props.view !== 'main')
const activeId = ref<string | null>(agents[0]!.id)
const reveal = ref<TranscriptReveal | null>(null)

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
  find: (number) => findAgent(agents, number),
  open: (number) => {
    const next = agentLinkWindow({ open: open.value, activeId: activeId.value }, agents, number)
    if (number === 0) {
      productWould('The Main Transcript Comes into View')
    }
    if (next) {
      open.value = next.open
      activeId.value = next.activeId
    }
  },
  stop: (number) => abortAgent(agents.find((agent) => agent.number === number)?.id ?? ''),
}))

/** Stop Agent: a running agent stops; these have all finished, so it is said. */
function abortAgent(id: string): void {
  const agent = agents.find((candidate) => candidate.id === id)
  if (agent?.phase === 'running') {
    agent.phase = 'aborted'
    agent.endedAt = new Date().toISOString()
  } else {
    productWould('The Agent Has Already Finished')
  }
}

/** Stop All: every running agent stops. */
function abortAll(): void {
  for (const agent of agents) {
    if (agent.phase === 'running') {
      abortAgent(agent.id)
    }
  }
}
</script>

<template>
  <div class="flex flex-col gap-2">
    <div class="flex flex-wrap gap-2">
      <Button variant="ghost" size="sm" :disabled="open && activeId === agents[0]!.id" @click="open = true; activeId = agents[0]!.id">
        {{ view === 'main' ? 'Open Tidy the Release Notes' : 'Open Again' }}
      </Button>
    </div>
    <div class="relative h-[40rem]">
      <div class="gallery-frame h-full bg-surface">
        <AgentMessageList
          class="h-full"
          :conversation-id="BUBBLES_ROOT"
          :blocks="bubblesMainBlocks()"
          :pending-steers="[]"
          :queue="[]"
          phase="idle"
          :bottom-offset="0"
          :persisted-scroll-state="undefined"
          read-only
        />
      </div>
      <!-- The window stands over the main transcript as the product's stands over the session. -->
      <DockWindowArea v-model:share="dockWindowShare" class="absolute inset-x-0 top-0 h-full">
        <SubagentPanel
          v-model:open="open"
          v-model:active-id="activeId"
          :agents="agents"
          :reveal="reveal"
          @revealed="reveal = null"
          @abort="abortAll"
          @abort-agent="abortAgent"
          v-bind="agentContext.panel"
        />
      </DockWindowArea>
    </div>
  </div>
</template>

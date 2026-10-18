<script setup lang="ts">
import { ref } from 'vue'
import { Play } from '@lucide/vue'
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionDockChip from '@demicodes/web-ui/agent/SessionDockChip.vue'
import { t as agentText } from '@demicodes/web-ui/agent/locales/text'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import { dockWindowShare } from '../dock-window-share'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import AssistantTextBlock from '@demicodes/web-ui/agent/blocks/AssistantTextBlock.vue'
import { ICON_PX } from '@demicodes/web-ui/ui/icon-metrics'
import { agentLinkWindow, findAgent, provideMessageAgents } from '@demicodes/web-ui/agent/agent-links'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { useSessionPanels } from '@demicodes/web-ui/agent/useSessionPanels'
import { agentLinksMarkdown } from '../fixtures/subagents'
import { productWould } from '../product-would'
import GalleryComposer from './GalleryComposer.vue'
import { useAgentContext } from '../agent-context'

/** What a child's context card in the Agents window shows and does beside its usage. */
const agentContext = useAgentContext()

/**
 * A dock with the agents chip over a conversation's children, as the
 * product's session has it: the chip shows while a child runs and opens the
 * Agents panel the way the product's does, a second click closing it. The
 * reply above links the children, so a finished one opens from its link
 * once no chip shows, as in the product's transcript.
 */
const props = defineProps<{
  agents: readonly SubagentRecord[]
  terminals: readonly TerminalRecord[]
}>()

const emit = defineEmits<{
  abort: []
  abortAgent: [id: string]
}>()

const { agentsOpen, activeSubagentId, toggleAgents, close } = useSessionPanels(
  () => props.agents,
  () => props.terminals,
  () => 'gallery-agents',
)
provideMessageAgents(() => ({
  find: (number) => findAgent(props.agents, number),
  open: (number) => {
    const next = agentLinkWindow({ open: agentsOpen.value, activeId: activeSubagentId.value }, props.agents, number)
    if (number === 0) {
      productWould('The Main Transcript Comes into View')
    }
    if (!next) {
      return
    }
    if (next.open) {
      activeSubagentId.value = next.activeId
    } else {
      close()
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
const surface = ref<InstanceType<typeof SessionSurface>>()
const createdAt = new Date(Date.now() - 60_000).toISOString()
</script>

<template>
  <div class="gallery-frame h-[26rem] w-full bg-surface">
    <SessionSurface ref="surface" v-model:dock-window-share="dockWindowShare">
      <div
        class="flex h-full flex-col justify-end"
        :style="{ paddingBottom: `${surface?.dockHeight ?? 0}px` }"
      >
        <AssistantTextBlock :content="agentLinksMarkdown" :created-at="createdAt" />
      </div>
      <template #dock>
        <SessionDock>
          <template #chips>
            <!-- Resume is no window's control: a press on it closes the newest window and resumes. -->
            <SessionDockChip @click="productWould('Resume the Turn')">
              <Play :size="ICON_PX.in28" />
              {{ agentText('dock.resume') }}
            </SessionDockChip>
            <AgentsChip
              :agents="agents"
              :open="agentsOpen"
              @open="toggleAgents"
            />
          </template>
          <GalleryComposer />
        </SessionDock>
      </template>
      <template #overDock>
        <SubagentPanel
          v-model:open="agentsOpen"
          v-model:active-id="activeSubagentId"
          :agents="agents"
          :terminals="terminals"
          @abort="emit('abort')"
          @abort-agent="(id) => emit('abortAgent', id)"
          v-bind="agentContext.panel"
        />
      </template>
    </SessionSurface>
  </div>
</template>

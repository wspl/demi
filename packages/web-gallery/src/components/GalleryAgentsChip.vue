<script setup lang="ts">
import AgentsChip from '@demicodes/web-ui/agent/AgentsChip.vue'
import SessionDock from '@demicodes/web-ui/agent/SessionDock.vue'
import SessionSurface from '@demicodes/web-ui/agent/SessionSurface.vue'
import SubagentPanel from '@demicodes/web-ui/agent/SubagentPanel.vue'
import type { SubagentRecord } from '@demicodes/web-ui/agent/subagents'
import type { TerminalRecord } from '@demicodes/web-ui/agent/terminals'
import { useSessionPanels } from '@demicodes/web-ui/agent/useSessionPanels'
import GalleryComposer from './GalleryComposer.vue'

/**
 * A dock with the agents chip over a conversation's children, as the
 * product's session has it: the chip opens the Agents panel the way the
 * product's does, and a second click closes it.
 */
const props = defineProps<{
  agents: readonly SubagentRecord[]
  terminals: readonly TerminalRecord[]
}>()

const emit = defineEmits<{
  abort: []
  abortAgent: [id: string]
}>()

const { activeSubagentId, toggleAgents } = useSessionPanels(
  () => props.agents,
  () => props.terminals,
)
</script>

<template>
  <div class="gallery-frame h-[26rem] w-full bg-surface">
    <SessionSurface>
      <template #dock>
        <SessionDock>
          <template #chips>
            <AgentsChip
              :agents="agents"
              :open="activeSubagentId !== null"
              @open="toggleAgents"
            />
          </template>
          <GalleryComposer placeholder="Ask Demi…" />
        </SessionDock>
      </template>
      <template #overDock>
        <SubagentPanel
          v-model:active-id="activeSubagentId"
          :agents="agents"
          :terminals="terminals"
          :dismiss-outside="false"
          @abort="emit('abort')"
          @abort-agent="(id) => emit('abortAgent', id)"
        />
      </template>
    </SessionSurface>
  </div>
</template>

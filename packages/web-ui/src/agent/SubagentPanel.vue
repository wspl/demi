<script setup lang="ts">
import { computed } from 'vue'
import { Square } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import Tooltip from '../ui/Tooltip.vue'
import AgentMessageList from './AgentMessageList.vue'
import SessionOverlay from './SessionOverlay.vue'
import SubagentHistoryMenu from './SubagentHistoryMenu.vue'
import TabItem from './TabItem.vue'
import { subagentPanelTabs, subagentStatus, type SubagentRecord } from './subagents'

const props = withDefaults(
  defineProps<{
    agents: readonly SubagentRecord[]
    dismissOutside?: boolean
  }>(),
  {
    dismissOutside: true,
  },
)

const emit = defineEmits<{
  /** Stop all: every live child. */
  abort: []
  /** The close on a running tab: that child and its subtree. */
  abortAgent: [id: string]
}>()
const activeId = defineModel<string | null>('activeId', { required: true })

const tabs = computed(() => subagentPanelTabs(props.agents, activeId.value))
const active = computed(
  () =>
    tabs.value.find((agent) => agent.id === activeId.value) ?? tabs.value[0] ?? null,
)

function activate(id: string): void {
  activeId.value = id
}

function closePanel(): void {
  activeId.value = null
}

// Closing a running tab aborts the child; closing a finished one puts it away.
function closeTab(agent: SubagentRecord): void {
  if (agent.phase === 'running') {
    emit('abortAgent', agent.id)
    return
  }
  activeId.value = tabs.value.filter((tab) => tab.id !== agent.id).at(-1)?.id ?? null
}
</script>

<template>
  <SessionOverlay
    :open="tabs.length > 0"
    :dismiss-outside="dismissOutside"
    @close="closePanel"
  >
    <template #tabs>
      <TabItem
        v-for="agent in tabs"
        :key="agent.id"
        :tab="{ id: agent.id, title: agent.name }"
        :is-active="agent.id === active?.id"
        :status="subagentStatus(agent.phase)"
        mark="bot"
        @pointerdown="activate(agent.id)"
        @close="closeTab(agent)"
      />
    </template>
    <template #trailing>
      <Tooltip
        v-if="agents.some((agent) => agent.phase === 'running')"
        content="Stop all agents"
      >
        <IconButton
          :icon="Square"
          aria-label="Stop all agents"
          @click="emit('abort')"
        />
      </Tooltip>
      <SubagentHistoryMenu
        :agents="agents"
        :active-id="activeId"
        @select="activate"
      />
    </template>
    <AgentMessageList
      v-if="active"
      :conversation-id="active.id"
      :blocks="active.blocks"
      :pending-steers="[]"
      :queue="[]"
      :phase="active.phase === 'running' ? 'running' : 'idle'"
      :bottom-offset="16"
      :persisted-scroll-state="undefined"
      read-only
    />
  </SessionOverlay>
</template>

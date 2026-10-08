<script setup lang="ts">
import { computed } from 'vue'
import { CircleStop } from '@lucide/vue'
import Button from '../ui/Button.vue'
import { ICON_PX } from '../ui/icon-metrics'
import Tooltip from '../ui/Tooltip.vue'
import AgentMessageList from './AgentMessageList.vue'
import SessionOverlay from './SessionOverlay.vue'
import SubagentHistoryMenu from './SubagentHistoryMenu.vue'
import TabItem from './TabItem.vue'
import { provideLiveCalls } from './live-calls'
import { subagentPanelTabs, subagentStatus, type SubagentRecord } from './subagents'
import { callTerminal, type TerminalRecord } from './terminals'

const props = withDefaults(
  defineProps<{
    agents: readonly SubagentRecord[]
    /** The conversation's commands: a child's running calls show theirs. */
    terminals?: readonly TerminalRecord[]
    dismissOutside?: boolean
  }>(),
  {
    terminals: () => [],
    dismissOutside: true,
  },
)

const emit = defineEmits<{
  /** Stop All: every live child. */
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
provideLiveCalls((toolUseId) =>
  active.value ? callTerminal(props.terminals, active.value.id, toolUseId) : undefined,
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
        :title="agent.name"
        :is-active="agent.id === active?.id"
        :status="subagentStatus(agent.phase)"
        mark="bot"
        @select="activate(agent.id)"
        @close="closeTab(agent)"
      />
    </template>
    <template #trailing>
      <Tooltip
        v-if="agents.some((agent) => agent.phase === 'running')"
        content="Stop all agents"
      >
        <Button
          size="sm"
          variant="ghost"
          aria-label="Stop all agents"
          @click="emit('abort')"
        >
          <CircleStop :size="ICON_PX.in24" aria-hidden="true" />
          Stop All
        </Button>
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
      :node="active.id"
      :blocks="active.blocks"
      :failures="active.failures"
      :pending-steers="[]"
      :queue="[]"
      :phase="active.phase === 'running' ? 'running' : 'idle'"
      :bottom-offset="16"
      :persisted-scroll-state="undefined"
      read-only
    />
  </SessionOverlay>
</template>

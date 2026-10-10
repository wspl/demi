<script setup lang="ts">
import { computed, watch } from 'vue'
import { CircleStop, X } from '@lucide/vue'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import AgentMessageList from './AgentMessageList.vue'
import SessionOverlay from './SessionOverlay.vue'
import SubagentHistoryMenu from './SubagentHistoryMenu.vue'
import TabItem from './TabItem.vue'
import { provideLiveCalls } from './live-calls'
import { subagentPanelTabs, subagentStatus, type SubagentRecord } from './subagents'
import { callTerminal, type TerminalRecord } from './terminals'
import { shownWindow, windowEdges, type TranscriptReveal } from './history'

const props = withDefaults(
  defineProps<{
    agents: readonly SubagentRecord[]
    /** The conversation's commands: a child's running calls show theirs. */
    terminals?: readonly TerminalRecord[]
    dismissOutside?: boolean
    /** A block of a child's transcript to bring into view and mark for a moment, on that child's tab. */
    reveal?: TranscriptReveal | null
  }>(),
  {
    terminals: () => [],
    dismissOutside: true,
  },
)

const emit = defineEmits<{
  /** Stop All: every live child. */
  abort: []
  /** Stop Agent in a live tab's menu: that child and its subtree. */
  abortAgent: [id: string]
  /**
   * Read a page of the child's transcript: its latest when the panel shows
   * a child it holds none of, or the one before the window shown as the
   * reader nears its start (`web-application.md` § Transcript windows).
   */
  read: [id: string, at: 'latest' | 'before']
  /** The block `reveal` names is in view. */
  revealed: []
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
// A child's transcript shows its latest window, and earlier pages as the reader scrolls up.
const shown = computed(() => active.value ? shownWindow(active.value.history, null) : null)
const shownEdges = computed(() => active.value && shown.value ? windowEdges(active.value.history, shown.value) : null)
watch(
  () => active.value,
  (agent) => {
    if (agent && agent.history.windows.length === 0) {
      emit('read', agent.id, 'latest')
    }
  },
  { immediate: true },
)

function activate(id: string): void {
  activeId.value = id
}

function closePanel(): void {
  activeId.value = null
}

const anyRunning = computed(() => props.agents.some((agent) => agent.phase === 'running'))

function agentOf(id: string): SubagentRecord | undefined {
  return tabs.value.find((agent) => agent.id === id)
}

// A finished child's tab puts it away; a running one's tab stays until the child is stopped.
function closeTab(id: string): void {
  activeId.value = tabs.value.filter((tab) => tab.id !== id).at(-1)?.id ?? null
}
</script>

<template>
  <SessionOverlay
    :open="tabs.length > 0"
    :dismiss-outside="dismissOutside"
    @close="closePanel"
  >
    <template #tabs="{ openTabMenu }">
      <TabItem
        v-for="agent in tabs"
        :key="agent.id"
        :title="agent.name"
        :is-active="agent.id === active?.id"
        :status="subagentStatus(agent.phase)"
        mark="bot"
        :closable="false"
        @select="activate(agent.id)"
        @contextmenu="openTabMenu($event, agent.id)"
      />
    </template>
    <template #tabMenu="{ id }">
      <MenuItem
        :icon="CircleStop"
        label="Stop Agent"
        :disabled="agentOf(id)?.phase !== 'running'"
        @select="emit('abortAgent', id)"
      />
      <MenuDivider />
      <MenuItem
        :icon="X"
        label="Close Tab"
        :disabled="agentOf(id)?.phase === 'running'"
        @select="closeTab(id)"
      />
    </template>
    <template #stripMenu>
      <MenuItem
        :icon="CircleStop"
        label="Stop All"
        :disabled="!anyRunning"
        @select="emit('abort')"
      />
    </template>
    <template #trailing>
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
      :blocks="shown?.blocks ?? []"
      :at-start="shownEdges?.atStart ?? true"
      :at-end="true"
      :pending-calls="active.pendingCalls"
      :failures="active.failures"
      :pending-steers="[]"
      :queue="[]"
      :phase="active.phase === 'running' ? 'running' : 'idle'"
      :bottom-offset="16"
      :persisted-scroll-state="undefined"
      read-only
      :reveal-block-id="reveal?.node === active.id ? reveal.blockId : null"
      @revealed="emit('revealed')"
      @read-before="emit('read', active.id, 'before')"
    />
  </SessionOverlay>
</template>

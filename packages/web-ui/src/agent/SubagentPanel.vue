<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { Bot, CircleStop, Copy, X } from '@lucide/vue'
import type { InstructionEntry } from '@demicodes/protocol'
import MenuDivider from '../ui/MenuDivider.vue'
import MenuItem from '../ui/MenuItem.vue'
import RegionStatus from '../ui/RegionStatus.vue'
import AgentMessageList from './AgentMessageList.vue'
import AgentStatusBar from './AgentStatusBar.vue'
import SessionOverlay from './SessionOverlay.vue'
import SubagentHistoryMenu from './SubagentHistoryMenu.vue'
import TabItem from './TabItem.vue'
import AgentMark from './AgentMark.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { provideLiveCalls } from './live-calls'
import { agentCompaction, subagentPanelTabs, type SubagentRecord } from './subagents'
import { useCallTerminals, type TerminalRecord } from './terminals'
import { latestBlocks, shownWindow, windowEdges, type TranscriptReveal } from './history'
import type { UsageLimits } from './context-usage'
import { loadedInstructions } from './instructions'
import { refusedForSize } from './session-status'
import { copyId } from './copy-id'

const props = withDefaults(
  defineProps<{
    agents: readonly SubagentRecord[]
    /** The conversation's commands: a child's running calls show theirs. */
    terminals?: readonly TerminalRecord[]
    /** A block of a child's transcript to bring into view and mark for a moment, on that child's tab. */
    reveal?: TranscriptReveal | null
    /** The usage limits of the account a provider entry runs on, which a child's context card shows; absent, it shows none. */
    usageLimits?: (providerId: string, account: string | null) => UsageLimits | null
  }>(),
  {
    terminals: () => [],
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
  /** Compact in the shown child's context card: that child's context. */
  compactAgent: [id: string]
  /** The child's context card asks for the usage limits of the account it runs on, `account` of its entry `providerId`, again. */
  refreshLimits: [providerId: string, account: string | null, automatic: boolean]
  /** An instruction in the child's context card opens where it is written. */
  openInstruction: [entry: InstructionEntry]
}>()
/**
 * Whether the panel is open, and the child it shows: none leaves it open,
 * empty, as it opens when no child runs (`subagents.md` § Product rendering).
 */
const open = defineModel<boolean>('open', { required: true })
const activeId = defineModel<string | null>('activeId', { required: true })

const tabs = computed(() => subagentPanelTabs(props.agents, activeId.value))
const active = computed(() => tabs.value.find((agent) => agent.id === activeId.value) ?? null)
const commandOf = useCallTerminals(() => props.terminals)
provideLiveCalls((toolUseId) => active.value ? commandOf(active.value.id, toolUseId) : undefined)
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

// A child picked from Completed replaces the finished tab in place: nothing
// opened or closed, so the strip shows the change at once.
const replacing = ref(false)
function pick(id: string): void {
  replacing.value = true
  activate(id)
  void nextTick(() => {
    replacing.value = false
  })
}

function closePanel(): void {
  open.value = false
}

/** What the shown child's context card holds beside its usage. */
const context = computed(() => {
  const agent = active.value
  if (!agent) {
    return null
  }
  return {
    ...agentCompaction(agent),
    sizeRefused: refusedForSize(latestBlocks(agent.history)),
    instructions: loadedInstructions(latestBlocks(agent.history)),
    limits: props.usageLimits?.(agent.model.providerId, agent.contextUsage?.account ?? null) ?? null,
  }
})

const anyRunning = computed(() => props.agents.some((agent) => agent.phase === 'running'))

function agentOf(id: string): SubagentRecord | undefined {
  return tabs.value.find((agent) => agent.id === id)
}

// The number the `demi agent` commands take, not the record's id.
function copyAgentId(id: string): void {
  const agent = agentOf(id)
  if (agent) {
    void copyId(String(agent.number))
  }
}

// A finished child's tab puts it away; a running one's tab stays until the child is stopped.
function closeTab(id: string): void {
  activeId.value = tabs.value.filter((tab) => tab.id !== id).at(-1)?.id ?? null
}
</script>

<template>
  <SessionOverlay
    :open="open"
    :still-tabs="replacing"
    @close="closePanel"
  >
    <template #tabs="{ openTabMenu }">
      <TabItem
        v-for="agent in tabs"
        :key="agent.id"
        :title="agent.name"
        :is-active="agent.id === active?.id"
        :closable="false"
        @select="activate(agent.id)"
        @contextmenu="openTabMenu($event, agent.id)"
      >
        <template #mark>
          <AgentMark :phase="agent.phase" :size="ICON_PX.markIn28" />
        </template>
      </TabItem>
    </template>
    <template #tabMenu="{ id }">
      <MenuItem
        :icon="CircleStop"
        label="Stop Agent"
        :disabled="agentOf(id)?.phase !== 'running'"
        @select="emit('abortAgent', id)"
      />
      <MenuItem
        :icon="Copy"
        label="Copy Agent ID"
        @select="copyAgentId(id)"
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
        @select="pick"
      />
    </template>
    <div v-if="active" class="flex h-full flex-col">
      <AgentMessageList
        class="min-h-0 flex-1"
        :conversation-id="active.id"
        :node="active.id"
        :cwd="active.cwd"
        :blocks="shown?.blocks ?? []"
        :at-start="shownEdges?.atStart ?? true"
        :at-end="true"
        :pending-calls="active.pendingCalls"
        :waiting-since="active.waitingSince"
        :failures="active.failures"
        :pending-steers="[]"
        :queue="[]"
        :phase="active.phase === 'running' ? active.sessionPhase : 'idle'"
        :bottom-offset="16"
        :persisted-scroll-state="undefined"
        read-only
        :reveal-block-id="reveal?.node === active.id ? reveal.blockId : null"
        @revealed="emit('revealed')"
        @read-before="emit('read', active.id, 'before')"
      />
      <AgentStatusBar
        :run="active"
        :usage="active.contextUsage"
        :compacting="context?.compacting ?? false"
        :unavailable-reason="context?.unavailableReason ?? null"
        :size-refused="context?.sizeRefused ?? false"
        :instructions="context?.instructions ?? []"
        :limits="context?.limits ?? null"
        @compact="emit('compactAgent', active.id)"
        @open-instruction="emit('openInstruction', $event)"
        @refresh-limits="emit('refreshLimits', active.model.providerId, active.contextUsage?.account ?? null, $event)"
      />
    </div>
    <RegionStatus v-else class="h-full" status="note" :icon="Bot" label="No agents running" />
  </SessionOverlay>
</template>

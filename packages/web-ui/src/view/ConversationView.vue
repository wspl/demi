<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import type { ContextUsage } from '@demicodes/protocol'
import { latestWindow, type RequestOutputReading } from '@demicodes/conversation-client'
import { keptComputed } from '../infra/keep-same'
import AgentMessageList from '../agent/AgentMessageList.vue'
import AgentStatusBar from '../agent/AgentStatusBar.vue'
import AgentsChip from '../agent/AgentsChip.vue'
import SessionDock from '../agent/SessionDock.vue'
import SessionSurface from '../agent/SessionSurface.vue'
import SubagentPanel from '../agent/SubagentPanel.vue'
import TerminalChip from '../agent/TerminalChip.vue'
import TerminalPanel from '../agent/TerminalPanel.vue'
import SchedulesChip from '../schedules/SchedulesChip.vue'
import type { ConversationSchedule } from '../schedules/types'
import { latestBlocks, shownWindow, windowEdges } from '../agent/history'
import { rootActivity } from '../agent/agent-activity'
import { refusedForSize } from '../agent/session-status'
import type { AgentRun } from '../agent/subagents'
import { useCallTerminals, useDockTerminals } from '../agent/terminals'
import { useSessionPanels } from '../agent/useSessionPanels'
import { provideLiveCalls } from '../agent/live-calls'
import { agentLinkWindow, findAgent, provideMessageAgents } from '../agent/agent-links'
import { provideScheduleOpener } from '../agent/message-origins'
import { provideBlockReader, type BlockReader } from '../agent/whole-blocks'
import { provideMessageFiles } from '../markdown/message-files'
import type { ChatSessionState } from '../agent/types'
import type { CostShown } from '../agent/context-cost'

/**
 * A conversation's view link page: what the conversation's page shows, live,
 * for someone who only watches. The root agent's transcript, then the dock's
 * chips, Agents, Running, Services and Scheduled, whose windows and menu open
 * as the conversation's page opens them, with no composer under them. Along
 * the bottom edge, the bar the Agents window shows along a child's, which
 * says what the root agent is doing and runs on, and whose context card holds
 * the context and the cost.
 *
 * Nothing on it acts on the conversation: no Stop, End Command, Compact,
 * Fork, edit, or a schedule's switch or Delete. A file or a branch is its
 * text, since the page reaches no Host.
 */
const props = withDefaults(
  defineProps<{
    conversation: ChatSessionState
    /** What the root agent runs on and how far its turn got, which the bar says. */
    run: AgentRun
    /** The root's context estimate; null until its session says. */
    usage: ContextUsage | null
    schedules: readonly ConversationSchedule[]
    /** What the conversation cost, which the root's context card shows. */
    cost: CostShown | null
    /** A child's own cost, which its context card shows. */
    agentCost?: (id: string) => CostShown | null
    /** The output of the request the root agent runs; null while none runs. */
    output: RequestOutputReading | null
    /** The output of the request a child runs. */
    agentOutput?: (id: string) => RequestOutputReading | null
    /** Reads a block whole, for a row the page holds light. */
    readBlock?: BlockReader
    /** Whether the session sends the transcripts of the children the page follows. */
    subagentsStreamed?: boolean
  }>(),
  { subagentsStreamed: false },
)

const emit = defineEmits<{
  /** Read the page of the root's transcript before the window shown. */
  readBefore: []
  /** Read the page after it, for a window that does not reach the end. */
  readAfter: []
  /** Read a child's page before its window shown. */
  readSubagent: [id: string]
  /** Read a child's page after its window shown. */
  readSubagentAfter: [id: string]
  /** The block of the root's transcript, null, or a child's, in view. */
  shown: [node: string | null, blockId: string | null, within: number]
  /** The child the Agents window shows, null once it shows none. */
  showSubagent: [id: string | null]
  /** Retry after the transcript failed to load. */
  retryLoad: []
}>()

const surface = ref<{ dockHeight: number }>()
const list = ref<InstanceType<typeof AgentMessageList>>()
const shown = computed(() => shownWindow(props.conversation.history, props.conversation.shownAt))
const shownEdges = keptComputed(() => windowEdges(props.conversation.history, shown.value))
const shownLatest = computed(() => shown.value === latestWindow(props.conversation.history) || shownEdges.value.atEnd)
const activity = keptComputed(() => rootActivity(props.conversation))

const shownAgents = keptComputed(() => props.conversation.subagents.filter((agent) => agent.onPath))
const terminals = useDockTerminals(() => props.conversation)
const commandOf = useCallTerminals(() => props.conversation.terminals)
provideLiveCalls((toolUseId) => commandOf(undefined, toolUseId))
const { agentsOpen, activeSubagentId, activeTerminalId, toggleAgents, toggleTerminals, close } =
  useSessionPanels(() => shownAgents.value, () => terminals.value, () => props.conversation.id)
watch(() => (agentsOpen.value ? activeSubagentId.value : null), (id) => emit('showSubagent', id), { immediate: true })

// An agent's link opens the Agents window on its tab, as on the
// conversation's page; its menu has no Stop Agent.
provideMessageAgents(() => ({
  find: (number) => findAgent(shownAgents.value, number),
  open: (number) => {
    const next = agentLinkWindow({ open: agentsOpen.value, activeId: activeSubagentId.value }, shownAgents.value, number)
    if (!next) {
      return
    }
    if (next.open) {
      activeSubagentId.value = next.activeId
    } else {
      close()
    }
  },
}))
// The page reaches no Host: a file or a branch is its text, whatever the page
// around it offers.
provideMessageFiles(() => undefined)
provideBlockReader(() => props.readBlock)

const schedulesOpen = ref(false)
provideScheduleOpener((id) => (props.schedules.some((schedule) => schedule.id === id)
  ? () => { schedulesOpen.value = true }
  : undefined))
watch(() => props.schedules.length, (count) => {
  if (count === 0) {
    schedulesOpen.value = false
  }
})

function scrollToBottom(): void {
  if (!shownLatest.value) {
    emit('shown', null, null, 0)
  }
  void nextTick(() => list.value?.scrollToBottom())
}
</script>

<template>
  <div class="flex h-full flex-col bg-surface">
    <SessionSurface ref="surface" class="min-h-0">
      <AgentMessageList
        ref="list"
        class="h-full"
        :conversation-id="conversation.id"
        :cwd="conversation.cwd"
        :elsewhere="false"
        :blocks="shown.blocks"
        :at-start="shownEdges.atStart"
        :at-end="shownEdges.atEnd"
        :latest="shownLatest"
        :summaries="conversation.summaries"
        :activity="activity"
        :failures="conversation.failures"
        :pending-steers="[]"
        :queue="[]"
        :phase="conversation.phase"
        :load="conversation.load"
        :load-error="conversation.lastError"
        :bottom-offset="surface?.dockHeight ?? 0"
        :persisted-scroll-state="undefined"
        read-only
        hide-fork
        @read-before="emit('readBefore')"
        @read-after="emit('readAfter')"
        @retry-load="emit('retryLoad')"
        @shown="(block, within) => emit('shown', null, block, within)"
      />
      <template #dock>
        <SessionDock
          :output="() => output"
          :scope="conversation.id"
          :turn-speed="conversation.turnSpeed"
          :show-scroll-to-bottom="!shownLatest || (!!list && !list.isAtBottom)"
          @scroll-to-bottom="scrollToBottom"
        >
          <template #chips>
            <AgentsChip :agents="shownAgents" :open="agentsOpen" @open="toggleAgents" />
            <TerminalChip :terminals="terminals" kind="command" :active-id="activeTerminalId" @open="toggleTerminals('command')" />
            <TerminalChip :terminals="terminals" kind="service" :active-id="activeTerminalId" @open="toggleTerminals('service')" />
            <SchedulesChip v-model:open="schedulesOpen" :schedules="schedules" read-only />
          </template>
        </SessionDock>
      </template>
      <template #overDock>
        <SubagentPanel
          v-model:open="agentsOpen"
          v-model:active-id="activeSubagentId"
          :conversation="conversation.id"
          :agents="shownAgents"
          :terminals="conversation.terminals"
          :agent-cost="agentCost"
          :output="agentOutput"
          :streamed="subagentsStreamed"
          read-only
          @read="(id) => emit('readSubagent', id)"
          @read-after="(id) => emit('readSubagentAfter', id)"
          @shown="(id, block, within) => emit('shown', id, block, within)"
        />
        <TerminalPanel v-model:active-id="activeTerminalId" :terminals="terminals" read-only />
      </template>
    </SessionSurface>
    <AgentStatusBar
      :run="run"
      :usage="usage"
      :compacting="false"
      :unavailable-reason="null"
      :size-refused="refusedForSize(latestBlocks(conversation.history))"
      :instructions="[]"
      :limits="null"
      :cost="cost"
      session-open
      read-only
    />
  </div>
</template>

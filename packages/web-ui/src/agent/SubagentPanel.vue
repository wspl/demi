<script setup lang="ts">
import { t } from './locales/text'
import { computed, nextTick, reactive, ref, watch } from 'vue'
import { Bot, CircleStop, X } from '@lucide/vue'
import type { InstructionEntry } from '@demicodes/protocol'
import { vContextMenu, type ContextMenuGroup } from '../ui/context-menu'
import RegionStatus from '../ui/RegionStatus.vue'
import AgentMessageList from './AgentMessageList.vue'
import AgentStatusBar from './AgentStatusBar.vue'
import SessionOverlay from './SessionOverlay.vue'
import SubagentHistoryMenu from './SubagentHistoryMenu.vue'
import TabItem from './TabItem.vue'
import AgentMark from './AgentMark.vue'
import { ICON_PX } from '../ui/icon-metrics'
import { provideLiveCalls } from './live-calls'
import { agentCompaction, agentName, subagentPanelTabs, type SubagentRecord } from './subagents'
import { useCallTerminals, type TerminalRecord } from './terminals'
import { latestBlocks, shownWindow, windowEdges, type TranscriptReveal } from './history'
import type { UsageLimits } from './context-usage'
import type { CostState } from './context-cost'
import { loadedInstructions } from './instructions'
import { refusedForSize } from './session-status'
import { agentMenu } from './agent-menu'
import { shownError, type ShownError } from '../infra/errors'

const props = withDefaults(
  defineProps<{
    agents: readonly SubagentRecord[]
    /** The conversation's commands: a child's running calls show theirs. */
    terminals?: readonly TerminalRecord[]
    /**
     * A block of a child's transcript to bring into view and mark for a moment,
     * on that child's tab.
     */
    reveal?: TranscriptReveal | null
    /**
     * The usage limits of the account a provider entry runs on, which a child's
     * context card shows; absent, it shows none.
     */
    usageLimits?: (providerId: string, account: string | null) => UsageLimits | null
    /**
     * The own cost of a child the panel shows, which its context card shows;
     * null while its first read is on its way. Absent, the card shows none.
     */
    agentCost?: (id: string) => CostState | null
    /**
     * Reads the latest page of a child's transcript, for a child the panel
     * shows and holds none of, unless the session sends it (`streamed`). It
     * resolves once the page is in the child's record. The panel shows the
     * child loading meanwhile, and a failed read with Retry. Absent, the panel
     * reads nothing and shows what it is given.
     */
    readLatest?: (id: string) => Promise<void>
    /**
     * Whether the page has, or is opening, a session that sends a live child's
     * transcript once the panel shows it. Without one, the panel reads a live
     * child's latest page as it reads a finished one's.
     */
    streamed?: boolean
  }>(),
  {
    terminals: () => [],
    streamed: false,
  },
)

const emit = defineEmits<{
  /** Stop All: every live child. */
  abort: []
  /** Stop Agent in a live tab's menu: that child and its subtree. */
  abortAgent: [id: string]
  /**
   * Read the page of the child's transcript before the window shown, as the
   * reader nears its start.
   */
  read: [id: string]
  /** The block `reveal` names is in view. */
  revealed: []
  /** Compact in the shown child's context card: that child's context. */
  compactAgent: [id: string]
  /**
   * The child's context card asks for the usage limits of the account it runs
   * on, `account` of its entry `providerId`, again.
   */
  refreshLimits: [providerId: string, account: string | null, automatic: boolean]
  /** An instruction in the child's context card opens where it is written. */
  openInstruction: [entry: InstructionEntry]
  /** Retry in the shown child's context card, after its cost's read failed. */
  retryAgentCost: [id: string]
}>()
/**
 * Whether the panel is open, and the child it shows. None leaves it open,
 * empty, as it opens when no child runs.
 */
const open = defineModel<boolean>('open', { required: true })
const activeId = defineModel<string | null>('activeId', { required: true })

const tabs = computed(() => subagentPanelTabs(props.agents, activeId.value))
const active = computed(() => tabs.value.find((agent) => agent.id === activeId.value) ?? null)
const commandOf = useCallTerminals(() => props.terminals)
provideLiveCalls((toolUseId) => active.value ? commandOf(active.value.id, toolUseId) : undefined)
// A child's transcript shows its latest window, and earlier pages as the reader
// scrolls up.
const shown = computed(() => active.value ? shownWindow(active.value.history, null) : null)
const shownEdges = computed(() => active.value && shown.value ? windowEdges(active.value.history, shown.value) : null)
/**
 * Each child's read of its latest page while it runs, or once it failed, by the
 * child's id. A child the map does not name was read, or never needed to be.
 */
const reads = reactive(new Map<string, { phase: 'loading' } | { phase: 'failed'; error: ShownError }>())
function readLatest(id: string): Promise<void> | undefined {
  const read = props.readLatest
  if (!read) {
    return undefined
  }
  reads.set(id, { phase: 'loading' })
  return read(id).then(
    () => {
      reads.delete(id)
    },
    (error: unknown) => {
      reads.set(id, { phase: 'failed', error: shownError(error) })
    },
  )
}
/**
 * Whether the session sends the child `agent`'s transcript once the panel shows
 * it: it is live, and the page has a session.
 */
function sent(agent: SubagentRecord): boolean {
  return agent.phase === 'running' && props.streamed
}
// A child the panel holds none of reads its latest page when it shows, unless
// the session sends it. A live child's transcript comes with the socket's reset
// once the page shows it (`crates/agent-server/src/server/tree.rs`).
//
// - A child whose read failed waits for Retry.
// - A child that ends, or whose page loses its session, before its reset came
//   reads then.
watch(
  () => {
    const agent = active.value
    return agent && agent.history.windows.length === 0 && !sent(agent) ? agent.id : null
  },
  (id) => {
    if (id !== null && !reads.has(id)) {
      void readLatest(id)
    }
  },
  { immediate: true },
)
/**
 * The shown child's load while the panel holds none of its transcript, so the
 * panel never shows an empty transcript in its place. A child the session sends
 * loads until its reset arrives, whatever the socket's own state. A page that
 * arrives shows at once.
 */
const load = computed(() => {
  const agent = active.value
  if (!agent || agent.history.windows.length > 0) {
    return null
  }
  return reads.get(agent.id) ?? (sent(agent) ? { phase: 'loading' as const } : null)
})
function retryRead(): Promise<void> | undefined {
  return active.value ? readLatest(active.value.id) : undefined
}

function activate(id: string): void {
  activeId.value = id
}

// A child picked from Completed replaces the finished tab in place. Nothing
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

/**
 * A tab's menu: its agent's, a link's less Open, then Close Tab once the agent
 * is done.
 */
function tabMenu(id: string): () => ContextMenuGroup[] {
  return () => {
    const agent = agentOf(id)
    if (!agent) {
      return []
    }
    return [
      ...agentMenu({ number: agent.number, running: agent.phase === 'running' }, { stop: () => emit('abortAgent', id) }),
      [{ icon: X, label: t('subagents.closeTab'), disabled: agent.phase === 'running', run: () => closeTab(id) }],
    ]
  }
}

/** What acts on every agent. */
function stripMenu(): ContextMenuGroup[] {
  return [[{ icon: CircleStop, label: t('subagents.stopAll'), disabled: !anyRunning.value, run: () => emit('abort') }]]
}

// A finished child's tab puts it away; a running one's tab stays until the
// child is stopped.
function closeTab(id: string): void {
  activeId.value = tabs.value.filter((tab) => tab.id !== id).at(-1)?.id ?? null
}
</script>

<template>
  <SessionOverlay
    :open="open"
    :still-tabs="replacing"
    :strip-menu="stripMenu"
    @close="closePanel"
  >
    <template #tabs>
      <TabItem
        v-for="agent in tabs"
        :key="agent.id"
        :title="agentName(agent.name, agent.number)"
        :is-active="agent.id === active?.id"
        :closable="false"
        @select="activate(agent.id)"
        v-context-menu="tabMenu(agent.id)"
      >
        <template #mark>
          <AgentMark :phase="agent.phase" :size="ICON_PX.markIn28" />
        </template>
      </TabItem>
    </template>
    <template #trailing>
      <SubagentHistoryMenu
        :agents="agents"
        :active-id="activeId"
        @select="pick"
      />
    </template>
    <div v-if="active" class="flex h-full flex-col">
      <RegionStatus
        v-if="load"
        class="min-h-0 flex-1"
        :status="load.phase"
        :label="load.phase === 'failed' ? t('subagents.loadFailed') : t('subagents.loading')"
        :loading-label="t('subagents.loading')"
        :detail="load.phase === 'failed' ? load.error.message : null"
        :request-id="load.phase === 'failed' ? load.error.requestId : null"
        :on-retry="retryRead"
      />
      <AgentMessageList
        v-else
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
        @read-before="emit('read', active.id)"
      />
      <AgentStatusBar
        :run="active"
        :usage="active.contextUsage"
        :compacting="context?.compacting ?? false"
        :unavailable-reason="context?.unavailableReason ?? null"
        :size-refused="context?.sizeRefused ?? false"
        :instructions="context?.instructions ?? []"
        :limits="context?.limits ?? null"
        :cost="agentCost?.(active.id)"
        @compact="emit('compactAgent', active.id)"
        @retry-cost="emit('retryAgentCost', active.id)"
        @open-instruction="emit('openInstruction', $event)"
        @refresh-limits="emit('refreshLimits', active.model.providerId, active.contextUsage?.account ?? null, $event)"
      />
    </div>
    <RegionStatus v-else class="h-full" status="note" :icon="Bot" :label="t('subagents.none')" />
  </SessionOverlay>
</template>

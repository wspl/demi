<script setup lang="ts">
import { t } from './locales/text'
import { computed, reactive, watch } from 'vue'
import { Bot } from '@lucide/vue'
import type { InstructionEntry } from '@demicodes/protocol'
import type { RequestOutputReading } from '@demicodes/conversation-client'
import { isAbortError } from '@demicodes/utils'
import RegionStatus from '../ui/RegionStatus.vue'
import AgentMessageList from './AgentMessageList.vue'
import AgentStatusBar from './AgentStatusBar.vue'
import RequestOutputMeter from './RequestOutputMeter.vue'
import { provideLiveCalls } from './live-calls'
import { agentCompaction, subagentActivity, subagentPanelTabs, type SubagentRecord } from './subagents'
import { useCallTerminals, type TerminalRecord } from './terminals'
import { latestBlocks, shownWindow, windowEdges, type TranscriptReveal } from './history'
import type { UsageLimits } from './context-usage'
import type { CostShown } from './context-cost'
import { loadedInstructions } from './instructions'
import { refusedForSize } from './session-status'
import { shownError, type ShownError } from '../infra/errors'

/**
 * What the Agents window shows: the child `activeId` names, its transcript
 * and its status bar, or that none runs. The window's tabs are the running
 * children and the one shown (`subagentPanelTabs`).
 */

const props = withDefaults(
  defineProps<{
    /** The conversation the agents belong to, which their lists' lines in the log are written under. */
    conversation?: string
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
     * The own cost of a child the panel shows, which its context card shows.
     * Absent, the card shows none.
     */
    agentCost?: (id: string) => CostShown | null
    /**
     * The output of the request a child's agent runs, as the server tells it;
     * a getter, as the dock's. Absent, the panel shows none.
     */
    output?: (id: string) => RequestOutputReading | null
    /**
     * Reads the latest page of a child's transcript, for a child the panel
     * shows and holds none of, unless the session sends it (`streamed`). It
     * resolves once the page is in the child's record. The panel shows the
     * child loading meanwhile, and a failed read with Retry. Absent, the panel
     * reads nothing and shows what it is given.
     */
    readLatest?: (id: string) => Promise<void>
    /**
     * Whether the page has, or is opening, a session whose lane follows the
     * child the panel shows and those read ahead, live or finished. Without
     * one, the panel reads the child's latest page when it shows it.
     */
    streamed?: boolean
    /** Whether the panel shows the child's output (Settings › Agent › Output speed); off, it keeps its room. */
    outputSpeed?: boolean
  }>(),
  {
    terminals: () => [],
    streamed: false,
    outputSpeed: true,
  },
)

const emit = defineEmits<{
  /**
   * Read the page of the child's transcript before the window shown, as the
   * reader nears its start.
   */
  read: [id: string]
  /**
   * Read the page of the child's transcript after the window shown, as the
   * reader nears its end, for a window that does not reach the transcript's.
   */
  readAfter: [id: string]
  /** The block of the child's transcript in view, or null for its latest. */
  shown: [id: string, blockId: string | null, within: number]
  /** The block `reveal` names is in view. */
  revealed: []
  /** Compact in the shown child's context card: that child's context. */
  compactAgent: [id: string]
  /**
   * The child's context card asks for the usage limits of the account it runs
   * on, `account` of its entry `providerId`, again.
   */
  refreshLimits: [providerId: string, account: string | null]
  /** An instruction in the child's context card opens where it is written. */
  openInstruction: [entry: InstructionEntry]
}>()
/** The child it shows; none shows it empty, as it opens when no child runs. */
const activeId = defineModel<string | null>('activeId', { required: true })

const tabs = computed(() => subagentPanelTabs(props.agents, activeId.value))
const active = computed(() => tabs.value.find((agent) => agent.id === activeId.value) ?? null)
const commandOf = useCallTerminals(() => props.terminals)
provideLiveCalls((toolUseId) => active.value ? commandOf(active.value.id, toolUseId) : undefined)
// A child's transcript shows the window around the block in view, its latest
// at first, and the pages beside it as the reader scrolls.
const shown = computed(() => active.value ? shownWindow(active.value.history, active.value.shownAt) : null)
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
      // A read stopped as the panel no longer showed the child is read
      // again when the child shows.
      if (isAbortError(error)) {
        reads.delete(id)
      } else {
        reads.set(id, { phase: 'failed', error: shownError(error) })
      }
    },
  )
}
// A child the panel holds none of reads its latest page when it shows, unless
// the session follows it. The page reads the running and recently ended
// children ahead and follows them, so one shows at once; a child not read yet,
// as one that ended in an earlier turn, shows loading until its read comes in.
//
// - A child whose read failed waits for Retry.
// - A child whose page loses its session before its reset came reads then.
watch(
  () => {
    const agent = active.value
    return agent && agent.history.windows.length === 0 && !props.streamed ? agent.id : null
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
 * loads until its reset arrives, whatever the lane's own state. A page that
 * arrives shows at once.
 */
const load = computed(() => {
  const agent = active.value
  if (!agent || agent.history.windows.length > 0) {
    return null
  }
  return reads.get(agent.id) ?? (props.streamed ? { phase: 'loading' as const } : null)
})
function retryRead(): Promise<void> | undefined {
  return active.value ? readLatest(active.value.id) : undefined
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

</script>

<template>
  <div class="h-full [--session-gutter:0px]">
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
        :conversation="props.conversation"
        :node="active.id"
        :cwd="active.cwd"
        :elsewhere="active.device !== null"
        :blocks="shown?.blocks ?? []"
        :at-start="shownEdges?.atStart ?? true"
        :at-end="shownEdges?.atEnd ?? true"
        :activity="subagentActivity(active)"
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
        @read-after="emit('readAfter', active.id)"
        @shown="(block, within) => emit('shown', active!.id, block, within)"
      />
      <!--
        The reading of the child's request, over its status bar as the dock
        shows the root's over the composer; the row takes no room while there
        is none.
      -->
      <div class="hidden shrink-0 justify-end px-4 pb-1 has-[>*]:flex">
        <RequestOutputMeter :output="() => (active ? output?.(active.id) ?? null : null)" :scope="active.id" :turn-speed="null" :visible="outputSpeed" />
      </div>
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
        @open-instruction="emit('openInstruction', $event)"
        @refresh-limits="emit('refreshLimits', active.model.providerId, active.contextUsage?.account ?? null)"
      />
    </div>
    <RegionStatus v-else class="h-full" status="note" :icon="Bot" :label="t('subagents.none')" />
  </div>
</template>

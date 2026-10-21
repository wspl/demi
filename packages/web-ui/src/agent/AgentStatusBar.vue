<script setup lang="ts">
import { around, t } from './locales/text'
import { computed } from 'vue'
import { Folder } from '@lucide/vue'
import { baseName } from '@demicodes/utils'
import { formatSpan } from '../ui/dates'
import type { ContextUsage, InstructionEntry } from '@demicodes/protocol'
import StatusBarSegment from '../ui/StatusBarSegment.vue'
import StatusDot from '../ui/StatusDot.vue'
import Tag from '../ui/Tag.vue'
import Tooltip from '../ui/Tooltip.vue'
import { useIconPx } from '../ui/icon-metrics'
import AgentMark from './AgentMark.vue'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import { agentPhaseWords, useMessageAgents } from './agent-links'
import type { UsageLimits } from './context-usage'
import type { CostShown } from './context-cost'
import { useElapsedTime } from '../composables/useElapsedTime'
import { useMessageFiles } from '../markdown/message-files'
import { statusTone } from './conversation-status'
import { isSubagentRunning, runEffortWords, subagentStatus, type AgentRun } from './subagents'

/**
 * The thin bar along the Agents window's bottom edge that says what the shown
 * child is doing and runs on.
 *
 * The bar is drawn in the secondary text color under its own top divider. Like
 * an editor's status bar, it is made of segments:
 *
 * - the state and time
 * - the folder the child works in
 * - *Forked from agent N*
 * - the profile
 * - the model with its effort
 * - the context indicator
 *
 * A segment that acts when pressed shows a hover background over its whole
 * extent. The folder, the forked-from link and the context indicator act when
 * pressed. The other segments only explain.
 *
 * The segments run close to the window's ends (`.status-bar`):
 *
 * - The bar's inset is the same on every side.
 * - The first and last text stand as far from the window's side as the window's
 *   corner is round, never at the transcript's wide indent.
 * - On a phone, the bar is a row's height, 44px, and its text 15px, as a tab's.
 *
 * Where the room runs out, segments give way by the bar's own width, in this
 * order:
 *
 * 1. *Forked from*
 * 2. the model's name
 * 3. the profile's tag
 * 4. the folder's name, which leaves its icon and its tip
 *
 * A phone's bar thus keeps the effort whole beside the context indicator. The
 * state and the time always stay.
 *
 * An ended child's session is closed, so its bar has no context indicator.
 */
const props = defineProps<{
  run: AgentRun
  /** The child's context estimate; null until its session says. */
  usage: ContextUsage | null
  compacting: boolean
  /**
   * Why Compact cannot run now apart from the usage, such as the child's
   * running turn.
   */
  unavailableReason: string | null
  /**
   * The child's unfinished turn failed for its size, so Compact runs at any
   * share.
   */
  sizeRefused?: boolean
  /** What the child's newest instructions block holds. */
  instructions: readonly InstructionEntry[]
  /**
   * The usage limits of the account the child runs on; none for a model on an
   * API key.
   */
  limits: UsageLimits | null
  /**
   * The child's own cost, from its metadata, which its context card shows
   * without the Profiles part, with its cache chart. Absent, the card shows
   * none.
   */
  cost?: CostShown | null
}>()

const emit = defineEmits<{
  compact: []
  openInstruction: [entry: InstructionEntry]
  refreshLimits: [automatic: boolean]
}>()

const icons = useIconPx()

// The time since the round began, following the page's clock while it runs.
const elapsedMs = useElapsedTime(
  () => Date.parse(props.run.startedAt),
  () => props.run.phase === 'running',
  () => (props.run.endedAt ? Date.parse(props.run.endedAt) : null),
)
const time = computed(() => formatSpan(elapsedMs.value ?? 0))
// The dot of the agent's tab, drawn in the flow; a Stop, like a child that ran
// nothing, has none.
const tone = computed(() => statusTone(subagentStatus(props.run.phase)))
const effort = computed(() => runEffortWords(props.run.model))
const agents = useMessageAgents()
function openForked(): void {
  if (props.run.forkedFrom !== null)
    agents()?.open(props.run.forkedFrom)
}
// The folder opens the File view on the child's directory where a plugin opens
// files.
const files = useMessageFiles()
const openFolder = computed(() => {
  const open = files()?.open
  return open ? () => open({ path: '', line: null, root: props.run.cwd }) : null
})
const forked = computed(() => (props.run.forkedFrom === null ? null : agents()?.find(props.run.forkedFrom) ?? null))
/**
 * What *Forked from agent N* says around the agent's name, as the language
 * words it.
 */
const forkedSentence = computed(() => around('subagents.forkedFrom', 'agent'))
</script>

<template>
  <div data-window-bar class="status-bar @container flex h-8 phone:h-row shrink-0 items-stretch border-t border-line text-chrome text-fg-muted">
    <div class="flex shrink-0 items-stretch">
      <StatusBarSegment>
        <StatusDot v-if="tone" :tone="tone" :breathing="run.phase === 'running'" />
        <span>{{ agentPhaseWords(run.phase) }}</span>
        <span class="text-fg-faint" aria-hidden="true">·</span>
        <span class="tabular-nums">{{ time }}</span>
      </StatusBarSegment>
      <!--
        Where it works: the directory's own name, its whole path in the tip.
      -->
      <Tooltip tag="span" class="flex min-w-0" wide>
        <StatusBarSegment :acting="openFolder !== null" class="min-w-0" @click="openFolder?.()">
          <Folder :size="icons.chip" class="shrink-0" aria-hidden="true" />
          <span class="hidden max-w-40 truncate @min-[28rem]:inline">{{ baseName(run.cwd) }}</span>
        </StatusBarSegment>
        <template #overlay>
          <div>{{ run.cwd }}</div>
        </template>
      </Tooltip>
      <!--
        A fork of an agent the conversation does not have names it and acts on
        nothing.
      -->
      <template v-if="run.forkedFrom !== null">
        <StatusBarSegment v-if="forked" acting class="hidden @min-[40rem]:flex" @click="openForked">
          <span v-if="forkedSentence[0].trim()">{{ forkedSentence[0].trim() }}</span>
          <span class="inline-flex shrink-0"><AgentMark :phase="forked.run?.phase ?? null" :size="icons.chip" /></span>
          {{ t('subagents.agentNumber', { number: run.forkedFrom }) }}
          <span v-if="forkedSentence[1].trim()">{{ forkedSentence[1].trim() }}</span>
        </StatusBarSegment>
        <StatusBarSegment v-else class="hidden @min-[40rem]:flex">{{ t('subagents.forkedFrom', { agent: t('subagents.agentNumber', { number: run.forkedFrom }) }) }}</StatusBarSegment>
      </template>
    </div>
    <div class="ml-auto flex min-w-0 items-stretch">
      <StatusBarSegment v-if="run.profile" class="hidden @min-[28rem]:flex">
        <Tag>{{ run.profile }}</Tag>
      </StatusBarSegment>
      <Tooltip tag="span" class="flex min-w-0" wide>
        <StatusBarSegment class="min-w-0">
          <!--
            One run of text, so that where even the effort has no room it ends
            in an ellipsis rather than over the indicator.
          -->
          <span class="truncate">
            <span class="hidden @min-[28rem]:inline">{{ run.model.model }}</span>
            <template v-if="effort">
              <span class="hidden px-1.5 text-fg-faint @min-[28rem]:inline" aria-hidden="true">·</span>
              <span>{{ effort }}</span>
            </template>
            <template v-if="run.model.fast">
              <span class="px-1.5 text-fg-faint" :class="effort ? '' : 'hidden @min-[28rem]:inline'" aria-hidden="true">·</span>
              <span>{{ t('subagents.fast') }}</span>
            </template>
          </span>
        </StatusBarSegment>
        <template #overlay>
          <div>{{ run.model.provider }} · {{ run.model.model }}</div>
          <div v-if="!run.profile" class="text-fg-muted">{{ t('subagents.inherited') }}</div>
        </template>
      </Tooltip>
      <ContextUsageIndicator
        v-if="isSubagentRunning(run.phase)"
        segment
        class="shrink-0"
        :usage="usage"
        :size-refused="sizeRefused"
        :is-compacting="compacting"
        :unavailable-reason="unavailableReason"
        :instructions="instructions"
        :limits="limits"
        :cost="cost"
        @compact="emit('compact')"
        @open-instruction="emit('openInstruction', $event)"
        @refresh-limits="emit('refreshLimits', $event)"
      />
    </div>
  </div>
</template>

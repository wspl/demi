<script setup lang="ts">
import type { ContextUsage, InstructionEntry } from '@demicodes/protocol'
import AgentRunLine from './AgentRunLine.vue'
import ContextUsageIndicator from './ContextUsageIndicator.vue'
import type { UsageLimits } from './context-usage'
import type { AgentRun } from './subagents'

/**
 * The thin bar along the Agents window's bottom edge that says what the
 * shown child is doing and runs on (`subagents.md` § What an agent runs on),
 * in the secondary text color under its own top divider, with the
 * composer's context indicator after the model and effort: the child's own
 * context, whose card compacts the child. Its text keeps the transcript's
 * distance from the window's left and right edges, `--agent-pad-x`; on a
 * phone the bar is 36px tall and its text 15px, as a tab's.
 */
defineProps<{
  run: AgentRun
  /** The child's context estimate; null until its session says. */
  usage: ContextUsage | null
  compacting: boolean
  /** Why Compact cannot run now apart from the usage, such as the child's running turn. */
  unavailableReason: string | null
  /** What the child's newest instructions block holds. */
  instructions: readonly InstructionEntry[]
  /** The usage limits of the account the child runs on; none for a model on an API key. */
  limits: UsageLimits | null
}>()

const emit = defineEmits<{
  compact: []
  openInstruction: [entry: InstructionEntry]
  refreshLimits: [automatic: boolean]
}>()
</script>

<template>
  <div class="flex h-8 phone:h-9 shrink-0 items-center gap-1.5 border-t border-line px-[var(--agent-pad-x,2rem)] text-chrome phone:text-[15px] text-fg-muted">
    <AgentRunLine class="min-w-0 flex-1" :run="run" />
    <ContextUsageIndicator
      class="shrink-0"
      :usage="usage"
      :is-compacting="compacting"
      :unavailable-reason="unavailableReason"
      :instructions="instructions"
      :limits="limits"
      @compact="emit('compact')"
      @open-instruction="emit('openInstruction', $event)"
      @refresh-limits="emit('refreshLimits', $event)"
    />
  </div>
</template>

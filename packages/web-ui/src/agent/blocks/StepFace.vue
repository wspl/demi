<script setup lang="ts">
import { computed } from 'vue'
import ThinkingFace from './ThinkingFace.vue'
import ToolCallFace from './ToolCallFace.vue'
import CommandReportFace from './CommandReportFace.vue'
import AgentReceiptFace from './AgentReceiptFace.vue'
import { reportShown } from '../command-reports'
import type { WorkStep } from '../work-groups'

/**
 * What a step's row says on its line, one component per kind
 * (`runtime.md` § Work groups): the run's folded face shows its newest step
 * by the same face as the step's own row, its tags and a notice's linked
 * title included; a `wakeup` that reports several ends shows the newest.
 */
const props = defineProps<{
  step: WorkStep
  /** A thinking step still streams. */
  streaming?: boolean
  /** When a thinking step ended: the start of the block after it. */
  endedAt?: string | null
}>()

const report = computed(() => (props.step.type === 'wakeup' ? props.step.reports.findLast(reportShown) : undefined))
</script>

<template>
  <ThinkingFace v-if="step.type === 'thinking'" :streaming="streaming ?? false" :created-at="step.createdAt" :ended-at="endedAt" />
  <ToolCallFace v-else-if="step.type === 'tool_call'" :block="step" />
  <span v-else-if="step.type === 'pending_call'" class="min-w-0 truncate">{{ step.call.description }}</span>
  <CommandReportFace v-else-if="step.type === 'wakeup' && report" :report="report" />
  <AgentReceiptFace v-else-if="step.type === 'agent_message'" :message="step.message" />
</template>

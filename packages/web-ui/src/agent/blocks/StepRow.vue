<script setup lang="ts">
import ThinkingBlock from './ThinkingBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import PendingCallBlock from './PendingCallBlock.vue'
import CommandReportBlock from './CommandReportBlock.vue'
import AgentReceiptBlock from './AgentReceiptBlock.vue'
import BlockScope from './BlockScope.vue'
import type { WorkStep } from '../work-groups'

/**
 * A step of a run as its own row (`runtime.md` § Work groups), one
 * component per kind, wherever it stands: as a run of that step alone, and
 * among the steps an opened run shows. It opens to the step alone.
 */
defineProps<{
  step: WorkStep
  /** When a thinking step ended: the start of the block after it. */
  endedAt?: string | null
}>()
const isOpen = defineModel<boolean>('open', { default: false })
</script>

<template>
  <BlockScope v-if="step.type === 'thinking'" :block="step">
    <ThinkingBlock
      v-model:open="isOpen"
      :thinking="step.text"
      :is-streaming="false"
      :created-at="step.createdAt"
      :ended-at="endedAt"
    />
  </BlockScope>
  <BlockScope v-else-if="step.type === 'tool_call'" :block="step">
    <ToolCallBlock v-model:open="isOpen" :block="step" />
  </BlockScope>
  <PendingCallBlock v-else-if="step.type === 'pending_call'" :step="step" />
  <CommandReportBlock v-else-if="step.type === 'wakeup'" :reports="step.reports" />
  <AgentReceiptBlock v-else :message="step.message" />
</template>

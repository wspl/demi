<script setup lang="ts">
import { Bot } from '@lucide/vue'
import ConversationStatusDot from './ConversationStatusDot.vue'
import { subagentStatus, type SubagentPhase } from './subagents'

/**
 * An agent's mark: the robot, with the dot of its state on its corner.
 *
 * The dot changes as the agent runs and ends. The agent's tab in the Agents
 * window wears the mark, and so does a message's link to the agent. The
 * conversation's own agent, without a phase, wears the robot alone.
 */
defineProps<{
  phase: SubagentPhase | null
  size: number
}>()
</script>

<template>
  <span class="relative inline-flex shrink-0 items-center justify-center">
    <Bot :size="size" class="text-fg-subtle" />
    <ConversationStatusDot v-if="phase" :status="subagentStatus(phase)" />
  </span>
</template>

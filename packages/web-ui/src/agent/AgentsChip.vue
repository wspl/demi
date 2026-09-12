<script setup lang="ts">
import { computed } from 'vue'
import { Bot } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import SessionDockChip from './SessionDockChip.vue'
import { agentsChipLabel, runningSubagents, type SubagentRecord } from './subagents'

const props = defineProps<{
  agents: readonly SubagentRecord[]
  open?: boolean
}>()

const emit = defineEmits<{
  open: []
}>()

const running = computed(() => runningSubagents(props.agents))
</script>

<template>
  <SessionDockChip
    v-if="running.length"
    data-session-overlay-toggle
    dot="accent"
    breathing
    :aria-expanded="open === true"
    aria-haspopup="dialog"
    @click="emit('open')"
  >
    <Bot :size="ICON_PX.in28" />
    {{ agentsChipLabel(running.length) }}
  </SessionDockChip>
</template>

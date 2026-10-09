<script setup lang="ts">
import { computed } from 'vue'
import { Bot } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import SessionDockChip from './SessionDockChip.vue'
import { agentsChip, type SubagentRecord } from './subagents'

const props = defineProps<{
  agents: readonly SubagentRecord[]
  open?: boolean
}>()

const emit = defineEmits<{
  open: []
}>()

const chip = computed(() => agentsChip(props.agents))
</script>

<template>
  <SessionDockChip
    v-if="chip"
    data-session-overlay-toggle
    :dot="chip.running ? 'accent' : undefined"
    :breathing="chip.running"
    :aria-expanded="open === true"
    aria-haspopup="dialog"
    @click="emit('open')"
  >
    <Bot :size="ICON_PX.in28" />
    {{ chip.label }}
  </SessionDockChip>
</template>

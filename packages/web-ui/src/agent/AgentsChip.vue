<script setup lang="ts">
import { computed } from 'vue'
import { Bot } from '@lucide/vue'
import { ICON_PX } from '../ui/icon-metrics'
import SessionDockChip from './SessionDockChip.vue'
import { agentsChipLabel, type SubagentRecord } from './subagents'

const props = defineProps<{
  agents: readonly SubagentRecord[]
  open?: boolean
}>()

const emit = defineEmits<{
  open: []
}>()

const label = computed(() => agentsChipLabel(props.agents))
</script>

<template>
  <SessionDockChip
    v-if="label"
    dot="accent"
    breathing
    :aria-expanded="open === true"
    aria-haspopup="dialog"
    @click="emit('open')"
  >
    <Bot :size="ICON_PX.in28" />
    {{ label }}
  </SessionDockChip>
</template>

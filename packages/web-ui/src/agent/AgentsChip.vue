<script setup lang="ts">
import { computed } from 'vue'
import { Bot } from '@lucide/vue'
import { useIconPx } from '../ui/icon-metrics'
import SessionDockChip from './SessionDockChip.vue'
import { KEEPS_WINDOWS } from '../ui/window-stack'
import { agentsChipLabel, type SubagentRecord } from './subagents'

const props = defineProps<{
  agents: readonly SubagentRecord[]
  open?: boolean
}>()

const emit = defineEmits<{
  open: []
}>()

const icons = useIconPx()
const label = computed(() => agentsChipLabel(props.agents))
</script>

<template>
  <!--
    The chip is its window's own control: a press on it is not one outside the
    session's windows (`window-stack.ts`).
  -->
  <SessionDockChip
    v-bind="{ [KEEPS_WINDOWS]: '' }"
    v-if="label"
    dot="accent"
    breathing
    :aria-expanded="open === true"
    aria-haspopup="dialog"
    @click="emit('open')"
  >
    <Bot :size="icons.chip" />
    {{ label }}
  </SessionDockChip>
</template>

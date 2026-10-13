<script setup lang="ts">
import { computed } from 'vue'
import SessionDockChip from './SessionDockChip.vue'
import { KEEPS_WINDOWS } from '../ui/window-stack'
import { runningChipLabel, runningTerminals, type TerminalRecord } from './terminals'

const props = defineProps<{
  terminals: readonly TerminalRecord[]
  open?: boolean
}>()

const emit = defineEmits<{
  open: []
}>()

const running = computed(() => runningTerminals(props.terminals))
</script>

<template>
  <!-- The chip is its window's own control: a press on it is not one outside the session's windows
       (`window-stack.ts`). -->
  <SessionDockChip
    v-bind="{ [KEEPS_WINDOWS]: '' }"
    v-if="running.length"
    dot="accent" breathing
    :aria-expanded="open === true"
    aria-haspopup="dialog"
    @click="emit('open')"
  >
    {{ runningChipLabel(running.length) }}
  </SessionDockChip>
</template>

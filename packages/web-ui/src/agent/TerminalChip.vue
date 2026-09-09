<script setup lang="ts">
import { computed } from 'vue'
import SessionDockChip from './SessionDockChip.vue'
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
  <SessionDockChip
    v-if="running.length"
    data-session-overlay-toggle
    dot="accent"
    :aria-expanded="open === true"
    aria-haspopup="dialog"
    @click="emit('open')"
  >
    {{ runningChipLabel(running.length) }}
  </SessionDockChip>
</template>

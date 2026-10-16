<script setup lang="ts">
import RegionStatus from '../ui/RegionStatus.vue'
import { t } from './locales/text'
import type { TerminalRecord } from './terminals'
import { useXterm } from './xterm'

defineProps<{
  terminal: TerminalRecord
}>()

// A terminal shows its loading state until xterm is there, whose load the
// first terminal that opens starts.
const { load, retry } = useXterm()
</script>

<template>
  <component
    :is="load.chunk.default"
    v-if="load.phase === 'ready'"
    :output="terminal.output"
    :chars="terminal.chars"
    :running="terminal.phase === 'running'"
    :script="terminal.script"
  />
  <RegionStatus
    v-else
    class="h-full"
    :status="load.phase"
    :label="load.phase === 'failed' ? t('terminals.unavailable') : t('terminals.loading')"
    :loading-label="t('terminals.loading')"
    :detail="load.phase === 'failed' ? load.message : null"
    :on-retry="retry"
  />
</template>

<script setup lang="ts">
import { t } from './locales/text'
import { computed } from 'vue'
import { SquareTerminal } from '@lucide/vue'
import RegionStatus from '../ui/RegionStatus.vue'
import TerminalView from './TerminalView.vue'
import type { TerminalRecord } from './terminals'

/**
 * What the Running or the Services window shows: the command its tab names,
 * or that none runs.
 */
const props = defineProps<{
  /** The window's commands, its tabs. */
  terminals: readonly TerminalRecord[]
  activeId: string | null
}>()

const active = computed(() => props.terminals.find((terminal) => terminal.id === props.activeId) ?? props.terminals[0] ?? null)
</script>

<template>
  <TerminalView v-if="active" :key="active.id" :terminal="active" />
  <RegionStatus v-else class="h-full" status="note" :icon="SquareTerminal" :label="t('terminals.none')" />
</template>

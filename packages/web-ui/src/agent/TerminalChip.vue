<script setup lang="ts">
import { computed } from 'vue'
import { Activity, SquareTerminal } from '@lucide/vue'
import { useIconPx } from '../ui/icon-metrics'
import SessionDockChip from './SessionDockChip.vue'
import { KEEPS_WINDOWS } from '../ui/window-stack'
import { chipLabel, runningOfKind, shownKind, type ChipKind, type TerminalRecord } from './terminals'

/**
 * The Running chip, or the Service chip: one chip whose `kind` says which
 * commands it counts, its icon and its dot. A command that went to the
 * background and is not a service is work in progress, so Running has the
 * breathing dot; a service that runs is in its settled state, so Service has
 * the pulse line and no dot.
 */
const props = defineProps<{
  terminals: readonly TerminalRecord[]
  kind: ChipKind
  /** The command the window shows, none while it is closed. */
  activeId?: string | null
}>()

const emit = defineEmits<{
  open: []
}>()

const icons = useIconPx()
const running = computed(() => runningOfKind(props.terminals, props.kind))
// The window is this chip's own while it shows a command of its kind.
const expanded = computed(() => shownKind(props.terminals, props.activeId ?? null) === props.kind)
</script>

<template>
  <!--
    The chip is its window's own control: a press on it is not one outside the
    session's windows (`window-stack.ts`).
  -->
  <SessionDockChip
    v-bind="{ [KEEPS_WINDOWS]: '' }"
    v-if="running.length"
    :dot="kind === 'command' ? 'accent' : undefined"
    :breathing="kind === 'command'"
    :aria-expanded="expanded"
    aria-haspopup="dialog"
    @click="emit('open')"
  >
    <SquareTerminal v-if="kind === 'command'" :size="icons.chip" />
    <Activity v-else :size="icons.chip" />
    {{ chipLabel(kind, running.length) }}
  </SessionDockChip>
</template>

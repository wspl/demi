<script setup lang="ts">
import { computed } from 'vue'
import { FileText, UserRound } from '@lucide/vue'
import type { ContextUsage, InstructionEntry } from '@demicodes/protocol'
import Button from '../ui/Button.vue'
import HoverCard from '../ui/HoverCard.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import { formatTokens } from '../ui/token-count'
import { compactionRefusal, contextPercent, contextRatio } from './context-usage'
import { instructionRows } from './instructions'

/**
 * The composer's context meter: a ring of how full the next request is, as
 * the backend estimates it. Pointing at it or focusing it opens a card with
 * the numbers and Compact; a click on the ring itself does nothing. Below
 * them it lists the instructions the model holds (`instructions.md` § What
 * the card lists), each of which opens where it is written.
 */
const props = defineProps<{
  usage?: ContextUsage | null
  isCompacting?: boolean
  /** Why Compact cannot run now apart from the usage, such as a running turn. */
  unavailableReason?: string | null
  /** The card shows without the pointer, as a gallery specimen pins it. */
  pinned?: boolean
  /** What the newest instructions block holds, in its order; absent where the card lists no instructions. */
  instructions?: readonly InstructionEntry[]
}>()

const emit = defineEmits<{
  compact: []
  /** Opens where an entry is written: the settings for the personal instructions, the file for a project file. */
  openInstruction: [entry: InstructionEntry]
}>()

const rows = computed(() => instructionRows(props.instructions ?? []))
const instructionTokens = computed(() => rows.value.reduce((sum, row) => sum + (row.tokens ?? 0), 0))

function openRow(entry: InstructionEntry, close: () => void): void {
  close()
  emit('openInstruction', entry)
}

const percent = computed(() => contextPercent(props.usage ?? null))
const ratio = computed(() => contextRatio(props.usage ?? null) ?? 0)
const blockedReason = computed(() => props.unavailableReason ?? compactionRefusal(props.usage ?? null))

const radius = 5.5
const circumference = 2 * Math.PI * radius
const strokeDashoffset = computed(() => circumference * (1 - ratio.value))

const ringColor = computed(() => {
  if (percent.value == null)
    return 'text-fg-subtle'
  if (ratio.value >= 0.9)
    return 'text-on-danger'
  if (ratio.value >= 0.7)
    return 'text-on-warning'
  return 'text-fg-muted'
})
</script>

<template>
  <HoverCard :pinned="pinned">
    <button
      type="button"
      aria-label="Context usage"
      class="relative flex size-hit cursor-default items-center justify-center rounded-md transition-colors hover:bg-hover"
    >
      <IndeterminateSpinner v-if="isCompacting" />
      <svg
        v-else
        width="14"
        height="14"
        viewBox="0 0 14 14"
        class="-rotate-90"
      >
        <circle
          cx="7" cy="7" :r="radius"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          class="text-overlay/8"
        />
        <circle
          cx="7" cy="7" :r="radius"
          fill="none"
          stroke="currentColor"
          stroke-width="2.5"
          stroke-linecap="round"
          :stroke-dasharray="String(circumference)"
          :stroke-dashoffset="strokeDashoffset"
          :class="ringColor"
        />
      </svg>
    </button>
    <template #card>
      <span v-if="isCompacting" class="text-fg-body">Compacting context…</span>
      <span v-else-if="usage && percent != null" class="whitespace-nowrap">
        {{ percent }} used
        <span class="text-fg-subtle">({{ formatTokens(usage.tokens) }} / {{ formatTokens(usage.window) }})</span>
      </span>
      <span v-else class="text-fg-muted">Context usage unavailable</span>
    </template>
    <template v-if="!isCompacting" #action>
      <Button
        size="sm"
        :disabled="blockedReason != null"
        :disabled-reason="blockedReason ?? undefined"
        @click="emit('compact')"
      >
        Compact
      </Button>
    </template>
    <template v-if="instructions" #details="{ close }">
      <div class="flex items-center justify-between gap-6 px-3 pt-2 pb-1 text-[11px] font-medium text-fg-subtle">
        <span>Instructions</span>
        <span v-if="rows.length" class="tabular-nums">{{ formatTokens(instructionTokens) }}</span>
      </div>
      <ul v-if="rows.length" class="px-1 pb-1">
        <li v-for="row in rows" :key="row.path ?? 'personal'">
          <button
            type="button"
            :title="row.path ?? undefined"
            class="flex w-full items-center gap-2 rounded px-2 py-1 text-left hover:bg-hover"
            @click="openRow(row.entry, close)"
          >
            <UserRound v-if="row.path === null" :size="14" class="shrink-0 text-fg-subtle" />
            <FileText v-else :size="14" class="shrink-0 text-fg-subtle" />
            <span class="min-w-0 flex-1 truncate text-fg-body">{{ row.label }}</span>
            <span v-if="row.tokens === null" class="shrink-0 text-on-warning">Too large</span>
            <span v-else class="shrink-0 tabular-nums text-fg-subtle">{{ formatTokens(row.tokens) }}</span>
          </button>
        </li>
      </ul>
      <!-- As wide as the usage above it, never wider: the sentence wraps. -->
      <p v-else class="w-0 min-w-full px-3 pb-2 text-fg-subtle">No personal instructions, AGENTS.md or CLAUDE.md</p>
    </template>
  </HoverCard>
</template>

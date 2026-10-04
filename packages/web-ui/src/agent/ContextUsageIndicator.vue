<script setup lang="ts">
import { computed } from 'vue'
import type { ContextUsage } from '@demicodes/protocol'
import Button from '../ui/Button.vue'
import HoverCard from '../ui/HoverCard.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import { formatTokens } from '../ui/token-count'
import { compactionRefusal, contextPercent } from './context-usage'

/**
 * The composer's context meter: a ring of how full the next request is, as
 * the backend estimates it. Pointing at it or focusing it opens a card with
 * the numbers and Compact; a click on the ring itself does nothing.
 */
const props = defineProps<{
  usage?: ContextUsage | null
  isCompacting?: boolean
  /** Why Compact cannot run now apart from the usage, such as a running turn. */
  unavailableReason?: string | null
  /** The card shows without the pointer, as a gallery specimen pins it. */
  pinned?: boolean
}>()

const emit = defineEmits<{
  compact: []
}>()

const percent = computed(() => contextPercent(props.usage ?? null))
const ratio = computed(() => (percent.value ?? 0) / 100)
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
        {{ percent }}% used
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
  </HoverCard>
</template>

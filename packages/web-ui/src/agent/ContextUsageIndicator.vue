<script setup lang="ts">
import { computed } from 'vue'
import type { TokenUsage } from '@demicodes/core'
import IndeterminateSpinner from '@demicodes/web-ui/ui/IndeterminateSpinner.vue'
import Tooltip from '@demicodes/web-ui/ui/Tooltip.vue'
import { t } from '@demicodes/web-ui/infra/i18n'

const props = defineProps<{
  conversationId?: string
  usage?: TokenUsage | null
  contextWindow?: number | null
  inputLimit?: number | null
  isCompacting?: boolean
  isClickable?: boolean
  instructionFiles?: string[]
}>()

const emit = defineEmits<{
  compact: []
}>()

const EMPTY_USAGE: TokenUsage = { inputTokens: 0, outputTokens: 0, cacheReadTokens: 0, cacheWriteTokens: 0 }

const displayUsage = computed(() => props.usage ?? EMPTY_USAGE)
const isUsageAvailable = computed(() => props.usage != null)
const isTokenLimitAvailable = computed(() =>
  (props.inputLimit != null && props.inputLimit > 0)
  || (props.contextWindow != null && props.contextWindow > 0),
)
const usedTokens = computed(() =>
  displayUsage.value.inputTokens + displayUsage.value.outputTokens
  + (displayUsage.value.cacheReadTokens ?? 0) + (displayUsage.value.cacheWriteTokens ?? 0),
)
const effectiveLimit = computed(() => {
  if (props.inputLimit != null && props.inputLimit > 0) return props.inputLimit
  if (props.contextWindow != null && props.contextWindow > 0) return props.contextWindow
  return 1
})
const ratio = computed(() => {
  if (!isTokenLimitAvailable.value) return 0
  return Math.min(usedTokens.value / effectiveLimit.value, 1)
})
const percentage = computed(() => Math.round(ratio.value * 100))

const radius = 5.5
const circumference = 2 * Math.PI * radius
const strokeDashoffset = computed(() => circumference * (1 - ratio.value))

const ringColor = computed(() => {
  if (!isTokenLimitAvailable.value) return 'text-fg-subtle'
  if (ratio.value >= 0.9) return 'text-on-danger'
  if (ratio.value >= 0.7) return 'text-on-warning'
  return 'text-fg-muted'
})

function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${(tokens / 1_000_000).toFixed(1)}M`
  if (tokens >= 1_000) return `${(tokens / 1_000).toFixed(1)}K`
  return String(tokens)
}

function handleClick() {
  if (!props.isClickable || props.isCompacting) return
  emit('compact')
}
</script>

<template>
  <Tooltip>
    <span
      class="relative flex size-hit cursor-default items-center justify-center rounded-md transition-colors"
      :class="[
        isClickable && !isCompacting
          ? 'hover:bg-hover'
          : '',
      ]"
      @click="handleClick"
    >
      <IndeterminateSpinner v-if="isCompacting" />
      <svg v-else width="14" height="14" viewBox="0 0 14 14" class="-rotate-90">
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
    </span>
    <template #overlay>
      <template v-if="isCompacting">
        <div class="text-fg-body">{{ t('agent.context.compacting') }}</div>
      </template>
      <template v-else>
        <template v-if="isTokenLimitAvailable">
          <div class="text-fg">{{ percentage }}% used <span class="text-fg-subtle">({{ formatTokens(usedTokens) }} / {{ formatTokens(effectiveLimit) }})</span></div>
          <div v-if="!isUsageAvailable" class="mt-0.5 text-fg-subtle">{{ t('agent.context.noUsage') }}</div>
        </template>
        <div v-else class="text-fg-muted">{{ t('agent.context.unavailable') }}</div>
        <div
          v-for="file in instructionFiles"
          :key="file"
          class="mt-0.5 max-w-48 truncate text-left text-fg-subtle direction-rtl"
        >{{ file }}</div>
        <div v-if="isClickable" class="mt-1 text-fg-subtle">{{ t('agent.context.compactHint') }}</div>
      </template>
    </template>
  </Tooltip>
</template>

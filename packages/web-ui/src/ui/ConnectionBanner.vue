<script setup lang="ts">
import { WifiOff } from '@lucide/vue'
import { connectionWords, type ConnectionProblem } from '../transport/connection'
import IndeterminateSpinner from './IndeterminateSpinner.vue'
import { CONNECTION_BANNER_PX } from './app-bar'

/**
 * The strip across the top of the app while the backend cannot be reached, as Slack
 * and Linear show one. Everything under it stays readable, and it goes once the page
 * reaches the backend again. It takes its own row, so nothing under it is covered.
 */
defineProps<{
  problem: ConnectionProblem
}>()
</script>

<template>
  <div
    class="flex shrink-0 select-none items-center justify-center gap-1.5 bg-tint-warning px-4 text-chrome text-on-warning"
    :style="{ height: `${CONNECTION_BANNER_PX}px` }"
    role="status"
    aria-live="polite"
  >
    <WifiOff v-if="problem === 'offline'" :size="14" class="shrink-0" aria-hidden="true" />
    <IndeterminateSpinner v-else :size="12" class="shrink-0" />
    <span class="truncate">{{ connectionWords(problem) }}</span>
  </div>
</template>

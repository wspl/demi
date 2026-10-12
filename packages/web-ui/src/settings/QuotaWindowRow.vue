<script setup lang="ts">
import Meter from '../ui/Meter.vue'
import ResetTime from '../ui/ResetTime.vue'
import { formatPercent } from '../ui/percent'
import type { SettingsQuotaWindow } from './types'

/**
 * One usage window as every surface shows it (`usage-and-quota.md` § What a
 * snapshot says): one line over its bar, the window's name at the start, its
 * reset and its use at the end, "Resets in 4 hr 42 min · 4% used". Where the
 * line is short the reset truncates; the name and the use never do.
 */
defineProps<{ window: SettingsQuotaWindow }>()
</script>

<template>
  <div class="flex flex-col gap-1">
    <div class="flex items-baseline gap-3 whitespace-nowrap">
      <span class="shrink-0 text-fg-body">{{ window.label }}</span>
      <span class="ml-auto flex min-w-0 items-baseline">
        <ResetTime v-if="window.resetsAt" class="min-w-0 truncate" :timestamp="window.resetsAt" />
        <span class="shrink-0"><template v-if="window.resetsAt">&nbsp;·&nbsp;</template>{{ formatPercent(window.used, window.max) }} used</span>
      </span>
    </div>
    <Meter :value="window.used" :max="window.max" :label="window.label" />
  </div>
</template>

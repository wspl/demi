<script setup lang="ts">
import { computed } from 'vue'
import { clamp } from '@demicodes/utils'

/**
 * How far a task has come, as a thin bar: `value` of `max`. Unlike a `Meter`,
 * which warns as a quota fills, a full bar is simply done.
 */
const props = defineProps<{
  value: number
  max: number
  /** What is under way, for assistive technology. */
  label?: string
}>()

const ratio = computed(() => props.max > 0 ? clamp(props.value / props.max, 0, 1) : 0)
</script>

<template>
  <div
    class="h-1 w-full overflow-hidden rounded-full bg-overlay/8"
    role="progressbar"
    :aria-valuenow="value"
    :aria-valuemin="0"
    :aria-valuemax="max"
    :aria-label="label"
  >
    <div
      class="h-full rounded-full bg-accent-fill transition-[width] duration-150 ease-out"
      :style="{ width: `${ratio * 100}%` }"
    />
  </div>
</template>

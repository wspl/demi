<script setup lang="ts">
import { computed } from 'vue'

/** A quota bar. Turns warning past 80 percent and danger when full. */
const props = withDefaults(defineProps<{
  value: number
  max: number
  label?: string
}>(), {})

const ratio = computed(() => (props.max > 0 ? Math.min(1, Math.max(0, props.value / props.max)) : 0))
const tone = computed(() => (ratio.value >= 1 ? 'bg-on-danger' : ratio.value >= 0.8 ? 'bg-on-warning' : 'bg-accent-fill'))
</script>

<template>
  <div
    class="h-1 w-full overflow-hidden rounded-full bg-overlay/8"
    role="meter"
    :aria-valuenow="value"
    :aria-valuemin="0"
    :aria-valuemax="max"
    :aria-label="label"
  >
    <div class="h-full rounded-full transition-[width] duration-300 ease-out" :class="tone" :style="{ width: `${ratio * 100}%` }" />
  </div>
</template>

<script setup lang="ts">
import { ICON_PX } from './icon-metrics'

withDefaults(defineProps<{
  size?: number
}>(), {
  size: ICON_PX.in28,
})
</script>

<template>
  <svg
    :width="size"
    :height="size"
    viewBox="0 0 24 24"
    fill="none"
    class="activity-mark"
    aria-hidden="true"
  >
    <circle class="activity-track" cx="12" cy="12" r="8.25" />
    <circle class="activity-arc" cx="12" cy="12" r="8.25" />
  </svg>
</template>

<style scoped>
.activity-mark {
  display: block;
  overflow: visible;
}

.activity-track,
.activity-arc {
  fill: none;
  stroke: currentColor;
  stroke-width: var(--icon-stroke-width);
}

.activity-track {
  opacity: 0.18;
}

.activity-arc {
  stroke-linecap: round;
  stroke-dasharray: 36 18;
  transform-box: view-box;
  transform-origin: 12px 12px;
  animation: activity-spin 0.9s linear infinite;
}

@keyframes activity-spin {
  to { transform: rotate(360deg); }
}
</style>

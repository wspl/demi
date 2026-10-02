<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'

/**
 * A thin line along the top of what is loading, such as a page that a tab
 * still shows while the next one loads (`web-application.md` § Responding to
 * the user). It appears only after a moment, so a load that ends at once
 * shows nothing, and leaves at once when the load ends.
 */
const props = defineProps<{ active: boolean }>()

/** Long enough that a load the viewer would not notice shows nothing. */
const DELAY_MS = 150

const shown = ref(false)
let delay: ReturnType<typeof setTimeout> | null = null

function stop(): void {
  if (delay !== null) {
    clearTimeout(delay)
    delay = null
  }
}

watch(
  () => props.active,
  (active) => {
    stop()
    if (!active) {
      shown.value = false
      return
    }
    delay = setTimeout(() => {
      delay = null
      shown.value = true
    }, DELAY_MS)
  },
  { immediate: true },
)

onBeforeUnmount(stop)
</script>

<template>
  <div
    class="pointer-events-none absolute inset-x-0 top-0 z-10 h-0.5 overflow-hidden"
    role="progressbar"
    aria-label="Loading"
    :aria-hidden="!shown"
  >
    <div v-if="shown" class="progress-line h-full w-1/3 rounded-full bg-accent-fill" />
  </div>
</template>

<style scoped>
.progress-line {
  animation: progress-line 1.2s ease-in-out infinite;
}

@keyframes progress-line {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(300%); }
}

@media (prefers-reduced-motion: reduce) {
  .progress-line {
    width: 100%;
    animation: none;
    opacity: 0.6;
  }
}
</style>

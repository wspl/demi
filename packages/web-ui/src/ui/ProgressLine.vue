<script setup lang="ts">
/**
 * A thin line along the top of what is loading, such as a page that a tab
 * still shows while the next one loads (`web-application.md` § Responding to
 * the user). It shows exactly while `active`: from the frame the user acts
 * in, so an action on a far backend shows at once that it was taken.
 */
defineProps<{ active: boolean }>()
</script>

<template>
  <div
    class="pointer-events-none absolute inset-x-0 top-0 z-10 h-0.5 overflow-hidden"
    role="progressbar"
    aria-label="Loading"
    :aria-hidden="!active"
  >
    <div v-if="active" class="progress-line h-full w-1/3 rounded-full bg-accent-fill" />
  </div>
</template>

<style scoped>
.progress-line {
  animation: progress-line 1.2s ease-in-out infinite;
  /* The first frame already shows the whole segment, rather than one that slides in from the edge. */
  animation-delay: -0.45s;
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

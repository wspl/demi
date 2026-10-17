<script setup lang="ts">
/**
 * The agent's activity over a live view (`mobile.md` § The live view):
 * while the agent's input is being delivered, a note at the top says so,
 * and a ring marks where it last acted. The view places it over its
 * picture and gives the mark in the overlay's own CSS pixels; any source's
 * view uses it the same way.
 */
defineProps<{
  /** Whether the agent's input is being delivered now. */
  active: boolean
  /** What the note says, such as "The agent is using this device". */
  label: string
  /** Where the agent last acted, in the overlay's CSS pixels; none before it acted. */
  mark: { x: number; y: number } | null
}>()
</script>

<template>
  <div class="pointer-events-none absolute inset-0 overflow-hidden">
    <Transition
      enter-active-class="transition-opacity duration-150"
      leave-active-class="transition-opacity duration-150"
      enter-from-class="opacity-0"
      leave-to-class="opacity-0"
    >
      <div
        v-if="active"
        class="absolute left-1/2 top-2 flex max-w-[calc(100%-16px)] -translate-x-1/2 items-center gap-1.5 truncate rounded-full bg-surface-float px-2.5 py-1 text-[12px] leading-4 text-fg shadow"
        role="status"
      >
        <span class="size-1.5 flex-none rounded-full bg-accent-fill" aria-hidden="true" />
        <span class="truncate">{{ label }}</span>
      </div>
    </Transition>
    <div
      v-if="mark"
      :key="`${mark.x},${mark.y}`"
      class="live-activity-mark absolute -ml-3.5 -mt-3.5 size-7 rounded-full border-2 border-accent-fill bg-accent-fill/25"
      :class="{ 'live-activity-mark--idle': !active }"
      :style="{ left: `${mark.x}px`, top: `${mark.y}px` }"
      aria-hidden="true"
    />
  </div>
</template>

<style scoped>
/* A press: the ring grows where the agent acted, and fades once it is done. */
.live-activity-mark {
  animation: live-activity-press 500ms ease-out;
}

.live-activity-mark--idle {
  opacity: 0;
  transition: opacity 600ms ease-out 1200ms;
}

@keyframes live-activity-press {
  from {
    transform: scale(0.4);
    opacity: 0.3;
  }
}
</style>

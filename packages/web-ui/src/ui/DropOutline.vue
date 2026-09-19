<script setup lang="ts">
/**
 * The mark of where a drop goes: a dashed line just inside its positioned
 * parent's edge, its corners rounded by `radius`. An SVG stroke with a set
 * dash, since a CSS dashed border's segments are too short to read as a line.
 */
defineProps<{
  /** The corner radius, a CSS length. */
  radius: string
}>()
</script>

<template>
  <svg class="pointer-events-none absolute inset-0 h-full w-full text-line-focus" aria-hidden="true">
    <rect
      class="drop-outline"
      :style="{ '--drop-outline-radius': radius }"
      fill="none"
      stroke="currentColor"
      stroke-width="1.5"
      stroke-dasharray="8 6"
    />
  </svg>
</template>

<style scoped>
.drop-outline {
  x: 0.75px;
  y: 0.75px;
  width: calc(100% - 1.5px);
  height: calc(100% - 1.5px);
  rx: var(--drop-outline-radius);
}
</style>

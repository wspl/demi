<script setup lang="ts">
/**
 * One item of a status bar, as in an editor's: it stands in its own segment
 * the bar's full height inside its inset, and segments are told apart by their
 * spacing, never by dividers. A segment that acts when pressed (`acting`) is a
 * button that Tab reaches and shows a hover background over its whole extent;
 * one that only explains is plain text, which may carry a tip and has no
 * background. The bar (`.status-bar`) gives the text size and color, its
 * inset and, with `.status-bar-segment`, the segment's padding.
 */
defineProps<{ acting?: boolean }>()
</script>

<template>
  <button
    v-if="acting"
    type="button"
    data-acting
    data-drawn-target
    class="status-bar-segment flex h-full cursor-default items-center gap-1.5 whitespace-nowrap transition-colors hover:bg-hover"
  >
    <slot />
  </button>
  <span v-else class="status-bar-segment flex items-center gap-1.5 whitespace-nowrap">
    <slot />
  </span>
</template>

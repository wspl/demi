<script setup lang="ts">
/**
 * The text a comment is about, as the comment's list and the phone's field
 * show it: faint text after a gray bar, cut with an ellipsis after its lines.
 */
withDefaults(defineProps<{
  text: string
  lines?: 1 | 2
}>(), { lines: 2 })
</script>

<template>
  <p class="border-l-2 border-line-strong pl-2 text-detail text-fg-subtle" :class="lines === 1 ? 'line-clamp-1' : 'line-clamp-2'">{{ text }}</p>
</template>

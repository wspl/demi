<script setup lang="ts">
/**
 * A new item's mark in the corner of an icon: a green plus inside its lower
 * right, as an editor's tree marks an added file. The parent is `relative`;
 * the plus is cut out of what is painted behind it by an outline in that
 * color (`--fill-color`), as `CornerDot`'s ring cuts the dot out.
 */
defineProps<{ label?: string }>()
</script>

<template>
  <!-- A small plus, a semibold one's weight: two strokes, the outline first in
    the color behind it, then the plus in green. -->
  <svg
    viewBox="0 0 5 5"
    class="pointer-events-none absolute bottom-px right-0 size-[5px] overflow-visible"
    :role="label ? 'img' : undefined"
    :aria-label="label"
    :aria-hidden="label ? undefined : 'true'"
  >
    <path d="M2.5 0.5v4M0.5 2.5h4" stroke="var(--fill-color)" stroke-width="3.3" stroke-linecap="round" />
    <path d="M2.5 0.5v4M0.5 2.5h4" stroke="var(--on-success)" stroke-width="1.3" stroke-linecap="round" />
  </svg>
</template>

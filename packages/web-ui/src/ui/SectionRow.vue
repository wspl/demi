<script setup lang="ts">
/**
 * A card's section heading line: its text at the start and, in the `end`
 * slot, `xs` icon buttons at the end. The row owns the buttons' inset, so each
 * is as far from the row's end as from its top and bottom
 * ((row height - control height) / 2), as a menu row's actions are; a caller
 * never places them with a margin of its own. The row's side padding is sized
 * for text, so the end cell pulls back the difference.
 */
defineSlots<{
  default(): unknown
  end?(): unknown
}>()
</script>

<template>
  <div class="section-row">
    <div class="section-row-text">
      <slot />
    </div>
    <div v-if="$slots['end']" class="section-row-end on-fill">
      <slot name="end" />
    </div>
  </div>
</template>

<style scoped>
.section-row {
  --section-row-h: 1.75rem;
  --section-row-px: 0.75rem;
  display: flex;
  align-items: center;
  min-height: var(--section-row-h);
  padding-inline: var(--section-row-px);
}

.section-row-text {
  display: flex;
  min-width: 0;
  flex: 1;
}

.section-row-end {
  display: flex;
  flex-shrink: 0;
  align-items: center;
  gap: 2px;
  margin-left: 0.25rem;
  margin-right: calc((var(--section-row-h) - var(--spacing-hit-xs)) / 2 - var(--section-row-px));
}
</style>

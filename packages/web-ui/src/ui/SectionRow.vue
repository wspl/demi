<script setup lang="ts">
/**
 * A card's section heading line, and the one owner of how a section's title
 * is drawn: the card's own text size (`--text-detail`), medium, in the
 * subtle ink; a figure in the title takes its own ink. Its text stands at the
 * start and, in the `end` slot, `xs` icon buttons or a mini segmented control
 * at the end, laid out as anything else in the card: they end at the row's
 * content edge, where the card's text ends.
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
  font-size: var(--text-detail);
  font-weight: 500;
  color: var(--color-fg-subtle);
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
}
</style>

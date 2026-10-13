<script setup lang="ts">
import { vInkEdgeEnd } from './ink-edge'

/**
 * A card's section heading line: its text at the start and, in the `end`
 * slot, `xs` icon buttons at the end. The row has no box of its own, so the
 * last button's icon ends at its content edge, where the text and figures of
 * the card around it end (`ink-edge.ts`).
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
    <div v-if="$slots['end']" v-ink-edge-end class="section-row-end on-fill">
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
}
</style>

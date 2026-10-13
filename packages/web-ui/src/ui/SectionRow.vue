<script setup lang="ts">
/**
 * A card's section heading line: its text at the start and, in the `end`
 * slot, `xs` icon buttons at the end, laid out as anything else in the card:
 * they end at the row's content edge, where the card's text ends.
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
}
</style>

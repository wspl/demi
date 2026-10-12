<script setup lang="ts">
/**
 * One row of a hover card: what it says, and an optional `action` at its
 * end, one `sm` Button, which keeps the same inset from the row's top, end
 * and bottom, (row height - control height) / 2. The card's header is one;
 * its details may hold more, such as a line with the button that acts on it.
 */
defineSlots<{
  default(): unknown
  action?(): unknown
}>()
</script>

<template>
  <div class="hover-card-row">
    <div class="hover-card-row-body min-w-0 flex-1">
      <slot />
    </div>
    <div v-if="$slots.action" class="hover-card-row-action">
      <slot name="action" />
    </div>
  </div>
</template>

<style scoped>
/* The row's height and side padding are named so the action cell derives
   its inset from them. The control is an `sm` Button, 1.5rem high. */
.hover-card-row {
  --hover-card-h: 2.25rem;
  --hover-card-px: 0.75rem;
  --hover-card-action-h: 1.5rem;
  display: flex;
  align-items: stretch;
  min-height: var(--hover-card-h);
  padding-left: var(--hover-card-px);
}

.hover-card-row-body {
  display: flex;
  align-items: center;
  padding-block: 0.5rem;
  padding-right: var(--hover-card-px);
  line-height: 1.25rem;
}

.hover-card-row-action {
  display: flex;
  align-items: center;
  padding-right: calc((var(--hover-card-h) - var(--hover-card-action-h)) / 2);
}
</style>

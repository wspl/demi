<script setup lang="ts">
import { FOLD_MS } from './fold'

/**
 * A panel that opens and closes by height. Keep the content mounted so the
 * close can animate; `open` is the only state.
 */
defineProps<{
  open: boolean
}>()
</script>

<template>
  <div
    class="fold"
    :class="open ? 'is-open' : ''"
    :style="{ '--fold-ms': `${FOLD_MS}ms` }"
  >
    <div class="fold-clip" :inert="!open">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.fold {
  display: grid;
  grid-template-rows: 0fr;
  transition: grid-template-rows var(--fold-ms) ease;
}

.fold.is-open {
  grid-template-rows: 1fr;
}

.fold-clip {
  min-height: 0;
  overflow: hidden;
}

@media (prefers-reduced-motion: reduce) {
  .fold {
    transition: none;
  }
}
</style>

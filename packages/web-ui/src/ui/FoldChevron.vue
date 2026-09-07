<script setup lang="ts">
import { ChevronRight } from '@lucide/vue'
import { FOLD_MS } from './fold'
import { ICON_PX } from './icon-metrics'

/**
 * The fold affordance: points right when closed, down when open.
 * `visible: false` keeps the slot so trailing controls stay aligned.
 */
withDefaults(defineProps<{
  open: boolean
  visible?: boolean
}>(), {
  visible: true,
})
</script>

<template>
  <ChevronRight
    :size="ICON_PX.in28"
    aria-hidden="true"
    class="fold-chevron shrink-0"
    :class="[visible ? '' : 'invisible', open ? 'is-open' : '']"
    :style="{ '--fold-ms': `${FOLD_MS}ms` }"
  />
</template>

<style scoped>
.fold-chevron {
  transition: color var(--fold-ms) ease-out, rotate var(--fold-ms) ease-out;
}

.fold-chevron.is-open {
  rotate: 90deg;
}

@media (prefers-reduced-motion: reduce) {
  .fold-chevron {
    transition: none;
  }
}
</style>

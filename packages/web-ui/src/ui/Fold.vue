<script setup lang="ts">
import { ref, watch } from 'vue'
import { FOLD_MS } from './fold'

/**
 * A panel that opens and closes by height. The content mounts when the panel
 * first opens, as a closed `<details>` costs nothing, and then stays mounted
 * so the close can animate; `open` is the only state.
 */
const props = defineProps<{
  open: boolean
}>()

const opened = ref(props.open)
watch(() => props.open, (open) => {
  if (open)
    opened.value = true
})
</script>

<template>
  <div
    class="fold"
    :class="open ? 'is-open' : ''"
    :style="{ '--fold-ms': `${FOLD_MS}ms` }"
  >
    <div class="fold-clip" :inert="!open">
      <slot v-if="opened" />
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

<script setup lang="ts">
import { nextTick, ref, useTemplateRef, watch } from 'vue'
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
/**
 * Whether the panel is drawn open. A first open mounts the content closed and
 * has the browser lay it out so before opening it: content that appears in
 * the same style change that grows its row gives the transition nothing to
 * grow from, and the panel jumped open instead.
 */
const expanded = ref(props.open)
const root = useTemplateRef<HTMLElement>('root')
watch(() => props.open, async (open) => {
  if (!open || opened.value) {
    expanded.value = open
    return
  }
  opened.value = true
  await nextTick()
  // Reading a layout value makes the browser compute the closed state now.
  void root.value?.offsetHeight
  expanded.value = props.open
})
</script>

<template>
  <div
    ref="root"
    class="fold"
    :class="expanded ? 'is-open' : ''"
    :style="{ '--fold-ms': `${FOLD_MS}ms` }"
  >
    <div class="fold-clip overflow-hidden" :inert="!open">
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
}

@media (prefers-reduced-motion: reduce) {
  .fold {
    transition: none;
  }
}
</style>

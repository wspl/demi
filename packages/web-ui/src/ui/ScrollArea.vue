<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useOverlayScrollbars } from 'overlayscrollbars-vue'
import { scrollbarsOptions, scrollbarsTarget, type ScrollAxis } from './scrollbars'

/**
 * The app's scroller: its scrollbar floats over the content and takes no
 * room, so content keeps symmetric padding whether or not it overflows. The
 * bar shows while the pointer is over this scroller (not one around it),
 * while it scrolls and while its thumb is dragged (`scrollbars.ts`).
 *
 * The root is a flex item (`min-h-0`); the viewport inside it scrolls along
 * `axis` (vertically unless told otherwise) and clips the other axis. Padding
 * for rings drawn at the content's edges goes on `viewportClass`, as for any
 * scroll region.
 */
const props = withDefaults(defineProps<{
  /** Classes for the scrolling viewport: padding, flex layout of the content. */
  viewportClass?: string
  axis?: ScrollAxis
}>(), {
  axis: 'y',
})

const emit = defineEmits<{
  scroll: [event: Event]
}>()

const root = ref<HTMLElement>()
const viewport = ref<HTMLElement>()

// The composable destroys the instance on unmount and applies a changed axis.
const [initialize] = useOverlayScrollbars({
  options: computed(() => scrollbarsOptions(props.axis)),
})

// At once, not deferred: a caller reads the viewport's geometry as it mounts.
onMounted(() => {
  if (viewport.value && root.value)
    initialize(scrollbarsTarget(viewport.value, root.value))
})

defineExpose({
  /** The scrolling element, for scrollTop and scrollTo. */
  el: viewport,
})
</script>

<template>
  <div ref="root" class="scroll-area relative flex min-h-0 flex-col overflow-hidden">
    <!-- A stacking context of its own (a flex item with a z-index), as
         OverlayScrollbars gives a viewport it makes: what the content pins
         over itself (a sticky header) stays under the bar, which follows it
         in the root. -->
    <div
      ref="viewport"
      data-overlayscrollbars-initialize
      class="scroll-area-viewport z-0 min-h-0 flex-auto"
      :class="viewportClass"
      @scroll.passive="emit('scroll', $event)"
    >
      <slot />
    </div>
  </div>
</template>

<style scoped>
/* OverlayScrollbars positions the element it takes over, for bars placed
   inside it. These are placed in the root, so the viewport stays unpositioned
   and content placed against the root (Tree's pinned stack) stays put while
   the viewport scrolls. */
.scroll-area-viewport[data-overlayscrollbars] {
  position: static;
}
</style>

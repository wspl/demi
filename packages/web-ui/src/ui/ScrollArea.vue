<script setup lang="ts">
import { computed, ref, type CSSProperties } from 'vue'
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

// The bar is set up when it would first show, as the content scrollers'
// (`useContentScrollers`): the pointer comes onto the scroller, the focus
// moves into it, or it scrolls. A list mounts many scrollers, most never
// touched, and setting one up costs layouts. Until then the viewport scrolls
// natively with its native bar hidden, as OverlayScrollbars leaves an
// element it is about to take over, and its geometry is the same before and
// after, so a caller can read it as it mounts.
const started = ref(false)
function start(): void {
  if (started.value || !viewport.value || !root.value)
    return
  started.value = true
  initialize(scrollbarsTarget(viewport.value, root.value))
}

// Before the bar is set up, the viewport clips the axis it does not scroll
// along, as `scrollbarsOptions` has the bar do once it is.
const nativeOverflow = computed((): CSSProperties => ({
  overflowX: props.axis === 'y' ? 'hidden' : 'auto',
  overflowY: props.axis === 'x' ? 'hidden' : 'auto',
}))

function onScroll(event: Event): void {
  start()
  emit('scroll', event)
}

defineExpose({
  /** The scrolling element, for scrollTop and scrollTo. */
  el: viewport,
})
</script>

<template>
  <!-- While capturing, before the event reaches the viewport: the bar set up
       now hears this same event arrive and shows. -->
  <div
    ref="root"
    class="scroll-area relative flex min-h-0 flex-col overflow-hidden"
    @pointerover.capture.passive="start"
    @focusin="start"
  >
    <!-- A stacking context of its own (a flex item with a z-index), as
         OverlayScrollbars gives a viewport it makes: what the content pins
         over itself (a sticky header) stays under the bar, which follows it
         in the root. -->
    <div
      ref="viewport"
      data-overlayscrollbars-initialize
      class="scroll-area-viewport z-0 min-h-0 flex-auto"
      :class="viewportClass"
      :style="started ? undefined : nativeOverflow"
      @scroll.passive="onScroll"
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

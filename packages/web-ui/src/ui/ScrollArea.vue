<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'

/**
 * A vertical scroller whose scrollbar takes no room: the native bar is hidden and a
 * thumb is drawn over the content at the right edge. Content keeps symmetric padding
 * whether or not it overflows, and the thumb shows on hover or while scrolling.
 *
 * The root is a flex item (`min-h-0`); the viewport inside it scrolls. Padding for
 * rings drawn at the content's edges goes on `viewportClass`, as for any scroll region.
 */
const props = defineProps<{
  /** Classes for the scrolling viewport: padding, flex layout of the content. */
  viewportClass?: string
}>()

const emit = defineEmits<{
  scroll: [event: Event]
}>()

const viewport = ref<HTMLElement>()
const track = ref<HTMLElement>()
const thumbTop = ref(0)
const thumbHeight = ref(0)
const overflowing = ref(false)
const scrolling = ref(false)
const dragging = ref(false)
// Hover is tracked here, not with a CSS group: nested areas would light up together.
const hovered = ref(false)
let scrollTimer = 0
let observer: ResizeObserver | undefined

const MIN_THUMB = 24

function measure() {
  const el = viewport.value
  if (!el)
    return
  const { scrollHeight, clientHeight, scrollTop } = el
  overflowing.value = scrollHeight > clientHeight + 1
  if (!overflowing.value)
    return
  const trackHeight = track.value?.clientHeight ?? clientHeight
  const height = Math.max(MIN_THUMB, (clientHeight / scrollHeight) * trackHeight)
  const range = trackHeight - height
  thumbHeight.value = height
  thumbTop.value = (scrollTop / (scrollHeight - clientHeight)) * range
}

function onScroll(event: Event) {
  measure()
  scrolling.value = true
  window.clearTimeout(scrollTimer)
  scrollTimer = window.setTimeout(() => {
    scrolling.value = false
  }, 700)
  emit('scroll', event)
}

// Dragging the thumb scrolls the viewport by the same proportion.
let dragStartY = 0
let dragStartTop = 0

function onThumbDown(event: PointerEvent) {
  const el = viewport.value
  if (!el)
    return
  dragging.value = true
  dragStartY = event.clientY
  dragStartTop = el.scrollTop
  ;(event.currentTarget as HTMLElement).setPointerCapture(event.pointerId)
  event.preventDefault()
}

function onThumbMove(event: PointerEvent) {
  const el = viewport.value
  if (!dragging.value || !el)
    return
  const trackHeight = track.value?.clientHeight ?? el.clientHeight
  const range = trackHeight - thumbHeight.value
  if (range <= 0)
    return
  const scrollRange = el.scrollHeight - el.clientHeight
  el.scrollTop = dragStartTop + ((event.clientY - dragStartY) / range) * scrollRange
}

function onThumbUp() {
  dragging.value = false
}

// A click on the track jumps a page in that direction, as native bars do.
function onTrackDown(event: PointerEvent) {
  const el = viewport.value
  if (!el || event.target !== track.value)
    return
  const rect = track.value.getBoundingClientRect()
  const below = event.clientY - rect.top > thumbTop.value + thumbHeight.value
  el.scrollBy({ top: (below ? 1 : -1) * el.clientHeight, behavior: 'smooth' })
}

onMounted(() => {
  measure()
  const el = viewport.value
  if (!el || typeof ResizeObserver === 'undefined')
    return
  observer = new ResizeObserver(measure)
  observer.observe(el)
  for (const child of el.children) observer.observe(child)
})

onBeforeUnmount(() => {
  observer?.disconnect()
  window.clearTimeout(scrollTimer)
})

// Content that mounts later (a v-if page) is picked up on the next measurement.
watch(() => props.viewportClass, measure)

const thumbVisible = computed(
  () => overflowing.value && (hovered.value || scrolling.value || dragging.value)
)

defineExpose({
  /** The scrolling element, for scrollTop and scrollTo. */
  el: viewport,
  measure,
})
</script>

<template>
  <div
    class="scroll-area relative min-h-0 overflow-hidden"
    @pointerenter="hovered = true"
    @pointerleave="hovered = false"
  >
    <div
      ref="viewport"
      class="scroll-area-viewport h-full overflow-y-auto"
      :class="viewportClass"
      @scroll.passive="onScroll"
    >
      <slot />
    </div>
    <div
      v-show="overflowing"
      ref="track"
      class="absolute inset-y-0.5 right-0.5 w-1.5"
      @pointerdown="onTrackDown"
    >
      <div
        class="scroll-area-thumb absolute inset-x-0 rounded-full transition-[opacity,background-color] duration-250 ease-out"
        :class="[thumbVisible ? 'opacity-100' : 'opacity-0', dragging ? 'bg-overlay/25' : 'bg-overlay/12 hover:bg-overlay/25']"
        :style="{ top: `${thumbTop}px`, height: `${thumbHeight}px` }"
        @pointerdown="onThumbDown"
        @pointermove="onThumbMove"
        @pointerup="onThumbUp"
        @pointercancel="onThumbUp"
      />
    </div>
  </div>
</template>

<style scoped>
/* The native bar is hidden; the drawn thumb is the only one. */
.scroll-area-viewport {
  scrollbar-width: none;
}

.scroll-area-viewport::-webkit-scrollbar {
  display: none;
}

@media (prefers-reduced-motion: reduce) {
  .scroll-area-thumb {
    transition: none;
  }
}
</style>

<script setup lang="ts">
import { onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { inTextField } from './context-menu'
import { heldSelection, NO_SHAPES, selectionShapes, type DrawnShapes } from './drawn-selection'
import { usePhone } from './phone'

/**
 * Draws the page's selection in `root`, where the browser paints none
 * (`drawn-selection.ts`): the browser's selection while it lies in `root`,
 * and else the one the page holds for a field that took the focus. A phone
 * keeps the system's selection, which comes with its own handles.
 *
 * The layer stands over the rows and blends into them, so the words and the
 * backgrounds under a strip keep their colors.
 */
const props = defineProps<{
  /** The element the list's rows stand in, which this layer covers. */
  root: HTMLElement | null
}>()

const phone = usePhone()
const layer = ref<HTMLElement>()
const shapes = shallowRef<DrawnShapes>(NO_SHAPES)

/** The browser's selection, unless it is collapsed or in a text field, which paints its own. */
function browserSelection(): Range | null {
  const selection = window.getSelection()
  if (!selection || selection.isCollapsed || selection.rangeCount === 0)
    return null
  const range = selection.getRangeAt(0)
  const start = range.startContainer instanceof Element ? range.startContainer : range.startContainer.parentElement
  return start && inTextField(start) ? null : range
}

function measure(): void {
  const root = props.root
  const range = root && !phone.value ? browserSelection() ?? heldSelection.value : null
  const next = root && range ? selectionShapes(range, root) : NO_SHAPES
  const none = next.strips.length === 0 && next.pictures.length === 0
  if (!none || shapes.value !== NO_SHAPES)
    shapes.value = none ? NO_SHAPES : next
  // A row that slides to its place, or a fold that opens, moves the words
  // with no change this layer hears of: it follows them until they stop. A
  // shimmer or a spinner, which never stops, moves no words.
  if (!none && root?.getAnimations({ subtree: true }).some((animation) => animation.playState === 'running' && animation.effect?.getComputedTiming().iterations !== Infinity))
    schedule()
}

let frame = 0
function schedule(): void {
  if (frame === 0) {
    frame = requestAnimationFrame(() => {
      frame = 0
      measure()
    })
  }
}

// The root comes and goes with the list: everything that follows it is released with it.
watch([() => props.root, phone], ([root, onPhone], _before, onCleanup) => {
  if (!root || onPhone) {
    shapes.value = NO_SHAPES
    return
  }
  root.setAttribute('data-drawn-selection', '')
  const listeners = new AbortController()
  document.addEventListener('selectionchange', schedule, { signal: listeners.signal })
  // A box that scrolls inside a row, as a wide code block, moves its words; the list's own scroll moves this layer with them.
  root.addEventListener('scroll', schedule, { capture: true, passive: true, signal: listeners.signal })
  // A row the list moves changes its style; a reply that streams, its text. This layer's own strips move no words.
  const mutations = new MutationObserver((records) => {
    if (records.some((record) => !layer.value?.contains(record.target)))
      schedule()
  })
  mutations.observe(root, { childList: true, subtree: true, characterData: true, attributes: true, attributeFilter: ['style'] })
  const sizes = new ResizeObserver(schedule)
  sizes.observe(root)
  schedule()
  onCleanup(() => {
    root.removeAttribute('data-drawn-selection')
    listeners.abort()
    mutations.disconnect()
    sizes.disconnect()
  })
}, { immediate: true })

watch(heldSelection, schedule)

onBeforeUnmount(() => cancelAnimationFrame(frame))
</script>

<template>
  <div ref="layer" data-drawn-selection-layer aria-hidden="true" class="drawn-selection pointer-events-none absolute inset-0 z-20">
    <div
      v-for="(strip, index) in shapes.strips"
      :key="`strip:${index}`"
      class="drawn-selection-strip absolute"
      :style="{ top: `${strip.top}px`, left: `${strip.left}px`, width: `${strip.width}px`, height: `${strip.height}px` }"
    />
    <div
      v-for="(picture, index) in shapes.pictures"
      :key="`picture:${index}`"
      class="drawn-selection-picture absolute"
      :style="{ top: `${picture.top}px`, left: `${picture.left}px`, width: `${picture.width}px`, height: `${picture.height}px`, borderRadius: `${picture.radius}px`, clipPath: `inset(${picture.clip})` }"
    />
  </div>
</template>

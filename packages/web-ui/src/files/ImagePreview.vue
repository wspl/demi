<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch } from 'vue'
import { useElementSize } from '@vueuse/core'
import { dropSource } from './preview'
import { startPointerDrag } from '../ui/pointer-drag'
import {
  fittedView,
  isClick,
  isFitted,
  pannedView,
  resizedView,
  toggledView,
  wheelScale,
  zoomAbout,
  type FramePoint,
  type ImageView,
} from './image-zoom'
import type { PixelSize } from './thumbnail'

/**
 * A picture as an image viewer shows it (`file-previews.md` § The media
 * viewer, Zoom and move), in the media viewer's window and in the File
 * view: it opens fitted, scaled down and never enlarged; the wheel, and a
 * trackpad's pinch, zoom it about the pointer from the fitted size up to
 * eight times the actual size; while it is larger than its frame a drag
 * moves it, with the grab hand, and it never leaves an empty band on a side
 * that still has pixels; a click without a drag toggles the fitted and the
 * actual size about the clicked spot. A frame that changes size keeps a
 * fitted picture fitted and a zoomed one at its zoom; another picture opens
 * fitted. Transparency shows over a checkerboard. On a phone a tap toggles
 * and a drag moves the picture the same way, and a pinch zooms the page as
 * anywhere on it.
 *
 * It reports the picture's pixel size once loaded, and its zoom whenever it
 * changes, for the host to show beside it.
 */
const props = defineProps<{
  src: string
  name: string
  /** Edge to edge, with no margin around the picture, as a window that is sized to it shows it. */
  flush?: boolean
}>()
const emit = defineEmits<{
  size: [width: number, height: number]
  /** The picture's scale of its actual size, 1.5 for 150%. */
  zoom: [scale: number]
  failed: []
}>()

const image = ref<HTMLImageElement | null>(null)
const frame = ref<HTMLElement | null>(null)
const { width: frameWidth, height: frameHeight } = useElementSize(frame)
const room = computed<PixelSize | null>(() => frameWidth.value > 0 && frameHeight.value > 0
  ? { width: frameWidth.value, height: frameHeight.value }
  : null)
/** The picture's actual size, once it loaded. */
const natural = shallowRef<PixelSize | null>(null)
const view = shallowRef<ImageView | null>(null)

watch(() => props.src, () => {
  natural.value = null
  view.value = null
})

// The picture opens fitted once both its size and its frame's are known, and follows the frame's size after.
watch(
  [natural, room],
  ([size, now], [, before]) => {
    if (!size || !now)
      return
    view.value = view.value && before ? resizedView(view.value, size, before, now) : fittedView(size, now)
  },
)

watch(() => view.value?.scale, (scale) => {
  if (scale !== undefined)
    emit('zoom', scale)
})

/** The picture is larger than its frame on some side, so a drag moves it. */
const movable = computed(() => {
  const size = natural.value
  const shown = view.value
  const now = room.value
  if (!size || !shown || !now)
    return false
  return size.width * shown.scale > now.width + 0.5 || size.height * shown.scale > now.height + 0.5
})

/** What a click would do, as the pointer says it: move a large picture, zoom in a fitted one, fit a zoomed one. */
const cursor = computed(() => {
  const size = natural.value
  const shown = view.value
  const now = room.value
  if (!size || !shown || !now)
    return 'default'
  if (movable.value)
    return 'grab'
  if (!isFitted(shown, size, now))
    return 'zoom-out'
  return shown.scale < 1 ? 'zoom-in' : 'default'
})

const imageStyle = computed(() => {
  const size = natural.value
  const shown = view.value
  if (!size || !shown)
    return undefined
  return {
    width: `${size.width * shown.scale}px`,
    height: `${size.height * shown.scale}px`,
    transform: `translate(${shown.x}px, ${shown.y}px)`,
  }
})

function loaded(): void {
  const element = image.value
  if (!element)
    return
  const { naturalWidth: width, naturalHeight: height } = element
  // An SVG with no size of its own has none to scale from: it takes its frame's, or CSS's default
  // object size, 300 × 150, while the frame is not measured yet.
  natural.value = width > 0 && height > 0 ? { width, height } : room.value ?? { width: 300, height: 150 }
  emit('size', width, height)
}

/** Where `event` is in the frame. */
function framePoint(event: MouseEvent): FramePoint {
  const box = frame.value?.getBoundingClientRect()
  return { x: event.clientX - (box?.left ?? 0), y: event.clientY - (box?.top ?? 0) }
}

/** A wheel line or page as pixels, as the page would scroll by it. */
function wheelPixels(event: WheelEvent): number {
  if (event.deltaMode === WheelEvent.DOM_DELTA_LINE)
    return event.deltaY * 16
  if (event.deltaMode === WheelEvent.DOM_DELTA_PAGE)
    return event.deltaY * (room.value?.height ?? 0)
  return event.deltaY
}

function onWheel(event: WheelEvent): void {
  const size = natural.value
  const shown = view.value
  const now = room.value
  if (!size || !shown || !now)
    return
  // The wheel zooms the picture, never the page behind it; a pinch comes as a wheel with Control.
  event.preventDefault()
  view.value = zoomAbout(shown, wheelScale(shown.scale, wheelPixels(event), event.ctrlKey), framePoint(event), size, now)
}

/** Ends the press under way early, as its release would. */
let stopPress: (() => void) | null = null

function onPress(event: PointerEvent): void {
  const size = natural.value
  const start = view.value
  if (event.button !== 0 || stopPress || !size || !start || !room.value)
    return
  // No text selection, and no dragging of the picture out of the page, starts under the press.
  event.preventDefault()
  const at = framePoint(event)
  let dragged = false
  stopPress = startPointerDrag(event, {
    cursor: movable.value ? 'grabbing' : cursor.value,
    move(dx, dy) {
      dragged ||= !isClick(dx, dy)
      if (dragged && room.value)
        view.value = pannedView(start, dx, dy, size, room.value)
    },
    end(cancelled) {
      stopPress = null
      if (cancelled)
        view.value = start
      else if (!dragged && room.value)
        view.value = toggledView(view.value ?? start, at, size, room.value)
    },
  })
}

onBeforeUnmount(() => {
  stopPress?.()
  if (image.value)
    dropSource(image.value)
})
</script>

<template>
  <div class="h-full w-full" :class="flush ? '' : 'p-4'">
    <div
      ref="frame"
      class="relative h-full w-full touch-pinch-zoom select-none overflow-hidden"
      :style="{ cursor }"
      @wheel="onWheel"
      @pointerdown="onPress"
    >
      <img
        ref="image"
        :src="src"
        :alt="name"
        draggable="false"
        class="checkerboard absolute left-0 top-0 max-w-none"
        :class="imageStyle ? '' : 'invisible'"
        :style="imageStyle"
        @load="loaded"
        @error="emit('failed')"
      >
    </div>
  </div>
</template>

<script setup lang="ts">
import { LONG_PRESS_MS } from '../ui/context-menu'
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { useDevicePixelRatio, useElementSize } from '@vueuse/core'
import { dropSource } from './preview'
import { startPointerDrag } from '../ui/pointer-drag'
import {
  fittedView,
  isClick,
  isFitted,
  pannedView,
  resizedView,
  rescaledView,
  toggledView,
  wheelScale,
  zoomAbout,
  type FramePoint,
  type ImageView,
} from './image-zoom'
import { startsSwipe, swipeAxis, type StepReach, type SwipeAxis } from './swipe-step'
import { useSwipeLayer } from './swipe-layer'
import type { PixelSize } from './thumbnail'

/**
 * A picture as an image viewer shows it, in the media viewer's window and in the
 * File view.
 *
 * It opens fitted, scaled down and never enlarged. Its actual size is one of its
 * pixels on one device pixel, so 100% of a 2880-pixel-wide picture is 1440 page
 * pixels wide on a display of two device pixels per page pixel, and all the zoom
 * rules below count from that.
 *
 * Pointer and trackpad:
 *
 * - The wheel, and a trackpad's pinch, zoom it about the pointer from the fitted
 *   size up to eight times the actual size.
 * - While it is larger than its frame a drag moves it, with the grab hand, and it
 *   never leaves an empty band on a side that still has pixels.
 * - A click without a drag toggles the fitted and the actual size about the
 *   clicked spot.
 *
 * A frame that changes size keeps a fitted picture fitted and a zoomed one at its
 * zoom. Another picture opens fitted. Transparency shows over a checkerboard.
 *
 * Phone:
 *
 * - A tap toggles and a drag moves the picture the same way, and a pinch zooms
 *   it about the fingers, which the page itself never does (`phone-page.ts`).
 *   A long press is no tap: it leaves the picture as it is, under the
 *   system's menu for it where the system has one.
 * - With `dismissible`, as the phone's full screen and the work panel's
 *   overlay show it, a tap beside the picture, outside the box it is drawn in,
 *   asks to close it instead. A finger pulling the picture down at its fitted
 *   size moves it down and fades it; lifted past 72 px it asks to close it, and
 *   short of that, or cut short by the system, it springs back.
 *
 * With `steps` given, a finger swiped sideways across the picture at its fitted
 * size steps to the neighbour (`swipe-layer.ts`):
 *
 * - The picture follows the finger, then slides out to the step or springs back.
 * - A zoomed picture is moved by the drag instead.
 * - A picture that arrives by a swipe slides in from the side it came from.
 * - With reduced motion it only steps.
 *
 * It reports the picture's pixel size once loaded, and its zoom whenever it
 * changes, for the host to show beside it.
 */
const props = defineProps<{
  src: string
  name: string
  /** Edge to edge, with no margin around the picture, as a window that is sized to it shows it. */
  flush?: boolean
  /** The neighbours a swipe can step to; none when the picture is not swiped. */
  steps?: StepReach
  /** The side a swipe brought this picture from, 1 the right and -1 the left, 0 when it did not slide in. */
  from?: -1 | 0 | 1
  /** A tap beside the picture closes it rather than toggling its size, where the picture covers the screen. */
  dismissible?: boolean
}>()
const emit = defineEmits<{
  size: [width: number, height: number]
  /** The picture's scale of its actual size, 1.5 for 150%. */
  zoom: [scale: number]
  failed: []
  /** A swipe stepped to the next (1) or the previous (-1) picture; `slid` when this one slid out, so the next slides in. */
  step: [direction: -1 | 1, slid: boolean]
  /** A tap landed beside the picture of a `dismissible` preview. */
  dismiss: []
}>()

const image = ref<HTMLImageElement | null>(null)
const frame = ref<HTMLElement | null>(null)
const { width: frameWidth, height: frameHeight } = useElementSize(frame)
const room = computed<PixelSize | null>(() => frameWidth.value > 0 && frameHeight.value > 0
  ? { width: frameWidth.value, height: frameHeight.value }
  : null)
const { pixelRatio } = useDevicePixelRatio()
/** The picture's pixel size, once it loaded. */
const pixels = shallowRef<PixelSize | null>(null)
/** The picture at its actual size, in page px: one of its pixels on one device pixel. */
const natural = computed<PixelSize | null>(() => pixels.value
  ? { width: pixels.value.width / pixelRatio.value, height: pixels.value.height / pixelRatio.value }
  : null)
const view = shallowRef<ImageView | null>(null)
/** The layer the picture slides on while a finger swipes it (`swipe-layer.ts`). */
const slider = ref<HTMLElement | null>(null)
const swipe = useSwipeLayer(slider, () => room.value, {
  reach: () => props.steps,
  dismissible: () => props.dismissible,
  step: (direction, slid) => emit('step', direction, slid),
  dismiss: () => emit('dismiss'),
})

watch(() => props.src, () => {
  pixels.value = null
  view.value = null
  swipe.reset()
})

// A display with another pixel ratio, such as the window moved to it, changes the zoom the picture
// reads at, not its size on the page: the picture stays as large as it was.
watch(pixelRatio, (now, before) => {
  if (view.value)
    view.value = rescaledView(view.value, now / before)
}, { flush: 'sync' })

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
  // object size, 300 × 150, while the frame is not measured yet, in page px.
  const own = room.value ?? { width: 300, height: 150 }
  pixels.value = width > 0 && height > 0
    ? { width, height }
    : { width: own.width * pixelRatio.value, height: own.height * pixelRatio.value }
  emit('size', width, height)
}

/** Whether the frame's point `at` lies on the picture as it is drawn. */
function onPicture(at: FramePoint): boolean {
  const size = natural.value
  const shown = view.value
  if (!size || !shown)
    return false
  return at.x >= shown.x && at.x <= shown.x + size.width * shown.scale
    && at.y >= shown.y && at.y <= shown.y + size.height * shown.scale
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
/** Two fingers zoom the picture: their distance and midpoint as last seen. */
let pinch: { distance: number; at: FramePoint } | null = null
/** A pinch took over the press under way, whose release then neither toggles nor moves. */
let pinched = false

function onPress(event: PointerEvent): void {
  const size = natural.value
  const start = view.value
  const reach = props.steps
  if (event.button !== 0)
    return
  // No text selection, and no dragging of the picture out of the page, starts under the press; and
  // every press on the frame is the picture's, a pinch's second finger included, so nothing around
  // it takes the press for a tap on empty space (`MediaViewer`).
  event.preventDefault()
  if (stopPress || swipe.leaving() || !size || !start || !room.value)
    return
  // A press during a spring back takes the picture where the finger finds it.
  swipe.hold()
  const at = framePoint(event)
  // A fitted picture swipes where it steps sideways or closes with a pull down, either alone.
  const swiping = (reach !== undefined || props.dismissible) && startsSwipe(event.pointerType, isFitted(start, size, room.value))
  const pressedAt = performance.now()
  let axis: SwipeAxis = 'click'
  let last = { dx: 0, dy: 0 }
  let dragged = false
  stopPress = startPointerDrag(event, {
    cursor: movable.value ? 'grabbing' : cursor.value,
    move(dx, dy) {
      if (pinched)
        return
      if (swiping) {
        // The direction the drag first leaves a click's reach in is the one it keeps.
        if (axis === 'click')
          axis = swipeAxis(dx, dy)
        last = { dx, dy }
        swipe.follow(axis, dx, dy)
        return
      }
      dragged ||= !isClick(dx, dy)
      if (dragged && room.value)
        view.value = pannedView(start, dx, dy, size, room.value)
    },
    end(cancelled, released) {
      stopPress = null
      const wasPinched = pinched
      pinched = false
      // A swipe sideways or down ends in the layer; a pinch that took it over puts the picture back.
      if (swiping && (axis !== 'click' || wasPinched)) {
        swipe.release(axis, last.dx, last.dy, released && !cancelled && !wasPinched, performance.now() - pressedAt)
        return
      }
      if (wasPinched)
        return
      // A finger held as long as a long press asked for the system's menu, not for a tap.
      if (!cancelled && !dragged && event.pointerType === 'touch' && performance.now() - pressedAt >= LONG_PRESS_MS)
        return
      // A tap acts only when the finger itself lifted: a press the system cancelled, or one cut short,
      // neither zooms nor closes.
      if (cancelled)
        view.value = start
      else if (!released)
        return
      else if (!dragged && props.dismissible && !onPicture(at))
        emit('dismiss')
      else if (!dragged && room.value)
        view.value = toggledView(view.value ?? start, at, size, room.value)
    },
  })
}

/** The two fingers' distance and midpoint in the frame. */
function fingers(event: TouchEvent): { distance: number; at: FramePoint } | null {
  const [a, b] = [event.touches[0], event.touches[1]]
  if (event.touches.length !== 2 || !a || !b)
    return null
  const box = frame.value?.getBoundingClientRect()
  return {
    distance: Math.hypot(a.clientX - b.clientX, a.clientY - b.clientY),
    at: { x: (a.clientX + b.clientX) / 2 - (box?.left ?? 0), y: (a.clientY + b.clientY) / 2 - (box?.top ?? 0) },
  }
}

function onTouchStart(event: TouchEvent): void {
  pinch = fingers(event)
  if (pinch)
    pinched = stopPress !== null
}

function onTouchMove(event: TouchEvent): void {
  const next = fingers(event)
  const size = natural.value
  const shown = view.value
  const now = room.value
  if (!pinch || !next || !size || !shown || !now || pinch.distance === 0)
    return
  event.preventDefault()
  // The picture grows about the fingers' midpoint and follows it as it moves.
  const zoomed = zoomAbout(shown, shown.scale * (next.distance / pinch.distance), next.at, size, now)
  view.value = pannedView(zoomed, next.at.x - pinch.at.x, next.at.y - pinch.at.y, size, now)
  pinch = next
}

function onTouchEnd(event: TouchEvent): void {
  pinch = fingers(event)
}

onMounted(() => {
  // A picture that a swipe brought slides in from the side it came from.
  if (props.from)
    swipe.arrive(props.from)
})

onBeforeUnmount(() => {
  stopPress?.()
  swipe.dispose()
  if (image.value)
    dropSource(image.value)
})
</script>

<template>
  <div class="h-full w-full" :class="flush ? '' : 'p-4'">
    <div
      ref="frame"
      class="relative h-full w-full touch-none select-none overflow-hidden"
      :style="{ cursor }"
      @wheel="onWheel"
      @pointerdown="onPress"
      @touchstart.passive="onTouchStart"
      @touchmove="onTouchMove"
      @touchend.passive="onTouchEnd"
      @touchcancel.passive="onTouchEnd"
    >
      <div ref="slider" class="absolute inset-0" :style="swipe.style.value">
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
  </div>
</template>

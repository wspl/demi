<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import {
  Button,
  CanvasPictures,
  LiveActivityOverlay,
  LiveDrawnFrame,
  RegionStatus,
  SQUARE_CORNERS,
  keyInput,
  observeSettledSize,
  scaledCorners,
  screenClip,
  type ScreenCorners,
  touchInput,
  wheelInput,
} from '@demicodes/plugin-sdk'
import type { DeviceStream, MobileSession } from './session'
import { areaPoint, devicePoint, frameRoom, placeInFrame, placeScreen, type Placement, type Size } from './view'
import { insertedText } from './typed'
import { t } from '../locales/text'

/**
 * A simulator, live (`packages/plugin-mobile/src/index.ts`): its pictures on a
 * canvas, in the device's frame, fitted into the tab.
 *
 * - A click or a finger on it is a touch, a drag a swipe, and the keyboard types.
 * - Over it show the agent's activity: the note that it is using the device and the
 *   ring where it acted.
 * - A screen that is off keeps its last picture, dimmed under Screen Off with Wake;
 *   a tap or a key wakes it too.
 * - The view stays mounted while its tab is hidden, so its last picture shows at
 *   once, in its last frame, when the tab is shown again. The session watches the device only while its
 *   tab is shown.
 */
const props = defineProps<{
  session: MobileSession
  device: string
  shown: boolean
}>()

const area = ref<HTMLDivElement | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
const keys = ref<HTMLInputElement | null>(null)
const size = ref<Size>({ width: 0, height: 0 })
/** The area's size once a change of it has held, as after a drag of the panel's edge. */
const settledSize = ref<Size>({ width: 0, height: 0 })
/** The generation the canvas shows, which places it and maps input on it. */
const picture = shallowRef<DeviceStream | null>(null)
const painted = ref(false)
const watching = computed(() => props.shown && props.session.state.watched === props.device)
/** The device's last frame as the module drew it, kept with its last picture until a newer one comes. */
const framed = computed(() => props.session.state.frames[props.device] ?? null)
/** The screen in points: the picture's, or before the first picture the frame's, so the view says its size at once. */
const screen = computed<Size | null>(() => {
  if (picture.value) {
    return { width: picture.value.pointsWidth, height: picture.value.pointsHeight }
  }
  return framed.value ? { width: framed.value.pointsWidth, height: framed.value.pointsHeight } : null
})
/** The frame drawn for the picture's orientation; a frame of the orientation before a turn is not this picture's. */
const frame = computed(() => {
  const drawn = framed.value
  return drawn && drawn.orientation === (picture.value?.orientation ?? drawn.orientation) ? drawn.frame : null
})
type Layout = { screen: Placement; corners: ScreenCorners; image: { src: string; box: Placement } | null }

/** Where the picture sits in an area of `area`, with its corners, and the drawn frame around it, if any. */
function layoutIn(area: Size): Layout | null {
  if (!screen.value) {
    return null
  }
  const drawn = frame.value
  if (drawn?.kind === 'image') {
    const placed = placeInFrame(drawn, screen.value, area)
    return {
      screen: placed.screen,
      corners: placed.corners,
      image: { src: `data:image/png;base64,${drawn.png}`, box: placed.frame },
    }
  }
  const placed = placeScreen(screen.value, area)
  const corners = drawn?.kind === 'drawn' ? scaledCorners(drawn.corners, placed.width / screen.value.width) : SQUARE_CORNERS
  return { screen: placed, corners, image: null }
}

/** The picture follows the area at once. */
const layout = computed(() => layoutIn(size.value))
const placement = computed(() => layout.value?.screen ?? null)
/** The frame the view draws itself when none is drawn for the device. */
const drawnFrame = computed(() => {
  const drawn = frame.value
  if (drawn?.kind !== 'drawn' || !placement.value || !screen.value) {
    return null
  }
  return { corners: layout.value?.corners ?? SQUARE_CORNERS, buttons: drawn.buttons }
})
/**
 * How far the picture reaches past the screen's edge under a drawn frame's
 * image, in device pixels: past the hole's antialiased edge, and well within
 * the bezel's opaque rim.
 */
const UNDER_BEZEL = 2

/** The screen's shape, which the screen-off overlay over the frame takes. */
const screenShape = computed(() => {
  const placed = placement.value
  return placed && layout.value ? screenClip(placed.width, placed.height, layout.value.corners) : undefined
})
/**
 * The picture cut to the screen's corners, which the device draws black.
 * Under a drawn frame's image, whose hole is cut in the same shape, it
 * reaches a little past that shape, so the hole's own edge is the only one
 * that shows: two antialiased edges on one line let the area's background
 * through between them, a light seam around the corners.
 */
const clip = computed(() => {
  const placed = placement.value
  if (!placed || !layout.value) {
    return undefined
  }
  const offset = layout.value.image ? UNDER_BEZEL / window.devicePixelRatio : 0
  return screenClip(placed.width, placed.height, layout.value.corners, offset)
})
const activity = computed(() => watching.value ? props.session.state.activity : { active: false })
const mark = computed(() => {
  const point = activity.value.point
  return point && screen.value && placement.value ? areaPoint(point, screen.value, placement.value) : null
})
const stalled = computed(() => props.session.state.connection === 'stalled')
const screenOff = computed(() => watching.value && painted.value && props.session.state.screenOff)

// The Host captures the screen no larger than the view shows it, in device
// pixels, once the area's size settled: a larger size restarts its capture,
// which a drag of the panel's edge would otherwise do at every step.
const settledScreen = computed(() => {
  const area = settledSize.value
  return watching.value && area.width > 0 && area.height > 0 ? layoutIn(area)?.screen ?? null : null
})
watch(() => settledScreen.value ? [settledScreen.value.width, settledScreen.value.height] as const : null, (shownAt) => {
  if (shownAt) {
    props.session.showsScreenAt(shownAt[0] * window.devicePixelRatio, shownAt[1] * window.devicePixelRatio)
  }
}, { immediate: true })

let pictures: CanvasPictures<DeviceStream> | null = null
let stopSizing: (() => void) | null = null
let observer: ResizeObserver | null = null
/** The finger on the screen, while the viewer presses. */
let finger: number | null = null
let detachSink: (() => void) | null = null

function attach(): void {
  if (!canvas.value || pictures) {
    return
  }
  pictures = new CanvasPictures<DeviceStream>(canvas.value, {
    shown: (stream) => {
      picture.value = stream
      painted.value = true
    },
    lost: () => props.session.resync(),
  })
  // The canvas takes the generation's frames; its size waits for the first picture.
  const sink = {
    start: (stream: DeviceStream) => {
      picture.value = stream
      pictures?.start(stream)
    },
    show: (frame: Parameters<CanvasPictures<DeviceStream>['show']>[0]) => pictures?.show(frame),
    stop: () => pictures?.stop(),
  }
  props.session.attach(sink)
  detachSink = () => props.session.detach(sink)
}

function detach(): void {
  detachSink?.()
  detachSink = null
  pictures = null
}

/** The frame is drawn for the room the area has, in device pixels, once its size settled. */
function askFrame(): void {
  // An area not laid out yet has no room to draw a frame for.
  if (size.value.width === 0 || size.value.height === 0) {
    return
  }
  const room = frameRoom(size.value)
  const scale = window.devicePixelRatio
  props.session.frame(room.width * scale, room.height * scale, scale)
}

watch(watching, (now) => {
  if (now) {
    attach()
    askFrame()
  } else {
    detach()
  }
}, { flush: 'post' })

// A shown tab watches its device; a hidden one lets it go, so a device
// nobody looks at gets its hour.
watch(() => [props.shown, props.device] as const, ([shown, device], previous) => {
  if (previous && previous[1] !== device) {
    props.session.hiding(previous[1])
  }
  if (shown) {
    props.session.showing(device)
  } else {
    props.session.hiding(device)
  }
}, { immediate: true })

onMounted(() => {
  if (!area.value) {
    return
  }
  // The picture follows the area at once; the frame is asked for once its size settled.
  observer = new ResizeObserver(([entry]) => {
    if (entry) {
      size.value = { width: entry.contentRect.width, height: entry.contentRect.height }
    }
  })
  observer.observe(area.value)
  stopSizing = observeSettledSize(area.value, (settled) => {
    settledSize.value = settled
    if (watching.value) {
      askFrame()
    }
  })
  if (watching.value) {
    attach()
  }
})

onBeforeUnmount(() => {
  observer?.disconnect()
  stopSizing?.()
  props.session.hiding(props.device)
  detach()
})

function point(event: MouseEvent): { x: number; y: number } | null {
  const bounds = area.value?.getBoundingClientRect()
  if (!bounds || !screen.value || !placement.value) {
    return null
  }
  return devicePoint({ x: event.clientX - bounds.left, y: event.clientY - bounds.top }, screen.value, placement.value)
}

function down(event: PointerEvent): void {
  event.preventDefault()
  // The field follows the tap, so an input method's candidate window opens
  // where the viewer is typing. It takes no pointer events.
  const bounds = area.value?.getBoundingClientRect()
  if (keys.value && bounds) {
    keys.value.style.left = `${Math.max(0, Math.min(bounds.width - 2, event.clientX - bounds.left))}px`
    keys.value.style.top = `${Math.max(0, Math.min(bounds.height - 2, event.clientY - bounds.top))}px`
  }
  keys.value?.focus({ preventScroll: true })
  // A tap on a screen that is off wakes it, as on the device.
  if (screenOff.value) {
    props.session.wake()
    return
  }
  const at = point(event)
  if (!at || finger !== null) {
    return
  }
  finger = event.pointerId
  canvas.value?.setPointerCapture(event.pointerId)
  props.session.input(touchInput('start', [{ id: 0, ...at }]))
}

function move(event: PointerEvent): void {
  const at = point(event)
  if (finger === event.pointerId && at) {
    props.session.input(touchInput('move', [{ id: 0, ...at }]))
  }
}

function up(event: PointerEvent): void {
  const at = point(event)
  if (finger !== event.pointerId) {
    return
  }
  finger = null
  props.session.input(at ? touchInput('end', [{ id: 0, ...at }]) : touchInput('cancel', []))
}

function cancel(event: PointerEvent): void {
  if (finger === event.pointerId) {
    finger = null
    props.session.input(touchInput('cancel', []))
  }
}

/** A wheel turn scrolls the screen by as far as the picture moves under it, in the screen's points. */
function wheel(event: WheelEvent): void {
  event.preventDefault()
  const at = point(event)
  if (!at || !screen.value || !placement.value || screenOff.value || finger !== null) {
    return
  }
  const turn = wheelInput(at, event, placement.value.height)
  if (turn.type === 'wheel') {
    const scale = screen.value.width / placement.value.width
    props.session.input({ ...turn, deltaX: turn.deltaX * scale, deltaY: turn.deltaY * scale })
  }
}

function key(event: KeyboardEvent, action: 'down' | 'up'): void {
  if (event.isComposing) {
    return
  }
  event.preventDefault()
  // A key on a screen that is off wakes it, as on the device.
  if (screenOff.value) {
    if (action === 'down' && !event.repeat) {
      props.session.wake()
    }
    return
  }
  props.session.input(keyInput(action, event))
}

function inserted(event: Event): void {
  if (!(event instanceof InputEvent)) {
    return
  }
  const text = insertedText(event)
  if (text) {
    props.session.input({ type: 'text', text })
  }
  if (keys.value && !event.isComposing) {
    keys.value.value = ''
  }
}

function composed(event: CompositionEvent): void {
  if (event.data) {
    props.session.input({ type: 'text', text: event.data })
  }
  if (keys.value) {
    keys.value.value = ''
  }
}
</script>

<template>
  <div ref="area" class="relative min-h-0 flex-1 overflow-hidden bg-sunken">
    <LiveDrawnFrame
      v-if="drawnFrame && placement && painted"
      :screen="placement"
      :corners="drawnFrame.corners"
      :buttons="drawnFrame.buttons"
    />
    <canvas
      ref="canvas"
      class="absolute touch-none select-none"
      :class="painted ? '' : 'invisible'"
      :style="placement ? {
        left: `${placement.left}px`,
        top: `${placement.top}px`,
        width: `${placement.width}px`,
        height: `${placement.height}px`,
        clipPath: clip,
      } : undefined"
      @pointerdown="down"
      @pointermove="move"
      @pointerup="up"
      @pointercancel="cancel"
      @wheel="wheel"
      @contextmenu.prevent
    />
    <img
      v-if="layout?.image && painted"
      class="pointer-events-none absolute select-none"
      :src="layout.image.src"
      alt=""
      :style="{
        left: `${layout.image.box.left}px`,
        top: `${layout.image.box.top}px`,
        width: `${layout.image.box.width}px`,
        height: `${layout.image.box.height}px`,
      }"
    >
    <div
      v-if="screenOff && placement"
      class="absolute flex flex-col items-center justify-center gap-3 bg-black/60"
      :style="{
        left: `${placement.left}px`,
        top: `${placement.top}px`,
        width: `${placement.width}px`,
        height: `${placement.height}px`,
        clipPath: screenShape,
      }"
      @pointerdown="down"
    >
      <span class="text-[13px] font-medium text-white">{{ t('screenOff.title') }}</span>
      <Button size="sm" @pointerdown.stop @click="session.wake()">{{ t('screenOff.wake') }}</Button>
    </div>
    <RegionStatus
      v-if="session.state.pictureless"
      class="absolute inset-0 bg-surface"
      status="failed"
      :label="t('region.couldntShow')"
      :detail="t(`reasons.${session.state.pictureless}`)"
    />
    <RegionStatus
      v-else-if="!painted"
      class="absolute inset-0"
      status="loading"
      :label="t('region.connecting')"
    />
    <input
      ref="keys"
      class="pointer-events-none absolute left-0 top-0 h-[2px] w-[2px] border-0 bg-transparent p-0 text-transparent caret-transparent outline-none"
      :aria-label="t('keyboard')"
      autocomplete="off"
      autocorrect="off"
      spellcheck="false"
      @keydown="key($event, 'down')"
      @keyup="key($event, 'up')"
      @input="inserted"
      @compositionend="composed"
      @blur="session.release()"
    >
    <LiveActivityOverlay :active="activity.active" :label="t('activity')" :mark="mark" />
    <div v-if="stalled" class="pointer-events-none absolute inset-x-0 bottom-0 flex justify-center p-2">
      <span class="rounded-md bg-surface px-2 py-1 text-[12px] text-fg-subtle shadow">{{ t('region.waiting') }}</span>
    </div>
  </div>
</template>

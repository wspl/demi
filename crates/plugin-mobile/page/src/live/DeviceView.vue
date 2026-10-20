<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch, type CSSProperties } from 'vue'
import {
  Button,
  CanvasPictures,
  DRAWN_FRAME_ROOM,
  LiveActivityOverlay,
  LiveDrawnFrame,
  RegionStatus,
  SQUARE_CORNERS,
  keyInput,
  observeSettledSize,
  scaledCorners,
  drawnFramePictureClip,
  type ScreenCorners,
  touchInput,
  wheelInput,
} from '@demicodes/plugin-sdk'
import type { DeviceFrame, DeviceStream, MobileSession } from './session'
import { areaPoint, devicePoint, frameRoom, placeInFrame, placeScreen, type Placement, type Size } from './view'
import { insertedText } from './typed'
import { t } from '../locales/text'

/**
 * A simulator, live (`crates/plugin-mobile/page/src/index.ts`): its pictures on a
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
 * - As it unmounts, with its panel or its conversation, it hands its last picture to the session, and
 *   the next view of the device starts with it.
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
/** The generation of the picture the canvas shows, which places it and maps input on it. */
const picture = shallowRef<DeviceStream | null>(null)
/**
 * The generation announced before any picture is drawn, whose size the view
 * says at once; once a picture is drawn, layout, frame and turn follow the
 * picture drawn, not one only announced.
 */
const announced = shallowRef<DeviceStream | null>(null)
const painted = ref(false)
const watching = computed(() => props.shown && props.session.state.watched === props.device)
/** The device's last frame as the module drew it, kept with its last picture until a newer one comes. */
const framed = computed(() => props.session.state.frames[props.device] ?? null)
/** The screen in points: the drawn picture's, before the first one the announced one's or the frame's, so the view says its size at once. */
const screen = computed<Size | null>(() => {
  const stream = picture.value ?? announced.value
  if (stream) {
    return { width: stream.pointsWidth, height: stream.pointsHeight }
  }
  return framed.value ? { width: framed.value.pointsWidth, height: framed.value.pointsHeight } : null
})
/**
 * The last frame drawn for the shown picture's orientation, kept while the
 * frame of a turn comes before the turned pictures.
 */
const pictureFrame = shallowRef<DeviceFrame | null>(null)
watch([framed, picture], ([drawn, shown]) => {
  if (drawn && shown && drawn.orientation === shown.orientation) {
    pictureFrame.value = drawn
  }
}, { immediate: true })
/** The frame drawn for the picture's orientation; a frame of the orientation after a turn is not this picture's. */
const frame = computed(() => {
  const drawn = framed.value
  const orientation = picture.value?.orientation ?? drawn?.orientation
  if (drawn && drawn.orientation === orientation) {
    return drawn.frame
  }
  const kept = pictureFrame.value
  return kept && kept.device === props.device && kept.orientation === orientation ? kept.frame : null
})
/** A drawn frame's image, its box in the area, and the device's silhouette that bounds the picture under it, if any. */
type FrameImage = { src: string; box: Placement; silhouette: string | null }
type Layout = { screen: Placement; corners: ScreenCorners; image: FrameImage | null }

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
      corners: SQUARE_CORNERS,
      image: {
        src: `data:image/png;base64,${drawn.png}`,
        box: placed.frame,
        silhouette: drawn.silhouette ? `data:image/png;base64,${drawn.silhouette}` : null,
      },
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
 * What bounds the picture, and the screen-off dimming over it, under the
 * frame, whose screen is a hole the picture shows through, so the hole's
 * edge is the screen's only one: under a drawn frame's image, the device's
 * silhouette, as the screen's rectangle reaches past the device's outline
 * at its corners; under the frame the view draws, halfway into its bezel.
 * Android Studio's art covers the picture's corners itself, and a device
 * whose frame has not come shows its picture whole.
 */
const bounds = computed<CSSProperties | undefined>(() => {
  const placed = placement.value
  const shown = layout.value
  if (!placed || !shown) {
    return undefined
  }
  const silhouette = shown.image?.silhouette
  if (shown.image && silhouette) {
    const box = shown.image.box
    return {
      maskImage: `url(${silhouette})`,
      maskMode: 'luminance',
      maskRepeat: 'no-repeat',
      maskSize: `${box.width}px ${box.height}px`,
      maskPosition: `${box.left - placed.left}px ${box.top - placed.top}px`,
    }
  }
  const drawn = drawnFrame.value
  return drawn ? { clipPath: drawnFramePictureClip(placed.width, placed.height, drawn.corners) } : undefined
})
/**
 * The quarter turns, clockwise, from the picture drawn to the orientation
 * the device is shown in, the one it reports turned by the viewer's presses
 * it has not shown yet, and the scale that fits the turned device into the
 * area's room.
 */
const pending = computed(() => {
  const shown = picture.value
  const quarters = shown ? props.session.pendingQuarters(props.device, shown.orientation) : 0
  const placed = placement.value
  if (quarters === 0 || !placed || !layout.value) {
    return null
  }
  // The device's box: its drawn image, or the screen with the frame the view draws around it.
  const around = DRAWN_FRAME_ROOM * Math.min(placed.width, placed.height)
  const device = layout.value.image?.box ?? { width: placed.width + 2 * around, height: placed.height + 2 * around }
  const room = frameRoom(size.value)
  const scale = quarters % 2 === 0 ? 1 : Math.min(1, room.width / device.height, room.height / device.width)
  return { quarters, transform: `rotate(${quarters * 90}deg) scale(${scale})` }
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
      if (!picture.value) {
        announced.value = stream
      }
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

/** Draws the picture an earlier view of the device kept, until this view paints its own. */
function showKept(): void {
  const kept = props.session.keptPicture(props.device)
  const target = canvas.value
  const context = target?.getContext('2d', { alpha: false })
  if (!kept || !target || !context) {
    return
  }
  target.width = kept.image.width
  target.height = kept.image.height
  context.drawImage(kept.image, 0, 0)
  picture.value = kept.stream
  painted.value = true
}

/** A copy of the picture the canvas shows, which outlives the canvas, for the next view of the device. */
function keepPicture(): void {
  const source = canvas.value
  const stream = picture.value
  if (!source || !stream || !painted.value || source.width === 0 || source.height === 0) {
    return
  }
  const copy = document.createElement('canvas')
  copy.width = source.width
  copy.height = source.height
  copy.getContext('2d', { alpha: false })?.drawImage(source, 0, 0)
  props.session.keepPicture(props.device, { image: copy, stream })
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
  showKept()
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
  keepPicture()
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
    <!-- The device, turned at once for a rotation its pictures do not show yet. -->
    <div class="absolute inset-0" :style="pending ? { transform: pending.transform } : undefined">
      <canvas
        ref="canvas"
        class="absolute touch-none select-none"
        :class="painted ? '' : 'invisible'"
        :style="placement ? {
          left: `${placement.left}px`,
          top: `${placement.top}px`,
          width: `${placement.width}px`,
          height: `${placement.height}px`,
          ...bounds,
        } : undefined"
        @pointerdown="down"
        @pointermove="move"
        @pointerup="up"
        @pointercancel="cancel"
        @wheel="wheel"
        @contextmenu.prevent
      />
      <div
        v-if="screenOff && placement"
        class="absolute flex flex-col items-center justify-center gap-3 bg-black/60"
        :style="{
          left: `${placement.left}px`,
          top: `${placement.top}px`,
          width: `${placement.width}px`,
          height: `${placement.height}px`,
          ...bounds,
        }"
        @pointerdown="down"
      >
        <span class="text-[13px] font-medium text-white">{{ t('screenOff.title') }}</span>
        <Button size="sm" @pointerdown.stop @click="session.wake()">{{ t('screenOff.wake') }}</Button>
      </div>
      <!-- The frame lies over the picture, which shows through its screen. -->
      <LiveDrawnFrame
        v-if="drawnFrame && placement && painted"
        :screen="placement"
        :corners="drawnFrame.corners"
        :buttons="drawnFrame.buttons"
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

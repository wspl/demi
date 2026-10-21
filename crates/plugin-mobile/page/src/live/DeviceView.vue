<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch, watchEffect, type CSSProperties } from 'vue'
import {
  Button,
  IndeterminateSpinner,
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
import type { DeviceSession } from './session'
import type { LiveFrame } from '../generated/plugin'
import { areaPoint, devicePoint, frameRoom, placeInFrame, placeScreen, turnedBox, turnedFrame, type Placement, type Size } from './view'
import { insertedText } from './typed'
import { t } from '../locales/text'

/**
 * A simulator, live (`crates/plugin-mobile/page/src/index.ts`): its look, the
 * picture its session's canvas paints in the device's frame, fitted into the
 * tab. The view draws the session's look and nothing else, so the screen and
 * its frame show together or not at all
 * (`docs/decisions/0017-each-device-streams-on-its-own.md`).
 *
 * - A click or a finger on it is a touch, a drag a swipe, and the keyboard types.
 * - Over it show the agent's activity: the note that it is using the device and the
 *   ring where it acted.
 * - A screen that is off keeps its last picture, dimmed under Screen Off with Wake;
 *   a tap or a key wakes it too.
 * - The device's stream is looked at while the view shows in a visible page.
 *   The view stays mounted while its tab is hidden, and the canvas is the
 *   session's, so a return shows the device at once, and so does the next
 *   view of the device after this one unmounts.
 * - Until the first picture, it says what it waits for; a frame that could
 *   not be shown is a failure the viewer can retry.
 */
const props = defineProps<{
  session: DeviceSession
  shown: boolean
}>()

const area = ref<HTMLDivElement | null>(null)
const screenBox = ref<HTMLDivElement | null>(null)
const keys = ref<HTMLInputElement | null>(null)
const size = ref<Size>({ width: 0, height: 0 })
/** The area's size once a change of it has held, as after a drag of the panel's edge. */
const settledSize = ref<Size>({ width: 0, height: 0 })
const look = computed(() => props.session.state.look)
/** The screen in points, as the look shows it. */
const screen = computed<Size | null>(() => look.value ? { width: look.value.pointsWidth, height: look.value.pointsHeight } : null)
/** A drawn frame's image, its box in the area, and the device's silhouette that bounds the picture under it, if any. */
type FrameImage = { src: string; box: Placement; silhouette: string | null }
type Layout = { screen: Placement; corners: ScreenCorners; image: FrameImage | null }

/** Where a screen of `points` sits in an area of `area`, with its corners, and `drawn`, its frame, around it. */
function layoutIn(area: Size, drawn: LiveFrame, points: Size): Layout {
  if (drawn.kind === 'image') {
    const placed = placeInFrame(drawn, points, area)
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
  const placed = placeScreen(points, area)
  return { screen: placed, corners: scaledCorners(drawn.corners, placed.width / points.width), image: null }
}

/**
 * The device as it shows: its layout in the area, under the whole device's
 * quarter turns, clockwise, and the picture's within its screen.
 */
type Shown = { layout: Layout; frame: LiveFrame; device: number; picture: number }

/**
 * The device shows at once in the orientation the device reports, turned by
 * the viewer's presses it has not shown yet, laid out as its pictures in that
 * orientation will be, so their first paint leaves the device where it is
 * (`docs/decisions/0007-rotation-shows-at-once.md`). Until those pictures
 * paint, the look shows turned in the new screen, in that orientation's
 * frame; a device whose frame of that orientation is not ready turns whole,
 * its frame turned with it, laid out as that frame would be.
 */
const shown = computed<Shown | null>(() => {
  const current = look.value
  const points = screen.value
  if (!current || !points) {
    return null
  }
  const quarters = props.session.pendingQuarters(current.orientation)
  const orientation = props.session.shownOrientation()
  if (quarters === 0 || !orientation) {
    return { layout: layoutIn(size.value, current.frame, points), frame: current.frame, device: 0, picture: 0 }
  }
  const target = props.session.state.frames[orientation] ?? null
  if (target) {
    const turnedPoints = { width: target.pointsWidth, height: target.pointsHeight }
    return { layout: layoutIn(size.value, target.frame, turnedPoints), frame: target.frame, device: 0, picture: quarters }
  }
  const turnedPoints = quarters % 2 === 0 ? points : { width: points.height, height: points.width }
  const frame = current.frame
  const geometry = frame.kind === 'image' ? { ...frame, ...turnedFrame(frame, quarters) } : frame
  const final = layoutIn(size.value, geometry, turnedPoints)
  // Laid out turned, drawn in the frame's own orientation, which the device's turn turns back.
  const back = (box: Placement) => turnedBox(box, size.value, -quarters)
  return {
    layout: {
      screen: back(final.screen),
      corners: final.corners,
      image: final.image && { ...final.image, box: back(final.image.box) },
    },
    frame,
    device: quarters,
    picture: 0,
  }
})
const layout = computed(() => shown.value?.layout ?? null)
const placement = computed(() => layout.value?.screen ?? null)
/** The frame the view draws itself when none is drawn for the device. */
const drawnFrame = computed(() => {
  const drawn = shown.value?.frame
  if (drawn?.kind !== 'drawn' || !layout.value) {
    return null
  }
  return { corners: layout.value.corners, buttons: drawn.buttons }
})
/**
 * What bounds the picture, and the screen-off dimming over it, under the
 * frame, whose screen is a hole the picture shows through, so the hole's
 * edge is the screen's only one: under a drawn frame's image, the screen's
 * shape, as the screen's rectangle reaches past the device's outline at its
 * corners; under the frame the view draws, halfway into its bezel. Android
 * Studio's art covers the picture's corners itself.
 */
const bounds = computed<CSSProperties | undefined>(() => {
  const placed = placement.value
  const laid = layout.value
  if (!placed || !laid) {
    return undefined
  }
  const silhouette = laid.image?.silhouette
  if (laid.image && silhouette) {
    const box = laid.image.box
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
/** The whole device's turn, which turns a frame of the orientation drawn into the one shown. */
const deviceTurn = computed<CSSProperties | undefined>(() => shown.value?.device ? { transform: `rotate(${shown.value.device * 90}deg)` } : undefined)
/** The picture's box in its screen, turned into the orientation shown. */
const pictureBox = computed<Partial<CSSStyleDeclaration>>(() => {
  const placed = placement.value
  if (!placed || !shown.value || !look.value?.picture) {
    return { visibility: 'hidden' }
  }
  const quarters = shown.value.picture
  const [width, height] = quarters % 2 === 0 ? [placed.width, placed.height] : [placed.height, placed.width]
  return {
    visibility: '',
    left: `${(placed.width - width) / 2}px`,
    top: `${(placed.height - height) / 2}px`,
    width: `${width}px`,
    height: `${height}px`,
    transform: quarters === 0 ? '' : `rotate(${quarters * 90}deg)`,
  }
})
const activity = computed(() => props.shown ? props.session.state.activity : { active: false })
const mark = computed(() => {
  const point = activity.value.point
  return point && screen.value && placement.value ? areaPoint(point, screen.value, placement.value) : null
})
const stalled = computed(() => props.session.state.connection === 'stalled')
const painted = computed(() => !!look.value?.picture)
const screenOff = computed(() => painted.value && props.session.state.screenOff)
/** What the view waits for, in words, while it shows no picture. */
const waitingLabel = computed(() => {
  switch (props.session.waiting) {
    case 'connecting':
      return t('region.connecting')
    case 'frame':
      return t('region.frame')
    case 'screen':
      return t('region.screen')
    // A failure is no wait: the view says it and offers Retry.
    default:
      return null
  }
})

// The Host captures the screen no larger than the view shows it, in device
// pixels, once the area's size settled: a larger size restarts its capture,
// which a drag of the panel's edge would otherwise do at every step.
const settledScreen = computed(() => {
  const area = settledSize.value
  const current = look.value
  if (!props.shown || area.width === 0 || area.height === 0 || !current || !screen.value) {
    return null
  }
  return layoutIn(area, current.frame, screen.value).screen
})
watch(() => settledScreen.value ? [settledScreen.value.width, settledScreen.value.height] as const : null, (shownAt) => {
  if (shownAt) {
    props.session.showsScreenAt(shownAt[0] * window.devicePixelRatio, shownAt[1] * window.devicePixelRatio)
  }
}, { immediate: true })

let stopSizing: (() => void) | null = null
let observer: ResizeObserver | null = null
/** The finger on the screen, while the viewer presses. */
let finger: number | null = null
/** This view, as the session counts the views that show its device. */
const self = {}

// The session's canvas lies in this view's screen while it has one; its
// pointer goes to the device.
const canvasEvents: Array<[string, (event: never) => void]> = [
  ['pointerdown', down],
  ['pointermove', move],
  ['pointerup', up],
  ['pointercancel', cancel],
  ['wheel', wheel],
  ['contextmenu', (event: Event) => event.preventDefault()],
]
watch(screenBox, (box, previous) => {
  const canvas = props.session.canvas
  if (previous && canvas.parentElement === previous) {
    canvas.remove()
  }
  if (box) {
    canvas.className = 'absolute touch-none select-none'
    box.append(canvas)
  }
}, { flush: 'post' })
watchEffect(() => {
  Object.assign(props.session.canvas.style, pictureBox.value)
})

/** The frame is drawn for the room the area has, in device pixels. */
function askFrame(sized: Size): void {
  // An area not laid out, as in a hidden tab, has no room to draw a frame for.
  if (sized.width === 0 || sized.height === 0) {
    return
  }
  const room = frameRoom(sized)
  const scale = window.devicePixelRatio
  props.session.frame(room.width * scale, room.height * scale, scale)
}

watch(() => props.shown, (shown) => props.session.showIn(self, shown), { immediate: true })

onMounted(() => {
  const canvas = props.session.canvas
  for (const [name, listener] of canvasEvents) {
    canvas.addEventListener(name, listener as EventListener)
  }
  if (!area.value) {
    return
  }
  // The picture follows the area at once; the frame is asked for at the
  // first size, then once each change of it settled. A hidden tab's area,
  // which has no size, keeps the size it had.
  observer = new ResizeObserver(([entry]) => {
    if (!entry || entry.contentRect.width === 0 || entry.contentRect.height === 0) {
      return
    }
    const first = size.value.width === 0
    size.value = { width: entry.contentRect.width, height: entry.contentRect.height }
    if (first) {
      settledSize.value = size.value
      askFrame(size.value)
    }
  })
  observer.observe(area.value)
  stopSizing = observeSettledSize(area.value, (settled) => {
    if (settled.width === 0 || settled.height === 0) {
      return
    }
    settledSize.value = settled
    askFrame(settled)
  })
})

onBeforeUnmount(() => {
  observer?.disconnect()
  stopSizing?.()
  props.session.showIn(self, false)
  const canvas = props.session.canvas
  for (const [name, listener] of canvasEvents) {
    canvas.removeEventListener(name, listener as EventListener)
  }
  if (canvas.parentElement === screenBox.value) {
    canvas.remove()
  }
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
  props.session.canvas.setPointerCapture(event.pointerId)
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
    <!-- The device, whole, turned at once for a rotation its pictures do not show yet. -->
    <div v-if="layout" class="absolute inset-0" :style="deviceTurn">
      <!-- The screen, bounded under the frame, and the picture in it. -->
      <div
        ref="screenBox"
        class="absolute bg-black"
        :style="placement ? {
          left: `${placement.left}px`,
          top: `${placement.top}px`,
          width: `${placement.width}px`,
          height: `${placement.height}px`,
          ...bounds,
        } : undefined"
      />
      <!-- Before its first picture, the device's empty screen says what it waits for. -->
      <div
        v-if="waitingLabel && placement && !session.state.pictureless"
        role="status"
        class="pointer-events-none absolute flex flex-col items-center justify-center gap-3"
        :style="{
          left: `${placement.left}px`,
          top: `${placement.top}px`,
          width: `${placement.width}px`,
          height: `${placement.height}px`,
        }"
      >
        <IndeterminateSpinner class="text-white/80" />
        <span class="px-4 text-center text-[13px] text-white/80">{{ waitingLabel }}</span>
      </div>
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
        v-if="drawnFrame && placement"
        :screen="placement"
        :corners="drawnFrame.corners"
        :buttons="drawnFrame.buttons"
      />
      <img
        v-if="layout.image"
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
      v-else-if="session.state.frameFailed"
      class="absolute inset-0 bg-surface"
      status="failed"
      :label="t('region.couldntShow')"
      :detail="t('region.frameFailed')"
      :on-retry="() => session.retryFrame()"
    />
    <RegionStatus
      v-else-if="waitingLabel && !layout"
      class="absolute inset-0"
      status="loading"
      :label="waitingLabel"
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

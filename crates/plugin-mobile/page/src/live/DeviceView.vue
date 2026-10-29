<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch, watchEffect, type CSSProperties } from 'vue'
import {
  Button,
  IndeterminateSpinner,
  LiveActivityOverlay,
  LiveDrawnFrame,
  SQUARE_CORNERS,
  observeSettledSize,
  scaledCorners,
  drawnFramePictureClip,
  type ScreenCorners,
  touchInput,
  wheelInput,
  type HostDownload,
  prefersReducedMotion,
} from '@demicodes/plugin-sdk'
import { quartersBetween, type DeviceSession } from './session'
import type { LiveFrame } from '../generated/plugin'
import { RESTING, areaPoint, devicePoint, frameRoom, placeInFrame, placeScreen, poseAt, poseBetween, poseTransform, turnedBox, turnedFrame, unposed, type Placement, type Size, type TurnPose } from './view'
import DeviceKeys from './DeviceKeys.vue'
import DeviceStatus from './DeviceStatus.vue'
import { deviceStatus } from './status'
import { t } from '../locales/text'

/**
 * A simulator, live (`crates/plugin-mobile/page/src/index.ts`): its look, the
 * picture its session's canvas paints in the device's frame, fitted into the
 * tab. The view draws the session's look and nothing else, so the screen and
 * its frame show together or not at all.
 *
 * - A click or a finger on it is a touch, a drag a swipe, and the keyboard types
 *   (`./DeviceKeys.vue`).
 * - Over it show the agent's activity: the note that it is using the device and the
 *   ring where it acted.
 * - A screen that is off keeps its last picture, dimmed under Screen Off with Wake;
 *   a tap or a key wakes it too.
 * - The device's stream is looked at while the view shows in a visible page.
 *   The view stays mounted while its tab is hidden, and the canvas is the
 *   session's, so a return shows the device at once, and so does the next
 *   view of the device after this one unmounts.
 * - A device that changes the orientation it shows turns there, as a phone
 *   turned in the hand, so the moment Android takes to turn its windows
 *   shows as part of the turn.
 * - Until the first picture, it says what it waits for; a frame that could
 *   not be shown is a failure the viewer can retry. A stream that waits for
 *   the Host to download the plugin's program says how far the download came.
 */
const props = defineProps<{
  session: DeviceSession
  shown: boolean
  /** What the Host downloads of the plugin's packages now. */
  downloads: readonly HostDownload[]
}>()

const area = ref<HTMLDivElement | null>(null)
/** The device, whole, which a change of orientation turns. */
const turnBox = ref<HTMLDivElement | null>(null)
const screenBox = ref<HTMLDivElement | null>(null)
const keys = ref<InstanceType<typeof DeviceKeys> | null>(null)
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
 * orientation will be, so their first paint leaves the device where it is.
 * Until those pictures
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

/** How long a turn takes, as a phone turned in the hand, and the iOS Simulator's window. */
const TURN_MS = 300
/** The steps a turn's animation is drawn at, each one's pose computed here, so the device stays inside the area at every angle. */
const TURN_STEPS = 18
/** The turn under way: its pose at its start relative to the layout it turns into, and when it started. */
let turn: { start: TurnPose; began: number; duration: number; screen: Placement; device: Size; animation: Animation } | null = null

/** Where the turn under way shows the device now; none once it ended. */
function currentPose(): TurnPose | null {
  if (!turn) {
    return null
  }
  const progress = (performance.now() - turn.began) / turn.duration
  return progress >= 1 ? null : poseAt(turn.start, progress, turn.device, size.value)
}

/**
 * Turns the device from where it shows to where its layout puts it once the
 * orientation it shows changes, about its screen's center; a layout that
 * changes during a turn, as when the new orientation's frame arrives, or
 * another turn, continues from where the device shows.
 */
watch(
  () => {
    // Where the screen shows, a device turned whole included.
    const placed = placement.value
    const device = shown.value?.device ?? 0
    return { orientation: props.session.shownOrientation(), screen: placed && device ? turnedBox(placed, size.value, device) : placed }
  },
  (now, before) => {
    const box = turnBox.value
    const pose = currentPose()
    if (!box || !props.shown || !now.orientation || !before.orientation || !now.screen || !before.screen || prefersReducedMotion()) {
      return
    }
    const quarters = quartersBetween(before.orientation, now.orientation)
    if (quarters === 0 && !pose) {
      return
    }
    const start = poseBetween(before.screen, pose ?? RESTING, now.screen, quarters)
    // A new turn takes its whole time; a layout that changed during one, the rest of it.
    const duration = quarters !== 0 || !turn ? TURN_MS : Math.max(1, turn.duration - (performance.now() - turn.began))
    const frameBox = layout.value?.image?.box ?? now.screen
    const device = { width: frameBox.width, height: frameBox.height }
    const screen = now.screen
    const keyframes = Array.from({ length: TURN_STEPS + 1 }, (_, step) => ({
      transform: poseTransform(poseAt(start, step / TURN_STEPS, device, size.value), screen),
    }))
    turn?.animation.cancel()
    const animation = box.animate(keyframes, { duration, easing: 'linear' })
    turn = { start, began: performance.now(), duration, screen, device, animation }
    animation.onfinish = () => {
      if (turn?.animation === animation) {
        turn = null
      }
    }
  },
  { flush: 'post' },
)
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
const painted = computed(() => !!look.value?.picture)
const screenOff = computed(() => painted.value && props.session.state.screenOff)
const { progress, progressText, waitingLabel } = deviceStatus(props)

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
  // During a turn, the point of the layout the turning device shows there.
  const inArea = { x: event.clientX - bounds.left, y: event.clientY - bounds.top }
  const pose = currentPose()
  const at = pose && turn ? unposed(inArea, pose, turn.screen) : inArea
  return devicePoint(at, screen.value, placement.value)
}

function down(event: PointerEvent): void {
  event.preventDefault()
  const bounds = area.value?.getBoundingClientRect()
  if (bounds) {
    keys.value?.focusAt(event.clientX - bounds.left, event.clientY - bounds.top, bounds.width, bounds.height)
  }
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

</script>

<template>
  <div ref="area" class="relative min-h-0 flex-1 overflow-hidden bg-sunken">
    <!-- The device, whole, turning when its orientation changes. -->
    <div ref="turnBox" class="absolute inset-0 origin-top-left">
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
        <span class="text-[12px] tabular-nums text-white/60" :class="{ invisible: !progress }">{{ progressText }}</span>
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
    </div>
    <DeviceKeys ref="keys" :session="session" :screen-off="screenOff" />
    <LiveActivityOverlay :active="activity.active" :label="t('activity')" :mark="mark" />
    <DeviceStatus :session="session" :downloads="downloads" :wait-on-screen="!!layout" />
  </div>
</template>

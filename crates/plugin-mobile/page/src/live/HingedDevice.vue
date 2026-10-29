<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { Button, LiveActivityOverlay, observeSettledSize, touchInput, wheelInput, type HostDownload } from '@demicodes/plugin-sdk'
import { quartersBetween, type DeviceSession } from './session'
import type { LiveFold } from '../generated/plugin'
import DeviceKeys from './DeviceKeys.vue'
import DeviceStatus from './DeviceStatus.vue'
import FoldBar from './FoldBar.vue'
import { FoldScene, MAX_YAW } from './fold-scene'
import { appleBody } from './apple-body'
import { t } from '../locales/text'

/**
 * A hinged foldable, live, as iPhone Duo (`./fold-scene.ts`): the device in
 * 3D with each display's own pictures on its face (`./display-pictures.ts`),
 * and the capsule that turns its hinge. The device's own stream carries its
 * state and the input, and no pictures.
 *
 * - On the lit display a click is a touch and a drag a swipe, wherever the
 *   view is turned and however the device is folded; the keyboard types
 *   (`./DeviceKeys.vue`).
 * - While the device is partly open, a drag anywhere else turns the device
 *   about its upright axis the way the pointer moves, as Quick Look turns a
 *   model, at most `MAX_YAW` either way; Reset turns it back to face the
 *   user over `FACE_MS`. Closed or open flat, a flat device faces the user
 *   and does not turn: the turn eases out over the last `FLAT_EASE` degrees
 *   of the hinge, so the view never jumps.
 * - A posture, from the toolbar, and the slider, as it is dragged, turn the
 *   hinge towards their angle at once, from where it is, as Device Hub
 *   does; the model follows the angle on Device Hub's spring
 *   (`MODEL_SMOOTHING`), and then the angle the device reports.
 * - A device held sideways shows turned, as a 2D frame turns.
 */
const props = defineProps<{
  session: DeviceSession
  fold: Extract<LiveFold, { kind: 'hinge' }>
  shown: boolean
  /** Where the page reads the file of the device's 3D body; none without one. */
  bodyUrl: string | null
  /** What the Host downloads of the plugin's packages now. */
  downloads: readonly HostDownload[]
}>()

/** The smoothing time of the spring the model follows the hinge's angle on, Device Hub's: about the time it takes to cover most of the way. */
const MODEL_SMOOTHING = 0.25
/** How long after the view's last move of the hinge the device's reports of the angle take over again: longer than their way back from the Host. */
const ECHO_MS = 1000
/** How near its angle, in degrees, the model is when it gets there. */
const MODEL_ARRIVED = 0.05
/** How long Reset takes to turn the view back to face the device. */
const FACE_MS = 400
/** Degrees the view turns per CSS pixel of a drag. */
const TURN_PER_PX = 0.4
/** The hinge's degrees before flat over which the view's turn eases out. */
const FLAT_EASE = 20

const area = ref<HTMLDivElement | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
const keys = ref<InstanceType<typeof DeviceKeys> | null>(null)
const scene = shallowRef<FoldScene | null>(null)
/** Each display's pictures, on their own streams. */
const pictures = { outer: props.session.displayPictures('outer'), inner: props.session.displayPictures('inner') }
/** The lit display's pictures, which the pointer and the view's status are about. */
const lit = computed(() => pictures[props.fold.display])
/** The angle the model shows now, on its way to `target`. */
const degrees = ref(props.fold.degrees)
/** The angle the model moves to: the one a posture or the slider asked last, else the device's. */
let target = props.fold.degrees
/** When the view last turned the hinge itself, whose angle the device's reports catch up with after `ECHO_MS`. */
let asked = Number.NEGATIVE_INFINITY
let velocity = 0
let last = performance.now()
const yaw = ref(0)
const look = computed(() => lit.value.state.look?.picture ?? null)
const screenOff = computed(() => !!look.value && props.session.state.screenOff)
/** Quarter turns clockwise the device is held from upright, as its pictures show it. */
const quarters = computed(() => look.value ? quartersBetween('portrait', look.value.orientation) : 0)
const activity = computed(() => props.shown ? props.session.state.activity : { active: false })

/** The view's turn back to face the device, while Reset turns it. */
let facing: { from: number; started: number } | null = null

/** `progress` from 0 to 1 eased out, as the view's turn back moves. */
function easedOut(progress: number): number {
  return 1 - Math.pow(1 - progress, 3)
}

/**
 * One step of a critically damped spring of `smoothing` seconds from
 * `from`, moving at `speed`, towards `to`, `elapsed` seconds on, never past
 * it, as Device Hub steps its model.
 */
function spring(from: number, speed: number, to: number, elapsed: number, smoothing: number): [number, number] {
  const omega = 2 / smoothing
  const x = omega * elapsed
  const decay = 1 / (1 + x + 0.48 * x * x + 0.235 * x * x * x)
  const offset = from - to
  const pull = (speed + omega * offset) * elapsed
  const next = to + (offset + pull) * decay
  if ((to - from > 0) === (next > to)) {
    return [to, 0]
  }
  return [next, (speed - omega * pull) * decay]
}

function face(): void {
  facing = { from: yaw.value, started: performance.now() }
}

watch(() => props.fold.display, (display) => scene.value?.light(display))
watch(quarters, (turn) => scene.value?.setTurn(turn))

for (const display of ['outer', 'inner'] as const) {
  watch(() => !!pictures[display].state.look, (painted) => scene.value?.painted(display, painted))
}

/** Turns the hinge towards `to`, as a posture or the slider does: the model goes there on its spring; a move that cannot go leaves it where the device is. */
function hinge(to: number): void {
  if (props.session.hinge(to)) {
    target = to
    asked = performance.now()
  }
}

let frame = 0
function draw(): void {
  frame = requestAnimationFrame(draw)
  const drawing = scene.value
  if (!drawing) {
    return
  }
  const now = performance.now()
  // A frame late by more than 50 ms moves as one of 50 ms, as Device Hub's model does.
  const elapsed = Math.min(Math.max((now - last) / 1000, 0), 0.05)
  last = now
  // Once the view's own moves stopped, the model goes where the device
  // says, so a report of an earlier move never turns it back meanwhile.
  if (now - asked > ECHO_MS) {
    target = props.fold.degrees
  }
  if (degrees.value !== target) {
    ;[degrees.value, velocity] = spring(degrees.value, velocity, target, elapsed, MODEL_SMOOTHING)
    if (Math.abs(degrees.value - target) < MODEL_ARRIVED) {
      degrees.value = target
      velocity = 0
    }
  }
  if (facing) {
    const progress = Math.min(1, (performance.now() - facing.started) / FACE_MS)
    yaw.value = facing.from * (1 - easedOut(progress))
    if (progress >= 1) {
      facing = null
    }
  }
  drawing.setDegrees(degrees.value)
  const fromFlat = Math.min(degrees.value, 180 - degrees.value)
  if (fromFlat <= 0 && !turning) {
    yaw.value = 0
  }
  drawing.setYaw(yaw.value * Math.min(1, fromFlat / FLAT_EASE))
  drawing.render()
}

let stopSizing: (() => void) | null = null
let observer: ResizeObserver | null = null
const self = {}
watch(() => props.shown, (shown) => {
  props.session.showIn(self, shown)
  pictures.outer.showIn(self, shown)
  pictures.inner.showIn(self, shown)
}, { immediate: true })

onMounted(() => {
  if (!canvas.value || !area.value) {
    return
  }
  // A 2D view shown before the device said it folds asked for the device's
  // own pictures, which this view never shows.
  props.session.unshow()
  const drawing = new FoldScene(
    canvas.value,
    { outer: pictures.outer.canvas, inner: pictures.inner.canvas },
    props.fold.outer,
    props.fold.inner,
    !!props.bodyUrl,
  )
  drawing.light(props.fold.display)
  drawing.painted('outer', !!pictures.outer.state.look)
  drawing.painted('inner', !!pictures.inner.state.look)
  drawing.setTurn(quarters.value)
  scene.value = drawing
  // Apple's body takes the stand-in's place once it loaded and fits.
  if (props.bodyUrl) {
    const url = props.bodyUrl
    void (async () => {
      const body = await appleBody(url, props.fold.outer, props.fold.inner)
      if (!body) {
        // The file does not fit: the stand-in shows the device.
        drawing.useStandIn()
        return
      }
      // A view that went meanwhile leaves the body unused.
      if (scene.value !== drawing || !(await drawing.useBody(body))) {
        body.dispose()
      }
    })().catch((error: unknown) => {
      // The stand-in shows the device: the file could not be read, or not as a body.
      console.warn('iPhone Duo\'s body did not load', error)
      drawing.useStandIn()
    })
  }
  // Each display's pictures come no larger than the view shows the device.
  const ask = (width: number, height: number) => {
    const ratio = window.devicePixelRatio
    pictures.outer.showsAt(width * ratio, height * ratio)
    pictures.inner.showsAt(width * ratio, height * ratio)
  }
  let asked = false
  observer = new ResizeObserver(([entry]) => {
    if (!entry || entry.contentRect.width === 0 || entry.contentRect.height === 0) {
      return
    }
    scene.value?.resize(entry.contentRect.width, entry.contentRect.height, window.devicePixelRatio)
    if (!asked) {
      asked = true
      ask(entry.contentRect.width, entry.contentRect.height)
    }
  })
  observer.observe(area.value)
  stopSizing = observeSettledSize(area.value, (settled) => {
    if (settled.width > 0 && settled.height > 0) {
      ask(settled.width, settled.height)
    }
  })
  frame = requestAnimationFrame(draw)
})

onBeforeUnmount(() => {
  cancelAnimationFrame(frame)
  observer?.disconnect()
  stopSizing?.()
  props.session.showIn(self, false)
  pictures.outer.showIn(self, false)
  pictures.inner.showIn(self, false)
  scene.value?.dispose()
})

/** The lit display's point under the pointer, in the points the device takes; none off it. */
function displayPoint(event: MouseEvent): { x: number; y: number } | null {
  const bounds = canvas.value?.getBoundingClientRect()
  const drawing = scene.value
  const current = look.value
  if (!bounds || !drawing || !current) {
    return null
  }
  const at = drawing.displayPoint(event.clientX - bounds.left, event.clientY - bounds.top)
  return at && { x: at.x * current.pointsWidth, y: at.y * current.pointsHeight }
}

/** How far a drag from `from` to `event` went across the device as it is held, in CSS pixels. */
function across(from: { x: number; y: number }, event: PointerEvent): number {
  const [dx, dy] = [event.clientX - from.x, event.clientY - from.y]
  const turn = ((quarters.value % 4) + 4) % 4
  return [dx, dy, -dx, -dy][turn]!
}

/** The pointer that touches the device, or that turns the view, while it is down. */
let finger: number | null = null
let turning: { pointer: number; x: number; y: number; yaw: number } | null = null
const cursor = ref('default')

function down(event: PointerEvent): void {
  event.preventDefault()
  const bounds = area.value?.getBoundingClientRect()
  const at = displayPoint(event)
  if (at && bounds) {
    keys.value?.focusAt(event.clientX - bounds.left, event.clientY - bounds.top, bounds.width, bounds.height)
    if (screenOff.value) {
      props.session.wake()
      return
    }
    if (finger === null) {
      finger = event.pointerId
      canvas.value?.setPointerCapture(event.pointerId)
      props.session.input(touchInput('start', [{ id: 0, ...at }]))
    }
    return
  }
  if (!turnable()) {
    return
  }
  // A drag takes the view from where Reset left it.
  facing = null
  turning = { pointer: event.pointerId, x: event.clientX, y: event.clientY, yaw: yaw.value }
  canvas.value?.setPointerCapture(event.pointerId)
  cursor.value = 'grabbing'
}

/** Whether the view turns: only while the device is partly open. */
function turnable(): boolean {
  const angle = Math.round(degrees.value)
  return angle > 0 && angle < 180
}

function move(event: PointerEvent): void {
  if (turning?.pointer === event.pointerId) {
    yaw.value = Math.max(-MAX_YAW, Math.min(MAX_YAW, turning.yaw - across(turning, event) * TURN_PER_PX))
    return
  }
  const at = displayPoint(event)
  if (finger === event.pointerId) {
    if (at) {
      props.session.input(touchInput('move', [{ id: 0, ...at }]))
    }
    return
  }
  cursor.value = at || !turnable() ? 'default' : 'grab'
}

function up(event: PointerEvent): void {
  if (turning?.pointer === event.pointerId) {
    turning = null
    cursor.value = turnable() ? 'grab' : 'default'
    return
  }
  if (finger !== event.pointerId) {
    return
  }
  finger = null
  const at = displayPoint(event)
  props.session.input(at ? touchInput('end', [{ id: 0, ...at }]) : touchInput('cancel', []))
}

function cancel(event: PointerEvent): void {
  if (turning?.pointer === event.pointerId) {
    turning = null
  }
  if (finger === event.pointerId) {
    finger = null
    props.session.input(touchInput('cancel', []))
  }
}

function wheel(event: WheelEvent): void {
  event.preventDefault()
  const at = displayPoint(event)
  const current = look.value
  if (!at || !current || screenOff.value || finger !== null) {
    return
  }
  const turn = wheelInput(at, event, current.pointsHeight)
  if (turn.type === 'wheel') {
    props.session.input(turn)
  }
}

/** The toolbar's postures turn the hinge as the slider does. */
defineExpose({ posture: hinge })

</script>

<template>
  <div class="flex min-h-0 flex-1 flex-col bg-sunken">
    <div ref="area" class="relative min-h-0 flex-1 overflow-hidden">
      <canvas
        ref="canvas"
        class="absolute inset-0 h-full w-full touch-none select-none"
        :style="{ cursor }"
        @pointerdown="down"
        @pointermove="move"
        @pointerup="up"
        @pointercancel="cancel"
        @wheel="wheel"
        @contextmenu.prevent
      />
      <div v-if="screenOff" class="absolute inset-x-0 bottom-3 flex justify-center">
        <span class="flex items-center gap-2 rounded-md bg-surface px-2 py-1 text-[12px] text-fg-subtle shadow">
          {{ t('screenOff.title') }}
          <Button size="sm" @click="session.wake()">{{ t('screenOff.wake') }}</Button>
        </span>
      </div>
      <DeviceKeys ref="keys" :session="session" :screen-off="screenOff" />
      <LiveActivityOverlay :active="activity.active" :label="t('activity')" :mark="null" />
      <DeviceStatus :session="lit" :downloads="downloads" :wait-on-screen="false" />
    </div>
    <FoldBar :degrees="degrees" :turned="Math.abs(yaw) > 0.5" @hinge="hinge" @reset="face" />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { Button, LiveActivityOverlay, observeSettledSize, touchInput, wheelInput, type HostDownload } from '@demicodes/plugin-sdk'
import { quartersBetween, type DeviceSession } from './session'
import type { LiveFold } from '../generated/plugin'
import DeviceKeys from './DeviceKeys.vue'
import DeviceStatus from './DeviceStatus.vue'
import FoldBar from './FoldBar.vue'
import { FoldScene, MAX_YAW } from './fold-scene'
import { appleBody, bodyBytes, sha256 } from './apple-body'
import { t } from '../locales/text'

/**
 * A hinged foldable, live, as iPhone Duo (`./fold-scene.ts`): the device in
 * 3D with the picture on its lit display, and the capsule that turns its hinge.
 *
 * - On the lit display a click is a touch and a drag a swipe, wherever the
 *   view is turned and however the device is folded; the keyboard types
 *   (`./DeviceKeys.vue`).
 * - While the device is partly open, a drag anywhere else turns the view
 *   about the device's upright axis, at most `MAX_YAW` either way; Reset
 *   turns it back to face the device over `FACE_MS`. Closed or open flat,
 *   a flat device faces the user and does not turn: the turn eases out over
 *   the last `FLAT_EASE` degrees of the hinge, so the view never jumps.
 * - A posture, from the toolbar, moves the hinge over `SWEEP_MS`, the
 *   slider as it is dragged; the device follows, as `demi mobile fold`
 *   moves it, one fold at a time, to the latest angle asked. A fold the
 *   device refused leaves the view at the angle the device reports.
 * - A device held sideways shows turned, as a 2D frame turns.
 */
const props = defineProps<{
  session: DeviceSession
  fold: Extract<LiveFold, { kind: 'hinge' }>
  shown: boolean
  /** Moves the device's hinge, as `demi mobile fold` does: the angle it got to, or none when it refused. */
  hinge: (degrees: number) => Promise<number | null>
  /** Where the page reads the file of the device's 3D body; none without one. */
  bodyUrl: string | null
  /** What the Host downloads of the plugin's packages now. */
  downloads: readonly HostDownload[]
}>()

/** How long a posture takes to move the hinge, as Xcode's Device Hub moves it. */
const SWEEP_MS = 600
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
/** The hinge's angle the view shows: the device's, or the one a posture or the slider moves it to. */
const degrees = ref(props.fold.degrees)
const yaw = ref(0)
const look = computed(() => props.session.state.look)
const screenOff = computed(() => !!look.value?.picture && props.session.state.screenOff)
/** Quarter turns clockwise the device is held from upright, as its picture shows. */
const quarters = computed(() => look.value ? quartersBetween('portrait', look.value.orientation) : 0)
const activity = computed(() => props.shown ? props.session.state.activity : { active: false })

/** The angle the view moves the hinge to, while it moves it: the device's own reports wait until it got there. */
let moving: number | null = null
let sweep: { from: number; to: number; started: number } | null = null
/** The view's turn back to face the device, while Reset turns it. */
let facing: { from: number; started: number } | null = null

/** `progress` from 0 to 1 eased out, as the hinge's and the view's turns move. */
function easedOut(progress: number): number {
  return 1 - Math.pow(1 - progress, 3)
}

function face(): void {
  facing = { from: yaw.value, started: performance.now() }
}

watch(() => props.fold.display, (display) => scene.value?.light(display))
watch(quarters, (turn) => scene.value?.setTurn(turn))

watch(() => props.fold.degrees, (reported) => {
  if (moving === null) {
    startSweep(reported)
  }
})

function startSweep(to: number): void {
  sweep = { from: degrees.value, to, started: performance.now() }
}

function posture(to: number): void {
  startSweep(to)
  fold(to)
}

function slide(to: number): void {
  sweep = null
  degrees.value = to
  fold(to)
}

/** Whether a fold is under way, which the next waits for. */
let folding = false
/**
 * Moves the device's hinge to `to`: at once, or once the fold under way
 * ended, the latest angle asked meanwhile replacing any before it. A fold
 * the device refused moves the view back to the angle it reports.
 */
function fold(to: number): void {
  moving = to
  if (folding) {
    return
  }
  folding = true
  void props.hinge(to).then((reached) => {
    folding = false
    const latest = moving
    if (latest !== null && latest !== to) {
      fold(latest)
      return
    }
    moving = null
    if (reached === null) {
      startSweep(props.fold.degrees)
    }
  })
}

let frame = 0
function draw(): void {
  frame = requestAnimationFrame(draw)
  const drawing = scene.value
  if (!drawing) {
    return
  }
  if (sweep) {
    const progress = Math.min(1, (performance.now() - sweep.started) / SWEEP_MS)
    degrees.value = sweep.from + (sweep.to - sweep.from) * easedOut(progress)
    if (progress >= 1) {
      sweep = null
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
watch(() => props.shown, (shown) => props.session.showIn(self, shown), { immediate: true })

onMounted(() => {
  if (!canvas.value || !area.value) {
    return
  }
  const drawing = new FoldScene(canvas.value, props.session.canvas, props.fold.outer, props.fold.inner)
  drawing.light(props.fold.display)
  drawing.setTurn(quarters.value)
  scene.value = drawing
  // Apple's body takes the stand-in's place once it loaded and fits.
  if (props.bodyUrl) {
    const url = props.bodyUrl
    void (async () => {
      const bytes = await bodyBytes(url)
      const body = await appleBody(bytes, await sha256(bytes), props.fold.outer, props.fold.inner)
      if (!body) {
        return
      }
      if (scene.value === drawing) {
        drawing.useBody(body)
      } else {
        body.dispose()
      }
    })().catch((error: unknown) => {
      // The stand-in stays: the file could not be read, or not as a body.
      console.warn('iPhone Duo\'s body did not load', error)
    })
  }
  // The pictures come at the size the view shows the lit display at, and wait for its frame.
  const ask = (width: number, height: number) => {
    const ratio = window.devicePixelRatio
    props.session.frame(width * ratio, height * ratio, ratio)
    props.session.showsScreenAt(width * ratio, height * ratio)
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
    yaw.value = Math.max(-MAX_YAW, Math.min(MAX_YAW, turning.yaw + across(turning, event) * TURN_PER_PX))
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

/** The toolbar's postures move the hinge as the slider's view does, over `SWEEP_MS`. */
defineExpose({ posture })

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
      <DeviceStatus :session="session" :downloads="downloads" :wait-on-screen="false" />
    </div>
    <FoldBar :degrees="degrees" :turned="Math.abs(yaw) > 0.5" @hinge="slide" @reset="face" />
  </div>
</template>

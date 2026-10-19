<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import {
  CanvasPictures,
  LiveActivityOverlay,
  LiveDeviceOutline,
  RegionStatus,
  keyInput,
  observeSettledSize,
  touchInput,
} from '@demicodes/plugin-sdk'
import type { DeviceStream, MobileSession } from './session'
import { areaPoint, devicePoint, frameRoom, placeInFrame, placeScreen, type Placement, type Size } from './view'
import { insertedText } from './typed'
import { t } from '../locales/text'

/**
 * A simulator, live (`mobile.md` § The live view): its pictures on a
 * canvas, in the device's frame, fitted into the tab; a click or a finger
 * on it is a touch, a drag a swipe, the keyboard types; and over it the
 * agent's activity, the note that it is using the device and the ring
 * where it acted. The view stays mounted while its tab is hidden, so its
 * last picture shows at once when the tab is shown again; the session
 * watches the device only while its tab is shown.
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
/** The generation the canvas shows, which places it and maps input on it. */
const picture = shallowRef<DeviceStream | null>(null)
const painted = ref(false)
const watching = computed(() => props.shown && props.session.state.watched === props.device)
const screen = computed<Size | null>(() => picture.value
  ? { width: picture.value.pointsWidth, height: picture.value.pointsHeight }
  : null)
/** The frame drawn for the picture's orientation; a frame of the orientation before a turn is not this picture's. */
const frame = computed(() => {
  const framed = props.session.state.frame
  return watching.value && framed && framed.device === props.device && framed.orientation === picture.value?.orientation
    ? framed.frame
    : null
})
/** Where the picture sits, with its corners, and the drawn frame around it, if any. */
const layout = computed<{ screen: Placement; cornerRadius: number; image: { src: string; box: Placement } | null } | null>(() => {
  if (!screen.value) {
    return null
  }
  const drawn = frame.value
  if (drawn?.kind === 'image') {
    const placed = placeInFrame(drawn, screen.value, size.value)
    return {
      screen: placed.screen,
      cornerRadius: placed.cornerRadius,
      image: { src: `data:image/png;base64,${drawn.png}`, box: placed.frame },
    }
  }
  const placed = placeScreen(screen.value, size.value)
  const cornerRadius = drawn?.kind === 'outline' ? drawn.cornerRadius * placed.width / screen.value.width : 0
  return { screen: placed, cornerRadius, image: null }
})
const placement = computed(() => layout.value?.screen ?? null)
/** The outline the view draws when no frame is drawn for the device. */
const outline = computed(() => {
  const drawn = frame.value
  if (drawn?.kind !== 'outline' || !placement.value || !screen.value) {
    return null
  }
  return { cornerRadius: layout.value?.cornerRadius ?? 0, buttons: drawn.buttons }
})
const activity = computed(() => watching.value ? props.session.state.activity : { active: false })
const mark = computed(() => {
  const point = activity.value.point
  return point && screen.value && placement.value ? areaPoint(point, screen.value, placement.value) : null
})
const stalled = computed(() => props.session.state.connection === 'stalled')

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
  stopSizing = observeSettledSize(area.value, () => {
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

function point(event: PointerEvent): { x: number; y: number } | null {
  const bounds = area.value?.getBoundingClientRect()
  if (!bounds || !screen.value || !placement.value) {
    return null
  }
  return devicePoint({ x: event.clientX - bounds.left, y: event.clientY - bounds.top }, screen.value, placement.value)
}

function down(event: PointerEvent): void {
  event.preventDefault()
  keys.value?.focus({ preventScroll: true })
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

function key(event: KeyboardEvent, action: 'down' | 'up'): void {
  if (event.isComposing) {
    return
  }
  event.preventDefault()
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
    <LiveDeviceOutline
      v-if="outline && placement && painted"
      :screen="placement"
      :corner-radius="outline.cornerRadius"
      :buttons="outline.buttons"
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
        borderRadius: layout?.cornerRadius ? `${layout.cornerRadius}px` : undefined,
      } : undefined"
      @pointerdown="down"
      @pointermove="move"
      @pointerup="up"
      @pointercancel="cancel"
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

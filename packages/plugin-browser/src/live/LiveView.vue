<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import type { LiveTab } from '../generated/plugin'
import LiveControls from './LiveControls.vue'
import LiveDialog from './LiveDialog.vue'
import { RegionStatus } from '@demicodes/plugin-sdk'
import { refusalSentence } from './tabs'
import { CanvasPictures } from './pictures'
import type { LiveSession, LiveStream } from './session'
import { viewerClipboard } from './clipboard'
import { composingKey } from '@demicodes/utils'
import { ClickCount, keyMessage, localKey, pointerMessage, wheelMessage } from './input'
import { cursorAt, placePicture, tabPoint, type PanelSize } from './view'

/**
 * A tab of the conversation's browser, live (`live-view.md`): its
 * pictures on a canvas, the viewer's input on its way to the page, and the
 * page's own native controls and dialogs over it. The view stays mounted
 * while its content is hidden and while no session is open, so its last
 * picture shows at once when the tab is shown again; the page's panel
 * session alone decides which tab a session watches.
 */
const props = defineProps<{
  /** The page's view, while one is open. */
  session: LiveSession | null
  /** The tab as a view last reported it. */
  tab: LiveTab
  /** Whether the content is shown: only a shown view takes the session's pictures. */
  shown: boolean
  /** The panel's size, as its content measured it. */
  panel: PanelSize
  /** What moves the picture onto the screen's pixel grid. */
  snap: { x: number; y: number }
}>()

const frame = ref<HTMLDivElement | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
/** The viewer's keys, input method and clipboard go through this field. */
const bridge = ref<HTMLInputElement | null>(null)
/**
 * The generation of the picture the canvas shows: it places the picture and
 * maps input on it, never a viewport the tab list reports for a picture that
 * has not arrived (`live-view.md` § Modes). Before the first, the tab's.
 */
const picture = shallowRef<LiveStream | null>(null)
const generation = computed<LiveStream>(() => picture.value ?? {
  tab: props.tab.id,
  generation: 0,
  width: props.tab.viewport.width * props.tab.viewport.devicePixelRatio,
  height: props.tab.viewport.height * props.tab.viewport.devicePixelRatio,
  viewport: props.tab.viewport,
  scale: 1,
})
const placement = computed(() => {
  const placed = placePicture(generation.value, props.panel)
  return { ...placed, left: placed.left + props.snap.x, top: placed.top + props.snap.y }
})
const state = computed(() => props.session?.state ?? null)
/** Nothing arrives, or the view is connecting again after it ended: the last picture stays under a quiet note. */
const stalled = computed(() => {
  const connection = state.value?.connection
  return connection === 'stalled' || (connection === 'opening' && state.value?.ended !== null)
})
/** The pointer in the tab, while it is over the picture. */
const pointer = shallowRef<{ x: number; y: number } | null>(null)
/** The cursor the page shows under the pointer, resolved here from the page's cursor regions. */
const cursor = computed(() => {
  const at = pointer.value
  if (!at || !state.value) {
    return 'default'
  }
  return cursorAt(at, state.value.regions, state.value.cursor.cursor)
})

/**
 * Whether the canvas painted a picture: until then the view shows the blank
 * page the tab showed before it, so the page's first picture replaces it
 * with nothing between them (`live-view.md` § A browser tab in the panel).
 */
const painted = ref(false)
let pictures: CanvasPictures | null = null
let ticking: ReturnType<typeof setInterval> | null = null
let move: { x: number; y: number; event: PointerEvent } | null = null
let moving: ReturnType<typeof setInterval> | null = null
let composing = false
let committed: string | undefined
const clicks = new ClickCount()

function point(event: { clientX: number; clientY: number }): { x: number; y: number } {
  const bounds = frame.value?.getBoundingClientRect()
  const inside = { x: event.clientX - (bounds?.left ?? 0), y: event.clientY - (bounds?.top ?? 0) }
  return tabPoint(inside, generation.value.viewport, placement.value)
}

function flushMove(): void {
  if (move) {
    // The event itself: a DOM event's fields are getters on its prototype, so a spread copy has none of them.
    props.session?.input(pointerMessage(props.tab.id, 'move', move, move.event))
    move = null
  }
}

function pointerDown(event: PointerEvent): void {
  event.preventDefault()
  const bounds = frame.value?.getBoundingClientRect()
  // The field follows the pointer, so an input method composes where the
  // viewer is typing, and the viewer's own paste reaches this page first.
  // It takes no pointer events, so every click, a double click's second
  // included, reaches the picture.
  if (bridge.value && bounds) {
    bridge.value.style.left = `${Math.max(0, Math.min(bounds.width - 2, event.clientX - bounds.left))}px`
    bridge.value.style.top = `${Math.max(0, Math.min(bounds.height - 2, event.clientY - bounds.top))}px`
    bridge.value.focus({ preventScroll: true })
  }
  canvas.value?.setPointerCapture(event.pointerId)
  flushMove()
  props.session?.input(pointerMessage(props.tab.id, 'down', point(event), event, clicks.press(event)))
}

function pointerMove(event: PointerEvent): void {
  const at = point(event)
  pointer.value = at
  if (event.buttons) {
    move = null
    props.session?.input(pointerMessage(props.tab.id, 'move', at, event))
    return
  }
  // Hover moves are worth at most one message a frame.
  move = { ...at, event }
}

function pointerUp(event: PointerEvent): void {
  flushMove()
  if (canvas.value?.hasPointerCapture(event.pointerId)) {
    canvas.value.releasePointerCapture(event.pointerId)
  }
  props.session?.input(pointerMessage(props.tab.id, 'up', point(event), event, clicks.current))
}

function wheel(event: WheelEvent): void {
  event.preventDefault()
  flushMove()
  props.session?.input(wheelMessage(props.tab.id, point(event), event, generation.value.viewport.height))
}

function key(event: KeyboardEvent, action: 'down' | 'up'): void {
  if (composing || composingKey(event)) {
    return
  }
  // DOM event fields are prototype getters; copying an event drops modifiers.
  // Paste is the viewer's own: the field's paste event carries its clipboard.
  if (localKey(event)) {
    return
  }
  if (action === 'down') {
    committed = undefined
    // A copy here is the viewer's own: its clipboard takes what the tab copies.
    if ((event.ctrlKey || event.metaKey) && !event.getModifierState('AltGraph') && /^[cx]$/i.test(event.key)) {
      viewerClipboard.expect()
    }
  }
  props.session?.input(keyMessage(props.tab.id, action, event))
  event.preventDefault()
}

function typed(event: Event): void {
  const field = bridge.value
  if (!field || composing || (event as InputEvent).isComposing) {
    return
  }
  const text = (event as InputEvent).data ?? field.value
  field.value = ''
  // An input method's commit already arrived as text.
  if (committed !== undefined && text === committed) {
    committed = undefined
    return
  }
  if (text) {
    props.session?.input({ type: 'text', tab: props.tab.id, text })
  }
}

function pasted(event: ClipboardEvent): void {
  event.preventDefault()
  const text = event.clipboardData?.getData('text/plain') ?? ''
  const html = event.clipboardData?.getData('text/html') ?? ''
  if (text || html) {
    props.session?.input({
      type: 'paste',
      tab: props.tab.id,
      text: text.slice(0, 1_000_000),
      html: html.length <= 4_000_000 ? html : '',
    })
  }
  if (bridge.value) {
    bridge.value.value = ''
  }
}

function composition(event: CompositionEvent, phase: 'start' | 'update' | 'end'): void {
  composing = phase !== 'end'
  if (phase === 'start') {
    return
  }
  if (phase === 'update') {
    props.session?.input({ type: 'composition', tab: props.tab.id, text: event.data })
    return
  }
  committed = event.data
  props.session?.input(
    event.data
      ? { type: 'text', tab: props.tab.id, text: event.data }
      : { type: 'composition', tab: props.tab.id, text: '' },
  )
  if (bridge.value) {
    bridge.value.value = ''
  }
}

/** Leaving the view releases what the viewer holds in the page. */
function release(): void {
  move = null
  composing = false
  if (bridge.value) {
    bridge.value.value = ''
  }
  props.session?.release()
}

/** A shown view takes the session's pictures; a hidden one keeps its last picture. */
function takePictures(): void {
  if (pictures && props.session && props.shown) {
    props.session.attach(pictures)
  }
}

onMounted(() => {
  if (canvas.value) {
    pictures = new CanvasPictures(canvas.value, {
      shown: (stream, sequence, queue) => {
        painted.value = true
        picture.value = stream
        props.session?.showed(stream.generation, sequence, queue)
      },
      lost: () => props.session?.resync(),
    })
  }
  takePictures()
  ticking = setInterval(() => props.session?.tick(), 250)
  moving = setInterval(flushMove, 16)
  addEventListener('blur', release)
})

watch([() => props.session, () => props.shown], takePictures)

// Hidden, the view lets go of what the viewer holds in the page.
watch(() => props.shown, (shown) => {
  if (!shown) {
    release()
    pointer.value = null
  }
})

onBeforeUnmount(() => {
  removeEventListener('blur', release)
  if (ticking !== null) {
    clearInterval(ticking)
  }
  if (moving !== null) {
    clearInterval(moving)
  }
  pictures?.stop()
  release()
})
</script>

<template>
  <!-- The page's cursor shows on the frame, and every element over the picture inherits it. -->
  <!-- A Web picture's uncovered part is white, as a local window's while it resizes; a phone's sides are the panel's. -->
  <div
    ref="frame"
    class="relative min-h-0 flex-1 overflow-hidden"
    :class="generation.viewport.mode === 'web' ? 'bg-white' : 'bg-surface-base'"
    :style="{ cursor }"
  >
    <canvas
      ref="canvas"
      class="absolute origin-top-left"
      :style="{
        cursor: 'inherit',
        left: `${placement.left}px`,
        top: `${placement.top}px`,
        width: `${placement.width}px`,
        height: `${placement.height}px`,
      }"
      @pointerdown="pointerDown"
      @pointermove="pointerMove"
      @pointerup="pointerUp"
      @pointercancel="release"
      @pointerleave="pointer = null"
      @wheel.prevent="wheel"
      @contextmenu.prevent
    />
    <div v-if="!painted" class="pointer-events-none absolute inset-0 bg-white" />
    <!-- A notice that leaves no picture takes the picture's place; the page's controls and dialogs stay over it. -->
    <RegionStatus
      v-if="state?.pictureless"
      class="absolute inset-0 bg-surface"
      failed
      label="Couldn’t show this page."
      :detail="refusalSentence(state.pictureless)"
    />
    <input
      ref="bridge"
      class="pointer-events-none absolute h-[2px] w-[2px] border-0 bg-transparent p-0 text-transparent caret-transparent outline-none"
      aria-label="Browser input"
      autocomplete="off"
      autocorrect="off"
      spellcheck="false"
      @keydown="key($event, 'down')"
      @keyup="key($event, 'up')"
      @input="typed"
      @paste="pasted"
      @compositionstart="composition($event, 'start')"
      @compositionupdate="composition($event, 'update')"
      @compositionend="composition($event, 'end')"
      @blur="release"
    >
    <LiveControls
      v-if="session"
      :session="session"
      :controls="session.state.controls"
      :placement="placement"
    />
    <div
      v-if="stalled"
      class="pointer-events-none absolute inset-x-0 top-0 flex justify-center p-2"
    >
      <span class="rounded-md bg-surface px-2 py-1 text-[12px] text-fg-subtle shadow">
        Waiting for the host…
      </span>
    </div>
    <LiveDialog
      v-if="session?.state.dialog"
      :dialog="session.state.dialog.dialog"
      @answer="session.answerDialog($event.accept, $event.text)"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import type { LiveTab } from '@demicodes/browser-protocol/live'
import LiveControls from './LiveControls.vue'
import LiveDialog from './LiveDialog.vue'
import { CanvasPictures, picturesSupported } from './pictures'
import type { LiveSession } from './session'
import { viewerClipboard } from './clipboard'
import { keyMessage, localKey, composingKey, pointerMessage, wheelMessage } from './input'
import { panelSize, placePicture, tabPoint, type PanelSize } from './view'

/**
 * A tab of the conversation's browser, live (`browser-live-view.md`): its
 * pictures on a canvas, the viewer's input on its way to the page, and the
 * page's own native controls and dialogs over it.
 */
const props = defineProps<{ session: LiveSession; tab: LiveTab }>()

const frame = ref<HTMLDivElement | null>(null)
const canvas = ref<HTMLCanvasElement | null>(null)
/** The viewer's keys, input method and clipboard go through this field. */
const bridge = ref<HTMLInputElement | null>(null)
const panel = ref<PanelSize>({ width: 1, height: 1 })
const supported = picturesSupported()
const state = props.session.state
const placement = computed(() => placePicture(props.tab.viewport, panel.value))
const stalled = computed(() => state.connection === 'stalled')
const cursor = computed(() => {
  // A page can name a cursor this browser has no rule for, or an image.
  const name = state.cursor.cursor
  return /^[a-z-]+$/.test(name) && name !== 'auto' ? name : 'default'
})

let pictures: CanvasPictures | null = null
let ticking: ReturnType<typeof setInterval> | null = null
let observer: ResizeObserver | null = null
let density: MediaQueryList | null = null
let move: { x: number; y: number; event: PointerEvent } | null = null
let moving: ReturnType<typeof setInterval> | null = null
let composing = false
let committed: string | undefined

function report(): void {
  const bounds = frame.value?.getBoundingClientRect()
  if (!bounds || bounds.width < 1 || bounds.height < 1) {
    return
  }
  panel.value = panelSize(bounds.width, bounds.height)
  props.session.panel(
    panel.value,
    devicePixelRatio,
    panelSize(screen.width, screen.height),
  )
}

/** The screen's density can change when the window moves to another display. */
function watchDensity(): void {
  density?.removeEventListener('change', watchDensity)
  density = matchMedia(`(resolution: ${devicePixelRatio}dppx)`)
  density.addEventListener('change', watchDensity)
  report()
}

function point(event: { clientX: number; clientY: number }): { x: number; y: number } {
  const bounds = frame.value?.getBoundingClientRect()
  const inside = { x: event.clientX - (bounds?.left ?? 0), y: event.clientY - (bounds?.top ?? 0) }
  return tabPoint(inside, props.tab.viewport, placement.value)
}

function flushMove(): void {
  if (move) {
    // The event itself: a DOM event's fields are getters on its prototype, so a spread copy has none of them.
    props.session.input(pointerMessage(props.tab.id, 'move', move, move.event))
    move = null
  }
}

function pointerDown(event: PointerEvent): void {
  event.preventDefault()
  const bounds = frame.value?.getBoundingClientRect()
  // The field follows the pointer, so an input method composes where the
  // viewer is typing, and the viewer's own paste reaches this page first.
  if (bridge.value && bounds) {
    bridge.value.style.left = `${Math.max(0, Math.min(bounds.width - 2, event.clientX - bounds.left))}px`
    bridge.value.style.top = `${Math.max(0, Math.min(bounds.height - 22, event.clientY - bounds.top))}px`
    bridge.value.focus({ preventScroll: true })
  }
  canvas.value?.setPointerCapture(event.pointerId)
  flushMove()
  props.session.input(pointerMessage(props.tab.id, 'down', point(event), event))
}

function pointerMove(event: PointerEvent): void {
  const at = point(event)
  if (event.buttons) {
    move = null
    props.session.input(pointerMessage(props.tab.id, 'move', at, event))
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
  props.session.input(pointerMessage(props.tab.id, 'up', point(event), event))
}

function wheel(event: WheelEvent): void {
  event.preventDefault()
  flushMove()
  props.session.input(wheelMessage(props.tab.id, point(event), event, props.tab.viewport.height))
}

function key(event: KeyboardEvent, action: 'down' | 'up'): void {
  if (composing || composingKey(event)) {
    return
  }
  const input = {
    ...event,
    key: event.key,
    code: event.code,
    keyCode: event.keyCode,
    repeat: event.repeat,
    location: event.location,
    altGraph: event.getModifierState('AltGraph'),
  }
  // Paste is the viewer's own: the field's paste event carries its clipboard.
  if (localKey(input)) {
    return
  }
  if (action === 'down') {
    committed = undefined
    // A copy here is the viewer's own: its clipboard takes what the tab copies.
    if ((event.ctrlKey || event.metaKey) && !input.altGraph && /^[cx]$/i.test(event.key)) {
      viewerClipboard.expect()
    }
  }
  props.session.input(keyMessage(props.tab.id, action, input))
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
    props.session.input({ type: 'text', tab: props.tab.id, text })
  }
}

function pasted(event: ClipboardEvent): void {
  event.preventDefault()
  const text = event.clipboardData?.getData('text/plain') ?? ''
  const html = event.clipboardData?.getData('text/html') ?? ''
  if (text || html) {
    props.session.input({
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
    props.session.input({ type: 'composition', tab: props.tab.id, text: event.data })
    return
  }
  committed = event.data
  props.session.input(
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
  props.session.release()
}

onMounted(() => {
  if (canvas.value && supported) {
    pictures = new CanvasPictures(canvas.value, {
      shown: (generation, sequence, queue) => props.session.showed(generation, sequence, queue),
      lost: () => props.session.resync(),
    })
    props.session.attach(pictures)
  }
  observer = new ResizeObserver(() => report())
  if (frame.value) {
    observer.observe(frame.value)
  }
  watchDensity()
  ticking = setInterval(() => props.session.tick(), 250)
  moving = setInterval(flushMove, 16)
  addEventListener('blur', release)
})

onBeforeUnmount(() => {
  observer?.disconnect()
  density?.removeEventListener('change', watchDensity)
  removeEventListener('blur', release)
  if (ticking !== null) {
    clearInterval(ticking)
  }
  if (moving !== null) {
    clearInterval(moving)
  }
  pictures?.stop()
  release()
  // Nothing watches this tab any more, so the Host captures nothing.
  props.session.watch(null)
})

// The tab the view watches decides what the module captures.
watch(() => props.tab.id, (id) => {
  release()
  props.session.watch(id)
  report()
}, { immediate: true })
</script>

<template>
  <div ref="frame" class="relative min-h-0 flex-1 overflow-hidden bg-surface-base">
    <canvas
      ref="canvas"
      class="absolute origin-top-left"
      :style="{
        left: `${placement.left}px`,
        top: `${placement.top}px`,
        width: `${placement.width}px`,
        height: `${placement.height}px`,
        cursor,
      }"
      @pointerdown="pointerDown"
      @pointermove="pointerMove"
      @pointerup="pointerUp"
      @pointercancel="release"
      @wheel.prevent="wheel"
      @contextmenu.prevent
    />
    <input
      ref="bridge"
      class="absolute h-[2px] w-[2px] border-0 bg-transparent p-0 text-transparent caret-transparent outline-none"
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
      :session="session"
      :controls="session.state.controls"
      :placement="placement"
      :viewport="tab.viewport"
    />
    <div
      v-if="!supported"
      class="absolute inset-0 flex items-center justify-center bg-surface text-[13px] text-fg-faint"
    >
      This browser cannot show the live view: it has no video decoder.
    </div>
    <div
      v-else-if="stalled"
      class="pointer-events-none absolute inset-x-0 top-0 flex justify-center p-2"
    >
      <span class="rounded-md bg-surface px-2 py-1 text-[12px] text-fg-subtle shadow">
        Waiting for the Host…
      </span>
    </div>
    <LiveDialog
      v-if="session.state.dialog"
      :dialog="session.state.dialog.dialog"
      @answer="session.answerDialog($event.accept, $event.text)"
    />
  </div>
</template>

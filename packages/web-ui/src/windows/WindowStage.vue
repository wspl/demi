<script setup lang="ts">
import { computed, onBeforeUnmount, ref, shallowRef, watch, type Component } from 'vue'
import { useElementSize, useEventListener, usePreferredReducedMotion } from '@vueuse/core'
import { clamp } from '@demicodes/utils'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { startPointerDrag } from '../ui/pointer-drag'
import { resizeWindowRect, type WindowEdges, type WindowRect } from '../ui/floating-window'
import type { ContextMenuGroup } from '../ui/context-menu'
import WorkWindow, { type WindowTab } from './WorkWindow.vue'
import {
  WINDOW_GAP_PX,
  COLUMN_MIN_PX,
  COLUMN_GUTTER_PX,
  CONVERSATION_MIN_PX,
  UNDOCK_PX,
  WINDOW_MOTION_MS,
  boxOfRect,
  columnRect,
  columnSlot,
  cascade,
  cornerRect,
  keptInside,
  openingBox,
  rectOfBox,
  dropTarget,
  rowRects,
  type DropTarget,
  type Point,
  type StageWindow,
  type WindowBox,
  type WindowStageModel,
} from './stage'
import { t } from './locales/text'
import { moved } from '../ui/drag-order'

/**
 * The conversation's area with its work's windows (`stage.ts`): the
 * conversation, the column docked at its right, and the windows floating or
 * maximized over them.
 *
 * Every window stays mounted wherever it stands, minimized ones hidden, so a
 * page or a file it shows is not loaded again when it moves.
 *
 * Moving a window:
 *
 * - Dragging its title bar's room moves a floating window, which stays a gap
 *   inside the area: an edge that reaches the gap stops there and the window
 *   narrows behind the pointer, as long as the drag lasts. An anchored window
 *   does not move. A maximized one comes out as a floating window under the
 *   pointer once the press has moved a few px. It shrinks to its floating
 *   size in one motion while it follows the pointer, which holds the same
 *   spot of its title bar throughout, as a system's maximized window dragged
 *   by its title bar does.
 * - A window lands by where its middle is, not the pointer. While its middle
 *   is over the column, or over the area's right quarter while the column is
 *   empty, a box shows where it lands and what it does there (Dock, Place
 *   Above, Replace, Place Below); releasing it there docks it. While the box
 *   shows, the window keeps its size and stops at the area's edges.
 * - A docked window dragged along the column keeps its row's size and leaves
 *   its row empty, the others where they are. Over another row, the others
 *   step aside as they will stand once it lands there (Move Here); over its
 *   own, it goes back. Its middle beside the column, it comes out floating,
 *   shrinking to its floating size, and the column closes up when it is let
 *   go there.
 * - Escape puts it back where it was.
 *
 * Escape closes the window in front while it floats, is anchored or is
 * maximized, as a glance at a file or a picture is put away, unless a field
 * holds the keys.
 *
 * Sizes: the divider before the column sizes the column, the gaps between its
 * rows size the rows, a floating window's edges and corners size it, and an
 * anchored one's top and left edges and their corner size it. While any of
 * these drags, the windows follow it with no motion. A window opens above the
 * `bottom` px the composer keeps; a floating or anchored one may cover the
 * composer; a maximized one covers the conversation's side of the area, the
 * composer included, and leaves the column as it is, a gap inside its edges
 * as the column's rows keep. Each
 * drag or resize let go is settled in the model (`settle`).
 *
 * Windows move between places with motion, but stand at once where the area
 * takes its first size or shows another conversation's layout: a page that
 * opens shows its windows where they were, never flying in.
 *
 * On a phone (`phone`) only the window in front shows, over the whole area,
 * and none moves, sizes or docks.
 */
export interface StageWindowView {
  id: string
  icon: Component
  name: string
  tabs: readonly WindowTab[]
  current: string | null
  opensTabs?: boolean
  /** It only ever shows its one tab, which its title bar names in place of a tab. */
  single?: boolean
  /** The rows that act on every tab, its title bar's context menu. */
  menu?: () => readonly ContextMenuGroup[]
  /** It opens anchored in the corner, and its title bar switches it between there and floating. */
  anchors?: boolean
}

const props = defineProps<{
  model: WindowStageModel
  windows: readonly StageWindowView[]
  /** The px under the floating windows that the composer keeps. */
  bottom: number
  phone?: boolean
}>()

defineSlots<{
  /** The conversation, beside the column. */
  default(): unknown
  /** The window `id`'s content. */
  content(props: { id: string }): unknown
  /** What the window `id` adds to its title bar. */
  trailing(props: { id: string }): unknown
}>()

const emit = defineEmits<{
  select: [id: string, tab: string]
  closeTab: [id: string, tab: string]
  reorder: [id: string, from: number, to: number]
  newTab: [id: string]
  close: [id: string]
  /** Escape outside a field, while `id` floats, is anchored or is maximized in front. */
  escape: [id: string]
}>()

const area = ref<HTMLElement | null>(null)
const { width, height } = useElementSize(area)
const size = computed(() => ({ width: width.value, height: height.value }))
/** Whether the area has its size, which `opening` needs. */
const measured = computed(() => width.value > 0 && height.value > 0)
const state = computed(() => props.model.state)

/**
 * The column as it was when one of its windows was dragged out of it: its
 * rows hold their places, the dragged one's empty, until the window is let
 * go, so the drag alone never makes a row take another's room. Over another
 * row, the others step aside as they will stand once it lands there, as a
 * list's rows make room for one dragged along it.
 */
const held = shallowRef<{ id: string; column: readonly string[]; shares: readonly number[] } | null>(null)
/** The row the window dragged along the column would take, and the one it left. */
const heldMove = computed(() => (target.value?.kind === 'move' ? target.value : null))
/** The column's windows as it draws them, by row. */
const shownColumn = computed(() => {
  const kept = held.value
  const move = heldMove.value
  return kept ? (move ? moved(kept.column, move.from, move.row) : kept.column) : state.value.column
})
const shownShares = computed(() => {
  const kept = held.value
  const move = heldMove.value
  return kept ? (move ? moved(kept.shares, move.from, move.row) : kept.shares) : state.value.rowShares
})
const columnShown = computed(() => !props.phone && shownColumn.value.length > 0)
const column = computed(() => columnRect(size.value, state.value.share))
const rows = computed(() => rowRects(column.value, shownColumn.value.length, shownShares.value))
const conversationWidth = computed(() => columnShown.value ? column.value.x : size.value.width)
/** The area above the composer, where a window opens. */
const bounds = computed(() => ({ width: size.value.width, height: Math.max(0, size.value.height - props.bottom) }))

/** The window being moved, which shows where it would land. */
const target = shallowRef<DropTarget | null>(null)
const moving = ref<string | null>(null)
/** A drag under the pointer, of a window, an edge, the column or a row: the windows follow it with no motion, which would lag. */
const following = ref(false)

const reducedMotion = usePreferredReducedMotion()

/** A window coming out of the column or out of maximized, on its way to its floating size. */
interface Emerging {
  id: string
  /** Its size where it stood. */
  from: { width: number; height: number }
  /** Where the pointer held it where it stood, and where it holds its floating box. */
  heldFrom: Point
  heldTo: Point
  /** From 0 as it comes out to 1 at its floating size. */
  progress: number
}

const emerging = shallowRef<Emerging | null>(null)
let emergingFrame = 0

/**
 * A docked window carried along the column under the pointer, at its row's
 * size: it keeps that size while the pointer stays over the column, so a
 * drag that ends where it began changes nothing on the way, and comes out
 * floating only beside the column.
 */
const lifted = shallowRef<{ id: string; rect: WindowRect } | null>(null)

/**
 * The size a floating window keeps while a box shows where it would land:
 * it stops at the area's edges rather than narrowing against them, so a hand
 * held still over a place to dock never makes it change size.
 */
const frozen = shallowRef<{ id: string; width: number; height: number } | null>(null)

function emerge(next: Emerging | null): void {
  cancelAnimationFrame(emergingFrame)
  emerging.value = reducedMotion.value === 'reduce' ? null : next
  if (!emerging.value) {
    return
  }
  const start = performance.now()
  const step = (now: number) => {
    const progress = Math.min(1, (now - start) / WINDOW_MOTION_MS)
    emerging.value = progress < 1 && emerging.value ? { ...emerging.value, progress } : null
    if (emerging.value) {
      emergingFrame = requestAnimationFrame(step)
    }
  }
  emergingFrame = requestAnimationFrame(step)
}

onBeforeUnmount(() => cancelAnimationFrame(emergingFrame))

/** Whether windows move with motion: not until the area has its size and a conversation's layout has been drawn once. */
const animated = ref(false)
let animatedFrame = 0
watch([measured, () => props.model], ([sized]) => {
  cancelAnimationFrame(animatedFrame)
  animated.value = false
  if (sized) {
    // The frame after the one that draws the layout.
    animatedFrame = requestAnimationFrame(() => {
      animatedFrame = requestAnimationFrame(() => {
        animated.value = true
      })
    })
  }
}, { immediate: true })

onBeforeUnmount(() => cancelAnimationFrame(animatedFrame))

/**
 * The area changing size, as while the browser's window is resized: the
 * windows and the conversation follow it with no motion, which would lag.
 */
const sizing = ref(false)
/** How long the area keeps its size before a resize counts as ended, in ms: a resize sends a size every frame, so a few frames of quiet end it. */
const SIZING_QUIET_MS = 100
let sizingEnd: ReturnType<typeof setTimeout> | null = null
watch(size, () => {
  sizing.value = true
  if (sizingEnd) {
    clearTimeout(sizingEnd)
  }
  sizingEnd = setTimeout(() => {
    sizing.value = false
    sizingEnd = null
  }, SIZING_QUIET_MS)
})

onBeforeUnmount(() => {
  if (sizingEnd) {
    clearTimeout(sizingEnd)
  }
})

/** Whether a change of layout moves with motion: never while something follows the pointer or the area's size. */
const moves = computed(() => animated.value && !following.value && !sizing.value)

/**
 * Where each window was last drawn: a minimized one keeps it, so it fades out
 * where it stood, not where it would float, and never moves while hidden.
 */
const lastDrawn = new Map<string, WindowRect>()

function rectOf(window: StageWindow): WindowRect {
  if (window.placement === 'minimized') {
    return lastDrawn.get(window.id) ?? placedRect(window)
  }
  const rect = placedRect(window)
  lastDrawn.set(window.id, rect)
  return rect
}

/** Where `window` stands for its placement; a minimized one, where it would float. */
function placedRect(window: StageWindow): WindowRect {
  if (props.phone) {
    return { x: 0, y: 0, width: size.value.width, height: size.value.height }
  }
  if (lifted.value?.id === window.id) {
    return lifted.value.rect
  }
  if (window.placement === 'maximized') {
    // Up to the gutter before the column, whose divider still sizes it.
    const right = columnShown.value ? column.value.x : size.value.width - WINDOW_GAP_PX
    return {
      x: WINDOW_GAP_PX,
      y: WINDOW_GAP_PX,
      width: Math.max(0, right - WINDOW_GAP_PX),
      height: Math.max(0, size.value.height - 2 * WINDOW_GAP_PX),
    }
  }
  if (window.placement === 'docked') {
    return rows.value[shownColumn.value.indexOf(window.id)] ?? own(window)
  }
  if (window.placement === 'anchored') {
    return cornerRect(window.corner, size.value)
  }
  // A drag keeps the box it would reach; the window shows it kept inside.
  const still = frozen.value?.id === window.id ? frozen.value : null
  const free = own(window)
  const box = still
    ? {
        x: clamp(Math.round(free.x), WINDOW_GAP_PX, size.value.width - WINDOW_GAP_PX - still.width),
        y: clamp(Math.round(free.y), WINDOW_GAP_PX, size.value.height - WINDOW_GAP_PX - still.height),
        width: still.width,
        height: still.height,
      }
    : keptInside(free, size.value)
  const way = moving.value === window.id && emerging.value?.id === window.id ? emerging.value : null
  if (!way) {
    return box
  }
  // Eased out, as the CSS motion between places is; the spot the pointer
  // holds moves from where it was on the window to where it is on the box, so
  // the pointer stays on it.
  const p = 1 - (1 - way.progress) ** 3
  return {
    x: box.x + (1 - p) * (way.heldTo.x - way.heldFrom.x),
    y: box.y + (1 - p) * (way.heldTo.y - way.heldFrom.y),
    width: way.from.width + p * (box.width - way.from.width),
    height: way.from.height + p * (box.height - way.from.height),
  }
}

/** `window`'s floating box, in px. */
function own(window: StageWindow): WindowRect {
  return rectOfBox(window.box, size.value)
}

function place(window: StageWindow, rect: WindowRect): void {
  window.box = boxOfRect(rect, size.value)
}

/** The boxes of the floating windows other than `id`. */
function floatingRects(id?: string): WindowRect[] {
  return state.value.windows.filter((window) => window.placement === 'floating' && window.id !== id).map(own)
}

/** Where a window opening for the first time floats: in the middle, past those floating there. */
function opening(): WindowBox {
  return openingBox(size.value, bounds.value, floatingRects())
}

/**
 * How long a window takes to come in, in ms, and to go, a little quicker, as
 * a macOS window opens and closes.
 */
const APPEAR_MS = 250
const VANISH_MS = 180
/** The curve a window settles in on as it comes, as a macOS window does. */
const SETTLE = 'cubic-bezier(0.32, 0.72, 0, 1)'
/** The curve it leaves on, speeding up as it goes. */
const DEPART = 'cubic-bezier(0.4, 0, 1, 1)'

/** The windows about to show because they opened or came back from the taskbar, which the watch below learns before they show. */
const coming = new Set<string>()

/** How small a window is as it starts to come in and as it is gone. */
const AWAY = { opacity: 0, transform: 'scale(0.96)' }
const HERE = { opacity: 1, transform: 'none' }

/** Runs `frames` on `element`, where it stands, taking no press meanwhile, then `done`. */
function play(element: HTMLElement, frames: Keyframe[], duration: number, easing: string, done: () => void): void {
  element.style.pointerEvents = 'none'
  element.style.transition = 'none'
  const motion = element.animate(frames, { duration, easing })
  motion.onfinish = motion.oncancel = () => {
    element.style.pointerEvents = ''
    element.style.transition = ''
    done()
  }
}

/**
 * A window that opens or comes back from the taskbar grows into place as it
 * fades in, as a macOS window opens. One that shows again for another reason,
 * such as the column once a maximized window is restored, or the page's first
 * layout, just shows.
 */
function enter(element: Element, done: () => void): void {
  const box = element as HTMLElement
  const id = box.dataset.stageWindow ?? ''
  const opened = coming.delete(id)
  if (!opened || !animated.value || reducedMotion.value === 'reduce') {
    done()
    return
  }
  play(box, [AWAY, HERE], APPEAR_MS, SETTLE, done)
}

/** A window that closes or goes to the taskbar shrinks a little as it fades out, where it stands. */
function leave(element: Element, done: () => void): void {
  const box = element as HTMLElement
  const window = props.model.find(box.dataset.stageWindow ?? '')
  if ((window && window.placement !== 'minimized') || !animated.value || reducedMotion.value === 'reduce') {
    done()
    return
  }
  play(box, [HERE, AWAY], VANISH_MS, DEPART, done)
}

// A window that comes back from the taskbar, or opens again where it stood
// when it closed, steps past one that floats where it stood, never covering
// it, and comes in with motion (`enter`); one dragged out of the column or
// out of maximized stays under the pointer. Another conversation's layout
// shows as it was kept, at once.
watch(() => ({ model: props.model, placed: state.value.windows.map((window) => [window.id, window.placement] as const) }), (now, before) => {
  if (!measured.value || now.model !== before?.model) {
    return
  }
  const was = new Map(before.placed)
  coming.clear()
  for (const [id, placement] of now.placed) {
    const window = props.model.find(id)
    const came = was.get(id)
    if (!window || placement === 'minimized' || (came !== undefined && came !== 'minimized') || moving.value === id) {
      continue
    }
    if (placement === 'floating') {
      place(window, cascade(keptInside(own(window), size.value), floatingRects(id), bounds.value))
    }
    coming.add(id)
  }
})

defineExpose({ opening, measured })

/** The keys a field takes for itself, Escape among them. */
const FIELD = 'input, textarea, select, [contenteditable=""], [contenteditable="true"]'

useEventListener(document, 'keydown', (event: KeyboardEvent) => {
  if (event.key !== 'Escape' || event.defaultPrevented || event.altKey || event.ctrlKey || event.metaKey || event.shiftKey || moving.value) {
    return
  }
  if (event.target instanceof Element && event.target.closest(FIELD)) {
    return
  }
  const window = state.value.focused ? props.model.find(state.value.focused) : undefined
  if (window && (window.placement === 'floating' || window.placement === 'anchored' || window.placement === 'maximized')) {
    event.preventDefault()
    emit('escape', window.id)
  }
})

/** Docked windows lie under the others, which stand in the order they came in front, a maximized one among them. */
function layer(window: StageWindow): number {
  return window.placement === 'docked' && lifted.value?.id !== window.id ? 10 : 100 + window.order
}

/**
 * The windows by id, an order nothing changes: one arranged on the taskbar
 * keeps its element where it is, as moving it would load its page again.
 */
const shown = computed(() => props.windows
  .flatMap((view) => {
    const window = props.model.find(view.id)
    return window ? [{ view, window }] : []
  })
  .sort((a, b) => (a.view.id < b.view.id ? -1 : a.view.id > b.view.id ? 1 : 0)))

function local(event: PointerEvent): { x: number; y: number } {
  const box = area.value?.getBoundingClientRect()
  return { x: event.clientX - (box?.left ?? 0), y: event.clientY - (box?.top ?? 0) }
}

/**
 * How far past the edge between two landings the pointer goes before the
 * landing changes, in px: a hand held still on the edge trembles across it,
 * and would flip the preview every frame.
 */
const LANDING_SLACK_PX = 12

/** Where a window held at `point` lands: the landing it has while one within the slack still gives it, else the one `at` gives. */
function landing(point: { x: number; y: number }, at: (point: { x: number; y: number }) => DropTarget | null): DropTarget | null {
  const now = target.value
  const next = at(point)
  const same = (other: DropTarget | null) => other?.kind === now?.kind && other?.row === now?.row
  if (same(next)) {
    return next
  }
  const near = [[-1, 0], [1, 0], [0, -1], [0, 1]].map(([x, y]) => ({ x: point.x + x! * LANDING_SLACK_PX, y: point.y + y! * LANDING_SLACK_PX }))
  return near.some((other) => same(at(other))) ? now : next
}

function grab(id: string, event: PointerEvent): void {
  const window = props.model.find(id)
  if (!window) {
    return
  }
  props.model.focus(id)
  // An anchored window stays in its corner.
  if (window.placement === 'anchored') {
    return
  }
  const start = local(event)
  const from = { placement: window.placement, return: window.return, row: state.value.column.indexOf(id), box: { ...window.box } }
  const drawn = rectOf(window)
  // Where in the window the pointer holds it: the same spot of a floating
  // window, and the same share of the title bar's width of one that comes out.
  let hold = { x: start.x - drawn.x, y: start.y - drawn.y }
  const share = (start.x - drawn.x) / Math.max(1, drawn.width)
  let out = window.placement === 'floating'
  /** A docked window carried along the column, still docked, until it leaves it. */
  let carried = false
  if (out) {
    moving.value = id
  }
  following.value = true

  const dragged: StageWindow = window
  /** It comes out floating under the pointer at `point`, shrinking to its floating box. */
  function comeOut(point: Point): void {
    out = true
    const floating = own(dragged)
    const heldFrom = hold
    // The same share of the title bar, further along where the box would pass the area's far edge,
    // so a window that comes out by the edge comes out whole, not narrowed against it.
    const fits = point.x - (size.value.width - WINDOW_GAP_PX - floating.width)
    hold = { x: Math.min(Math.max(Math.min(share * floating.width, floating.width - 48), fits), floating.width - 48), y: Math.min(hold.y, 20) }
    props.model.lift(id)
    moving.value = id
    emerge({ id, from: { width: drawn.width, height: drawn.height }, heldFrom, heldTo: hold, progress: 0 })
  }

  startPointerDrag(event, {
    // The arrow, as a system's window keeps it while dragged by its title bar.
    cursor: 'default',
    move(dx, dy) {
      const point = { x: start.x + dx, y: start.y + dy }
      if (!out && !carried) {
        if (Math.hypot(dx, dy) < UNDOCK_PX) {
          return
        }
        if (from.placement === 'docked') {
          held.value = { id, column: [...state.value.column], shares: [...state.value.rowShares] }
          carried = true
          moving.value = id
        } else {
          comeOut(point)
        }
      }
      const kept = held.value
      // Along the column a window lands by where it is, its middle, not by where the pointer holds it.
      const slot = (at: Point) => kept ? columnSlot(at, size.value, state.value.share, kept.column.length, kept.shares, from.row) : null
      if (carried && kept) {
        const x = clamp(point.x - hold.x, WINDOW_GAP_PX, size.value.width - WINDOW_GAP_PX - drawn.width)
        const y = clamp(point.y - hold.y, WINDOW_GAP_PX, size.value.height - WINDOW_GAP_PX - drawn.height)
        const middle = { x: x + drawn.width / 2, y: y + drawn.height / 2 }
        if (slot(middle)) {
          lifted.value = { id, rect: { x, y, width: drawn.width, height: drawn.height } }
          target.value = landing(middle, slot)
          return
        }
        // Beside the column it comes out floating, as from any place.
        carried = false
        lifted.value = null
        comeOut(point)
      }
      place(window, { ...own(window), x: point.x - hold.x, y: point.y - hold.y })
      // A window lands by where it is, its middle, as it shows now.
      const shown = rectOf(window)
      const middle = { x: shown.x + shown.width / 2, y: shown.y + shown.height / 2 }
      if (kept) {
        target.value = landing(middle, slot)
      } else {
        const others = state.value.column.filter((other) => other !== id)
        const shares = others.length === state.value.rowShares.length ? state.value.rowShares : []
        target.value = landing(middle, (at) => dropTarget(at, size.value, state.value.share, others.length, shares))
      }
      if (!target.value) {
        frozen.value = null
      } else if (frozen.value?.id !== id) {
        frozen.value = { id, width: shown.width, height: shown.height }
      }
    },
    end(cancelled) {
      const landing = target.value
      const kept = held.value
      target.value = null
      held.value = null
      lifted.value = null
      frozen.value = null
      moving.value = null
      following.value = false
      emerge(null)
      if (carried) {
        // Still docked: it moves to the row under the pointer, or settles back into its own.
        if (!cancelled && kept && landing?.kind === 'move' && landing.row !== landing.from) {
          props.model.arrangeColumn(id, moved(kept.column, landing.from, landing.row), moved(kept.shares, landing.from, landing.row))
        }
        return
      }
      if (!out) {
        return
      }
      if (cancelled) {
        window.box = from.box
        if (kept) {
          props.model.arrangeColumn(id, kept.column, kept.shares)
        } else if (from.placement === 'maximized') {
          // Back to maximized, still returning where it did before.
          window.return = from.return
          props.model.maximize(id)
        }
        return
      }
      place(window, keptInside(own(window), size.value))
      if (kept && landing?.kind === 'move') {
        // Let go over the column, the window takes the row under the pointer, its own one putting it back.
        props.model.arrangeColumn(id, moved(kept.column, landing.from, landing.row), moved(kept.shares, landing.from, landing.row))
      } else if (landing?.kind === 'replace') {
        props.model.replace(id, landing.row)
      } else if (landing) {
        props.model.dock(id, landing.row)
      } else {
        props.model.settle()
      }
    },
  })
}

const EDGES: readonly { edges: WindowEdges; class: string }[] = [
  { edges: { top: true }, class: 'inset-x-2 -top-1 h-2 cursor-ns-resize' },
  { edges: { bottom: true }, class: 'inset-x-2 -bottom-1 h-2 cursor-ns-resize' },
  { edges: { left: true }, class: 'inset-y-2 -left-1 w-2 cursor-ew-resize' },
  { edges: { right: true }, class: 'inset-y-2 -right-1 w-2 cursor-ew-resize' },
  { edges: { top: true, left: true }, class: '-left-1 -top-1 size-3 cursor-nwse-resize' },
  { edges: { top: true, right: true }, class: '-right-1 -top-1 size-3 cursor-nesw-resize' },
  { edges: { bottom: true, left: true }, class: '-bottom-1 -left-1 size-3 cursor-nesw-resize' },
  { edges: { bottom: true, right: true }, class: '-bottom-1 -right-1 size-3 cursor-nwse-resize' },
]

function resize(id: string, edges: WindowEdges, event: PointerEvent): void {
  const window = props.model.find(id)
  if (!window || event.button !== 0) {
    return
  }
  props.model.focus(id)
  const before = { box: { ...window.box }, corner: window.corner }
  const start = rectOf(window)
  const anchored = window.placement === 'anchored'
  const cursor = getComputedStyle(event.currentTarget as Element).cursor
  following.value = true
  startPointerDrag(event, {
    cursor,
    move(dx, dy) {
      if (anchored) {
        // Its corner stays; its size is kept in shares of the area.
        const rect = keptInside(resizeWindowRect(start, edges, dx, dy, size.value), size.value)
        window.corner = { width: rect.width / Math.max(1, size.value.width), height: rect.height / Math.max(1, size.value.height) }
      } else {
        place(window, resizeWindowRect(start, edges, dx, dy, size.value))
      }
    },
    end(cancelled) {
      following.value = false
      if (cancelled) {
        window.box = before.box
        window.corner = before.corner
      } else if (!anchored) {
        place(window, keptInside(own(window), size.value))
      }
      props.model.settle()
    },
  })
}

/** The edges a window sizes from: all of a floating one's; an anchored one's away from its corner. */
function edgesOf(window: StageWindow): readonly { edges: WindowEdges; class: string }[] {
  return window.placement === 'anchored' ? EDGES.filter(({ edges }) => !edges.right && !edges.bottom) : EDGES
}

const columnBounds = computed(() => ({
  min: Math.min(COLUMN_MIN_PX, size.value.width),
  max: Math.max(COLUMN_MIN_PX, size.value.width - CONVERSATION_MIN_PX),
}))

function sizeColumn(px: number): void {
  following.value = true
  if (size.value.width > 0) {
    state.value.share = px / size.value.width
  }
}

function columnSized(): void {
  following.value = false
  props.model.settle()
}

/** Drags the gap under the row `index`, sharing the two rows' height anew. */
function sizeRows(index: number, event: PointerEvent): void {
  if (event.button !== 0) {
    return
  }
  const rects = rows.value
  const above = rects[index]
  const below = rects[index + 1]
  if (!above || !below) {
    return
  }
  const shares = [...state.value.rowShares]
  const pair = (shares[index] ?? 0) + (shares[index + 1] ?? 0)
  const span = above.height + below.height
  // A row keeps its title bar and a few lines.
  const least = 96
  following.value = true
  startPointerDrag(event, {
    cursor: 'row-resize',
    move(_dx, dy) {
      const top = Math.min(span - least, Math.max(least, above.height + dy))
      const next = [...shares]
      next[index] = (pair * top) / span
      next[index + 1] = pair - next[index]!
      state.value.rowShares = next
    },
    end(cancelled) {
      following.value = false
      if (cancelled) {
        state.value.rowShares = shares
      }
      props.model.settle()
    },
  })
}

const DROP_LABELS = { dock: 'dropDock', above: 'dropAbove', below: 'dropBelow' } as const

const dropLabel = computed(() => {
  const landing = target.value
  if (!landing) {
    return ''
  }
  if (landing.kind === 'replace') {
    return landing.full ? t('dropReplaceFull') : t('dropReplace')
  }
  if (landing.kind === 'move') {
    // Its own row puts it back as it was: the box says where, with nothing to read.
    return landing.row === landing.from ? '' : t('dropMove')
  }
  return t(DROP_LABELS[landing.kind])
})

/** Anchored goes floating; anywhere else, to its corner. */
function anchorPressed(window: StageWindow): void {
  if (window.placement === 'anchored') {
    props.model.float(window.id)
  } else {
    props.model.anchor(window.id)
  }
}

function maximizePressed(window: StageWindow): void {
  if (window.placement === 'maximized') {
    props.model.restore(window.id)
  } else {
    props.model.maximize(window.id)
  }
}
</script>

<template>
  <div
    ref="area"
    data-window-stage
    class="window-stage relative h-full min-h-0 w-full overflow-hidden"
    :style="{ '--window-motion': `${WINDOW_MOTION_MS}ms` }"
  >
    <!-- The conversation narrows and widens with motion as the column comes, goes or is sized, as the windows move. -->
    <div
      class="absolute inset-y-0 left-0 flex min-w-0"
      :class="moves ? 'stage-motion' : ''"
      :style="{ width: `${conversationWidth}px` }"
    >
      <slot />
    </div>
    <!-- In the middle of the gap before the column, with a grip that shows it sizes, as a split view's divider does. -->
    <div v-if="columnShown" class="group/column absolute inset-y-0 z-20 flex" :style="{ left: `${column.x + COLUMN_GUTTER_PX / 2}px` }">
      <span aria-hidden="true" class="window-grip pointer-events-none absolute left-0 top-1/2 h-10 w-1 -translate-x-1/2 -translate-y-1/2" />
      <ResizeHandle
        :model-value="column.width"
        :min="columnBounds.min"
        :max="columnBounds.max"
        orientation="vertical"
        side="end"
        :label="t('dock')"
        marked
        @update:model-value="(px) => sizeColumn(px)"
        @commit="columnSized"
      />
    </div>
    <template v-if="columnShown">
      <div
        v-for="(rect, index) in rows.slice(0, -1)"
        :key="`gap-${index}`"
        class="group/column absolute z-20 grid cursor-row-resize place-items-center"
        :style="{ left: `${rect.x}px`, width: `${rect.width}px`, top: `${rect.y + rect.height}px`, height: `${WINDOW_GAP_PX}px` }"
        @pointerdown="sizeRows(index, $event)"
      >
        <span aria-hidden="true" class="window-grip pointer-events-none h-1 w-10" />
      </div>
    </template>
    <TransitionGroup :css="false" @enter="enter" @leave="leave">
      <div
        v-for="{ view, window } in shown"
        v-show="phone ? state.focused === view.id && window.placement !== 'minimized' : window.placement !== 'minimized' && (window.placement !== 'docked' || columnShown)"
        :key="view.id"
        :data-stage-window="view.id"
        class="absolute"
        :class="[
          moves || (held && held.id !== view.id && !sizing) ? 'stage-motion' : '',
          window.placement === 'floating' || window.placement === 'anchored' || lifted?.id === view.id ? 'window-floating' : '',
        ]"
        :style="{
          left: `${rectOf(window).x}px`,
          top: `${rectOf(window).y}px`,
          width: `${rectOf(window).width}px`,
          height: `${rectOf(window).height}px`,
          zIndex: layer(window),
        }"
        @pointerdown.capture="model.focus(view.id)"
      >
        <WorkWindow
          :icon="view.icon"
          :name="view.name"
          :tabs="view.tabs"
          :current="view.current"
          :opens-tabs="view.opensTabs"
          :single="view.single"
          :placement="window.placement"
          :focused="state.focused === view.id"
          :menu="view.menu"
          :anchors="view.anchors"
          :phone="phone"
          @select="(tab) => emit('select', view.id, tab)"
          @close-tab="(tab) => emit('closeTab', view.id, tab)"
          @reorder="(from, to) => emit('reorder', view.id, from, to)"
          @new-tab="emit('newTab', view.id)"
          @anchor="anchorPressed(window)"
          @minimize="phone ? model.minimizeAll() : model.minimize(view.id)"
          @maximize="maximizePressed(window)"
          @close="emit('close', view.id)"
          @grab="grab(view.id, $event)"
        >
          <slot name="content" :id="view.id" />
          <template #trailing>
            <slot name="trailing" :id="view.id" />
          </template>
        </WorkWindow>
        <template v-if="(window.placement === 'floating' || window.placement === 'anchored') && !phone">
          <div
            v-for="edge in edgesOf(window)"
            :key="edge.class"
            class="absolute z-10"
            :class="edge.class"
            @pointerdown.stop="resize(view.id, edge.edges, $event)"
          />
        </template>
      </div>
    </TransitionGroup>
    <!-- Under the window that moves, over the column, as a system's snap preview stands. -->
    <div
      v-if="target"
      class="window-drop pointer-events-none absolute z-50 grid place-items-center rounded-xl"
      :style="{ left: `${target.rect.x}px`, top: `${target.rect.y}px`, width: `${target.rect.width}px`, height: `${target.rect.height}px` }"
    >
      <span v-if="dropLabel" class="rounded-lg bg-surface-float px-3 py-1.5 text-chrome font-medium text-fg-emphasis shadow-float">{{ dropLabel }}</span>
    </div>
  </div>
</template>

<style scoped>
.stage-motion {
  transition-property: left, top, width, height;
  transition-duration: var(--window-motion);
  transition-timing-function: cubic-bezier(0.2, 0, 0, 1);
}

/* Shown only while the pointer is on the place it sizes from, as a split view's divider grip is. */
.window-grip {
  border-radius: 9999px;
  background: var(--color-line-strong);
  opacity: 0;
  transition: opacity 150ms;
}

.group\/column:hover .window-grip {
  opacity: 1;
}

.window-floating {
  filter: drop-shadow(0 10px 24px rgb(0 0 0 / 0.22));
}

.window-drop {
  background: color-mix(in oklch, var(--color-accent-fill) 14%, transparent);
  box-shadow: inset 0 0 0 2px color-mix(in oklch, var(--color-accent-fill) 70%, transparent);
  transition-property: left, top, width, height;
  transition-duration: 90ms;
}

@media (prefers-reduced-motion: reduce) {
  .stage-motion,
  .window-drop {
    transition: none;
  }
}
</style>

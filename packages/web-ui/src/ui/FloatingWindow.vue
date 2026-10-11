<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { X } from '@lucide/vue'
import { onKeyStroke, useWindowSize } from '@vueuse/core'
import IconButton from './IconButton.vue'
import { startPointerDrag } from './pointer-drag'
import {
  clampWindowRect,
  escapeClosesWindow,
  moveWindowRect,
  resizeWindowRect,
  WINDOW_TITLE_BAR_PX,
  type WindowEdges,
  type WindowRect,
} from './floating-window'
import { useStackedWindow } from './window-stack'
import { useFocusReturn } from '../overlay/focusReturn'
import { provideLayerElevation } from '../overlay/layerElevation'
import type { OverlayStore } from '../overlay/overlayStore'

/**
 * A small window over the page, as Quick Look's and Preview's are on a Mac:
 * it blocks nothing, so the page beside and beneath it stays usable, with no
 * scrim and no focus held inside it (`file-previews.md` § The media viewer).
 *
 * Move and resize: dragging the title bar moves it, and dragging any edge or
 * corner resizes it, with the matching cursor; it stays at least 240 × 180,
 * no larger than the page, and its title bar on the page, also when the
 * browser window shrinks (`clampWindowRect`). The host keeps the box
 * (v-model:rect), so it can open the next window where the user left this
 * one. A drag captures the pointer (`startPointerDrag`), and Escape during it
 * puts the window back.
 *
 * Focus: opening moves the focus into the window, and closing gives it back
 * to what had it when the window opened, as long as the focus is still in
 * the window. `activate` does both again when the host shows something new
 * in an open window.
 *
 * It stands in the page's stack of windows (`window-stack.ts`), as the
 * session's windows do: while it is the newest, a press outside every window
 * closes it, and so does Escape, unless the focus is in a field the user
 * types in, or a menu or a dialog of `overlayStore` is open and takes the
 * key (`escapeClosesWindow`). The window is not one of the store's layers: a
 * menu opened beside it does not close it, and it does not close a menu.
 *
 * The title bar holds the `title`, then what the `trailing` slot gives, such
 * as a picture's size, then the close control at the bar's end, as far from
 * its top, bottom and end edges.
 */
const props = defineProps<{
  open: boolean
  /** The window's box, which the page clamps; a move or a resize updates it. */
  rect: WindowRect
  /** The window's accessible name: what it shows. */
  label: string
  /** The layers whose Escape comes first: the page's menus and dialogs. */
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  'update:rect': [rect: WindowRect]
  close: []
}>()

const slots = defineSlots<{
  /** What the window shows; it fills the space under the title bar. */
  default(): unknown
  /** What names the window in its title bar. */
  title(): unknown
  /** What follows the title, before the close control. */
  trailing?(): unknown
}>()

// What opens from inside the window floats above it.
provideLayerElevation()

const { width: pageWidth, height: pageHeight } = useWindowSize()
const page = computed(() => ({ width: pageWidth.value, height: pageHeight.value }))
const shown = computed(() => clampWindowRect(props.rect, page.value))

/** An edge or a corner a drag resizes from, and the cursor it wears. */
interface ResizeZone {
  edges: WindowEdges
  cursor: string
  class: string
}

// Edges straddle the window's border; corners stand over them, larger, so a corner is easy to hit.
const ZONES: ResizeZone[] = [
  { edges: { top: true }, cursor: 'ns-resize', class: 'inset-x-2 -top-1 h-2' },
  { edges: { bottom: true }, cursor: 'ns-resize', class: 'inset-x-2 -bottom-1 h-2' },
  { edges: { left: true }, cursor: 'ew-resize', class: 'inset-y-2 -left-1 w-2' },
  { edges: { right: true }, cursor: 'ew-resize', class: 'inset-y-2 -right-1 w-2' },
  { edges: { top: true, left: true }, cursor: 'nwse-resize', class: '-left-1 -top-1 size-3' },
  { edges: { top: true, right: true }, cursor: 'nesw-resize', class: '-right-1 -top-1 size-3' },
  { edges: { bottom: true, left: true }, cursor: 'nesw-resize', class: '-bottom-1 -left-1 size-3' },
  { edges: { bottom: true, right: true }, cursor: 'nwse-resize', class: '-bottom-1 -right-1 size-3' },
]

/** The window with its resize zones, which hold every press in it. */
const frame = ref<HTMLElement>()
const panel = ref<HTMLElement>()
/** Ends the drag under way early, as its release would. */
let stopDrag: (() => void) | null = null

/** Follows a drag that turns the pointer's offset into a box, from the box at the press. */
function drag(event: PointerEvent, cursor: string, next: (start: WindowRect, dx: number, dy: number) => WindowRect): void {
  if (event.button !== 0 || stopDrag)
    return
  // No text selection starts under the press; the window takes the focus as a click in it would.
  event.preventDefault()
  panel.value?.focus({ preventScroll: true })
  const start = shown.value
  stopDrag = startPointerDrag(event, {
    cursor,
    move(dx, dy) {
      emit('update:rect', next(start, dx, dy))
    },
    end(cancelled) {
      stopDrag = null
      if (cancelled)
        emit('update:rect', start)
    },
  })
}

/** A control in the title bar, such as the close button, takes its own press. */
const OWN_PRESS = 'button, a[href], input, select, textarea, [role=button]'

function startMove(event: PointerEvent): void {
  if (event.target instanceof Element && event.target.closest(OWN_PRESS) !== null)
    return
  drag(event, 'default', (start, dx, dy) => moveWindowRect(start, dx, dy, page.value))
}

function startResize(event: PointerEvent, zone: ResizeZone): void {
  drag(event, zone.cursor, (start, dx, dy) => resizeWindowRect(start, zone.edges, dx, dy, page.value))
}

const stacked = useStackedWindow({
  isOpen: () => props.open,
  element: () => frame.value,
  otherLayerOpen: () => props.overlayStore.hasExclusive(),
  close: () => emit('close'),
})

const focusReturn = useFocusReturn((active) => panel.value?.contains(active) === true)

/**
 * Remembers what has the focus now, to give it back on close, and moves the
 * focus into the window, which comes to the front of the stack.
 */
function activate(): void {
  stacked.raise()
  focusReturn.open()
  panel.value?.focus({ preventScroll: true })
}

// The panel shows on each opening, the first included, which mounts the window open;
// the opener still has the focus then.
watch(panel, (element) => {
  if (element)
    activate()
}, { flush: 'post' })
watch(() => props.open, (open) => {
  if (!open)
    focusReturn.close()
})

onKeyStroke('Escape', (event) => {
  // Escape during a drag never gets here: the drag takes it to put the window back.
  // A newer window in front takes it first.
  if (!props.open || !stacked.isNewest())
    return
  const target = event.target instanceof Element ? event.target : null
  if (!escapeClosesWindow({ defaultPrevented: event.defaultPrevented, isComposing: event.isComposing, target }, props.overlayStore.hasExclusive()))
    return
  event.preventDefault()
  emit('close')
})

onBeforeUnmount(() => {
  stopDrag?.()
  focusReturn.close()
})

defineExpose({ activate })
</script>

<template>
  <Teleport to="body">
    <Transition name="floating-window">
      <div
        v-if="open"
        ref="frame"
        class="fixed z-40"
        :style="{ left: `${shown.x}px`, top: `${shown.y}px`, width: `${shown.width}px`, height: `${shown.height}px` }"
      >
        <!-- The panel is a region (`ui/region.ts`): a menu opened in it grows toward its inside. -->
        <div
          ref="panel"
          data-region
          role="dialog"
          :aria-label="label"
          tabindex="-1"
          class="overlay-dialog flex h-full w-full flex-col overflow-hidden rounded-xl bg-surface-dialog shadow-2xl outline-none"
          :style="{ '--window-bar-h': `${WINDOW_TITLE_BAR_PX}px` }"
        >
          <div
            class="window-bar flex shrink-0 select-none items-center gap-2 border-b border-line-subtle"
            @pointerdown="startMove"
          >
            <div class="min-w-0 flex-1 truncate text-[13px] font-medium text-fg-emphasis">
              <slot name="title" />
            </div>
            <div v-if="slots.trailing" class="shrink-0 text-[12px] tabular-nums text-fg-muted">
              <slot name="trailing" />
            </div>
            <IconButton :icon="X" size="sm" variant="ghost" aria-label="Close" @click="emit('close')" />
          </div>
          <div class="relative min-h-0 flex-1">
            <slot />
          </div>
        </div>
        <div
          v-for="zone in ZONES"
          :key="zone.cursor + Object.keys(zone.edges).join()"
          aria-hidden="true"
          class="absolute touch-none"
          :class="zone.class"
          :style="{ cursor: zone.cursor }"
          @pointerdown="startResize($event, zone)"
        />
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* The close control keeps the same distance to the bar's top, bottom and end; the title starts as far in again. */
.window-bar {
  height: var(--window-bar-h);
  padding-inline-start: calc(var(--window-bar-h) - var(--spacing-hit-sm));
  padding-inline-end: calc((var(--window-bar-h) - var(--spacing-hit-sm)) / 2);
}

.floating-window-enter-active,
.floating-window-leave-active {
  transition: opacity 150ms ease-out, transform 150ms ease-out;
}

.floating-window-enter-from,
.floating-window-leave-to {
  opacity: 0;
  transform: scale(0.97);
}

@media (prefers-reduced-motion: reduce) {
  .floating-window-enter-active,
  .floating-window-leave-active {
    transition: none;
  }
}
</style>

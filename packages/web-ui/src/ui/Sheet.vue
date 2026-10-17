<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, provide, ref, watch } from 'vue'
import { useEventListener } from '@vueuse/core'
import { useEscapeClose } from '../composables/useEscapeClose'
import type { OverlayStore } from '../overlay/overlayStore'
import { useOverlay } from '../composables/useOverlay'
import { useFocusReturn } from '../overlay/focusReturn'
import { createOverlayFamily, overlayFamilyKey, overlayOwnerKey } from '../overlay/overlayFamily'
import { useBackClose } from '../overlay/back-close'
import { dragCloses, dragVelocity } from './drag-close'
import { focusByPage } from './focus-by-page'
import { inSheetKey } from './sheet-context'
import { stackAbove } from '../overlay/stacking'
// A context menu opened in the sheet reads its layer from its element and stands on it (`context-menu.ts`).
import { overlayLayerAttributes } from './context-menu'

/**
 * A bottom sheet, as a phone's own apps open a menu or a short choice: it
 * rises from the screen's bottom edge over a scrim, as wide as the screen and
 * at most 90% of its height, its content scrolling inside it. Every phone
 * sheet is this one, a menu's (Popover) and a page's own, such as the
 * conversation's title sheet. It has no heading of its own: a menu sheet has
 * none, as an iOS menu has none, since what it acts on is the control just
 * tapped, still in view above it; a sheet that is a form or a page carries
 * its title in its content.
 *
 * It closes as iOS's and Android's sheets do: a tap on the scrim; a drag down
 * on its grabber, past 30% of its height or flicked
 * (`dragCloses`), else it springs back; a pull down at the top of its
 * scrolled content, which turns into that same drag; Escape, when a store
 * is given to stand in (`overlayStore`; a menu's Popover stands in it
 * itself); and the browser's Back (`useBackClose`). The focus moves into it
 * as it opens and goes back to its opener as it closes. The page behind
 * takes no touch while it is open: the scrim covers it.
 */
const props = defineProps<{
  open: boolean
  /** The sheet's accessible name: what it acts on, which no heading in it says. */
  label?: string
  /** Where Escape is decided; absent, the sheet's owner handles Escape. */
  overlayStore?: OverlayStore
}>()

const emit = defineEmits<{
  close: []
}>()

defineSlots<{
  default(): unknown
}>()

provide(inSheetKey, true)

const panel = ref<HTMLElement>()
const body = ref<HTMLElement>()

function close(): void {
  if (props.open)
    emit('close')
}

useBackClose(() => props.open, close)

/** The sheet's layer in the store, when it stands in one. */
let layerId: string | null = null
if (props.overlayStore) {
  const store = props.overlayStore
  // The sheet takes Back itself, store or not.
  const id = useOverlay(store, () => props.open, close, 'exclusive', 'popover', undefined, () => false)
  // A menu or dropdown inside the sheet is a layer on it: Escape closes it first, and the sheet stays.
  provide(overlayFamilyKey, createOverlayFamily())
  provide(overlayOwnerKey, id)
  layerId = id
  useEscapeClose(() => props.open && store.isTop(id), close)
}

/** How far the sheet is pulled below its resting place, in px; null while it rests there. */
const offset = ref<number | null>(null)
/** A drag holds the sheet: it follows the finger with no motion of its own. */
const holding = ref(false)

const focusReturn = useFocusReturn((active) => panel.value?.contains(active) === true)
watch(() => props.open, (open) => {
  if (open) {
    // A sheet that opens again starts from its resting place.
    offset.value = null
    focusReturn.open()
    // The content may take the focus itself as it mounts, as a menu does; otherwise the sheet holds it.
    void nextTick(() => {
      if (panel.value && !panel.value.contains(document.activeElement))
        focusByPage(panel.value, { preventScroll: true })
    })
  } else {
    focusReturn.close()
  }
}, { immediate: true, flush: 'post' })

/** The drag under way: where it started, and its recent samples. */
let drag: { startY: number; fromHandle: boolean; samples: { at: number; t: number }[] } | null = null

/** A move this far settles whether a press on the content scrolls it or drags the sheet. */
const SLOP_PX = 6

function startDrag(y: number, t: number, target: EventTarget | null): void {
  const fromHandle = target instanceof Element && target.closest('[data-sheet-handle]') !== null
  drag = { startY: y, fromHandle, samples: [{ at: y, t }] }
}

/** Follows the finger; answers whether the sheet holds the move, which then scrolls nothing. */
function moveDrag(y: number, t: number): boolean {
  if (!drag)
    return false
  const dy = y - drag.startY
  if (!holding.value) {
    if (Math.abs(dy) < SLOP_PX)
      return false
    // The content scrolls a move up, and a pull down while it is not at its top; only then does the sheet take it.
    const atTop = (body.value?.scrollTop ?? 0) <= 0
    if (!drag.fromHandle && !(dy > 0 && atTop)) {
      drag = null
      return false
    }
    holding.value = true
  }
  drag.samples = [...drag.samples.filter((sample) => t - sample.t < 120), { at: y, t }]
  // The sheet follows the finger from where it first touched, the slop included.
  const pulled = y - drag.startY
  // Pulled up past its resting place, the sheet gives a little, as a phone's does.
  offset.value = pulled >= 0 ? pulled : -Math.sqrt(-pulled) * 2
  return true
}

/** The finger lifts: a long or flicked pull closes the sheet from where it stands, a short or still one springs back. */
function endDrag(liftAt: number): void {
  const ended = drag
  const held = holding.value
  drag = null
  holding.value = false
  if (!ended || !held)
    return
  const height = panel.value?.offsetHeight ?? 0
  if (dragCloses(offset.value ?? 0, height, dragVelocity(ended.samples, liftAt))) {
    // The sheet leaves from where the finger left it.
    emit('close')
  } else {
    offset.value = null
  }
}

/** A second finger, or the system taking the touch: the sheet goes back to its place, since only a lift closes it. */
function cancelDrag(): void {
  // A sheet already leaving keeps the place it leaves from.
  if (holding.value)
    offset.value = null
  drag = null
  holding.value = false
}

// A finger drags through touch events, whose moves can be kept from scrolling the content; a mouse or a pen through pointer events.
function onTouchStart(event: TouchEvent): void {
  const touch = event.touches[0]
  if (event.touches.length === 1 && touch)
    startDrag(touch.clientY, event.timeStamp, event.target)
  else
    cancelDrag()
}
// `touches` lists every finger on the screen: one put down anywhere else, such as on the scrim, takes the gesture too.
function onTouchMove(event: TouchEvent): void {
  const touch = event.touches[0]
  if (event.touches.length > 1)
    cancelDrag()
  else if (touch && moveDrag(touch.clientY, event.timeStamp) && event.cancelable)
    event.preventDefault()
}
function onTouchEnd(event: TouchEvent): void {
  // A finger still on the screen means the lift was not the gesture's end.
  if (event.touches.length > 0)
    cancelDrag()
  else
    endDrag(event.timeStamp)
}
/**
 * The sheet starts to leave: it drops the drag's styles, which a leaving node
 * is no longer patched to drop, so it eases down from where it stands.
 */
function onLeaving(element: Element): void {
  element.classList.remove('sheet-dragging')
}
function onPointerDown(event: PointerEvent): void {
  if (event.pointerType === 'touch' || event.button !== 0)
    return
  if (!(event.target instanceof Element) || event.target.closest('[data-sheet-handle]') === null)
    return
  startDrag(event.clientY, event.timeStamp, event.target)
  event.target.setPointerCapture(event.pointerId)
}
function onPointerMove(event: PointerEvent): void {
  if (event.pointerType !== 'touch')
    moveDrag(event.clientY, event.timeStamp)
}
function onPointerUp(event: PointerEvent): void {
  if (event.pointerType !== 'touch')
    endDrag(event.timeStamp)
}

// Touch moves are listened to without `passive`, so that a pull the sheet takes scrolls nothing behind it.
useEventListener(panel, 'touchmove', onTouchMove, { passive: false })

onBeforeUnmount(() => {
  drag = null
  holding.value = false
})

// Above the dialog or menu it opens from, as a popover stands.
const z = stackAbove()
const panelStyle = computed(() => ({
  zIndex: z(),
  ...(offset.value === null ? {} : { '--sheet-offset': `${offset.value}px` }),
}))
</script>

<template>
  <Teleport to="body">
    <Transition name="sheet-scrim">
      <div v-if="open" class="sheet-scrim fixed inset-0 bg-black/40" :style="{ zIndex: z() }" aria-hidden="true" @click="close" />
    </Transition>
    <Transition name="sheet" @before-leave="onLeaving">
      <div
        v-if="open"
        ref="panel"
        role="dialog"
        aria-modal="true"
        :aria-label="label"
        v-bind="layerId ? overlayLayerAttributes(layerId, z()) : {}"
        tabindex="-1"
        class="sheet fixed inset-x-0 bottom-0 flex flex-col rounded-t-2xl bg-surface-dialog text-fg shadow-2xl outline-none"
        :class="{ 'sheet-dragging': holding }"
        :style="panelStyle"
        @touchstart.passive="onTouchStart"
        @touchend="onTouchEnd"
        @touchcancel="cancelDrag"
        @pointerdown="onPointerDown"
        @pointermove="onPointerMove"
        @pointerup="onPointerUp"
        @pointercancel="cancelDrag"
      >
        <div data-sheet-handle class="sheet-handle flex h-5 shrink-0 items-center justify-center">
          <span class="h-1 w-9 rounded-full bg-fg-faint opacity-60" />
        </div>
        <div ref="body" class="sheet-body min-h-0 flex-1 overflow-y-auto overscroll-contain">
          <slot />
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
.sheet {
  /* 90% of the visible area, the keyboard's part left out (`phone-page.ts`). */
  max-height: 90%;
  padding-bottom: var(--safe-bottom);
  transform: translateY(var(--sheet-offset, 0px));
  /* A short spring, as a phone's sheet rises and falls. */
  transition: transform 340ms cubic-bezier(0.32, 0.72, 0, 1);
}

.sheet-handle {
  touch-action: none;
}

.sheet.sheet-dragging {
  transition: none;
}

.sheet-enter-from,
.sheet-leave-to {
  transform: translateY(100%) !important;
}

.sheet-scrim {
  touch-action: none;
}

.sheet-scrim-enter-active,
.sheet-scrim-leave-active {
  transition: opacity 340ms cubic-bezier(0.32, 0.72, 0, 1);
}

.sheet-scrim-enter-from,
.sheet-scrim-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .sheet,
  .sheet-scrim-enter-active,
  .sheet-scrim-leave-active {
    transition: none;
  }
}
</style>

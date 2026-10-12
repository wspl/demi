<script lang="ts">
import type { InjectionKey } from 'vue'

/** How a tip tells the tip around it that something inside is showing. One key for every instance. */
const tooltipNestKey: InjectionKey<{ showing(delta: 1 | -1): void }> = Symbol('tooltip-nest')

/**
 * The one tip with a warm window (`warmMs`) that shows at a time, and when
 * the last one closed: moving from one picture to the next shows the next
 * one's card at once, as moving along a toolbar does on macOS.
 */
let warmOpen: { close(): void } | null = null
let warmClosedAt = Number.NEGATIVE_INFINITY

/** Where the pointer last moved on the page; null before it moved. */
let lastMove: { x: number; y: number } | null = null
let following = false

/**
 * Follows the pointer over the whole page, for every tip: one listener,
 * added with the first tip and kept for the page's lifetime, since tips come
 * and go with every view.
 */
function followPointer(): void {
  if (following || typeof window === 'undefined')
    return
  following = true
  window.addEventListener('pointermove', (event) => {
    lastMove = { x: event.clientX, y: event.clientY }
  }, { capture: true, passive: true })
}

/**
 * Whether a pointer entering an element at (`x`, `y`) did not move there:
 * the element came under it. A pointer that moves in enters before the page
 * sees that move, so its last move lies elsewhere.
 */
function pointerRestsAt(x: number, y: number): boolean {
  return lastMove !== null && lastMove.x === x && lastMove.y === y
}
</script>

<script setup lang="ts">
import { computed, inject, onBeforeUnmount, provide, ref, useAttrs, useSlots, watch } from 'vue'
import {
  useFloating,
  offset as offsetMiddleware,
  flip,
  shift,
  limitShift,
  autoUpdate,
  getOverflowAncestors
} from '@floating-ui/vue'
import type { Middleware, Placement } from '@floating-ui/vue'
import { injectTooltipPlacement } from './tooltip-placement'
import { onClickOutside } from '@vueuse/core'
import { appOverlayStore } from '../overlay/appOverlay'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayFamilyKey } from '../overlay/overlayFamily'
import { useLayerElevation } from '../overlay/layerElevation'
import { useOverlay } from '../composables/useOverlay'
import type { SentenceText } from './ui-text'

/**
 * Tooltip copy, one style everywhere:
 * - An action: the verb alone, sentence case, no article, no period. The
 *   object is left out when the control sits on the thing it acts on (a row's
 *   Archive, a message's Copy, a model's Edit), and named only when the control
 *   is away from it or would be ambiguous (Add project, New folder, Stop all
 *   agents). A toggle names the action it will take, not the state (Unpin,
 *   Hide hidden files).
 * - The control's aria-label is the same text; when the tooltip follows state
 *   (Copied), the label follows with it.
 * - A reason (why a control is disabled or unavailable) is a full sentence and
 *   may be long: "Fork is available after this message completes."
 *
 * A tip shows as the pointer rests on its trigger, after a delay, and only
 * once the pointer moved there: a trigger that slides under a pointer at rest,
 * as a new tab does under the New tab control just clicked, shows none. A
 * press on the trigger hides the tip until the pointer leaves, so a click
 * never brings one. Keyboard focus shows it as well, but focus a click gave
 * does not. A touch shows none.
 */
defineOptions({
  inheritAttrs: false,
})

const props = withDefaults(defineProps<{
  content?: SentenceText
  placement?: Placement
  offset?: number
  disabled?: boolean
  /**
   * Asked of the trigger each time the tip is about to show: false keeps it
   * hidden this time. For a tip whose need is measured as the pointer
   * arrives, such as text that may or may not be cut.
   */
  showIf?: (trigger: HTMLElement) => boolean
  /**
   * The `overlay` slot is a card of its own, such as a picture's preview:
   * the tip adds only its surface, with a larger radius, and the slot brings
   * its own padding, size and text.
   */
  card?: boolean
  /**
   * How long after a tip with a warm window closed another one opens
   * without its delay; a tip with one also closes the one still showing.
   * For a run of cards the pointer moves along, such as a message's pictures.
   */
  warmMs?: number
  /**
   * Chooses the side the tip opens on, each time it is positioned, from the
   * trigger, the tip itself and the side it is on now; the tip then never
   * flips by itself, since the chooser knows the room better. For a card
   * that is sized to the room it has on a side.
   */
  placeBy?: (context: { reference: HTMLElement; floating: HTMLElement; placement: Placement }) => Placement
  /** A tip of the `overlay` slot that has room for a line of facts, such as what an agent runs on, not a few sentences. */
  wide?: boolean
  openDelayMs?: number
  closeDelayMs?: number
  tag?: 'span' | 'div'
  overlayStore?: OverlayStore
  /**
   * An element the page did not render through a component, such as a link
   * in a rendered message, that the tip describes in place of its slot: the
   * tip follows the pointer and the focus on it and stands by it.
   */
  trigger?: HTMLElement | null
}>(), {
  offset: 8,
  disabled: false,
  openDelayMs: 120,
  closeDelayMs: 0,
  tag: 'span',
})

const emit = defineEmits<{
  /** The tip shows. */
  open: []
  /** The tip went, however it did. */
  close: []
}>()

followPointer()

const triggerRef = ref<HTMLElement | null>(null)
const floatingRef = ref<HTMLElement | null>(null)
const isOpen = ref(false)
const hiddenByScroll = ref(false)
const attrs = useAttrs()
const slots = useSlots()
const overlayStore = computed(() => props.overlayStore ?? appOverlayStore)
// A trigger inside an exclusive panel (a menu row) may still hint; triggers outside yield to it.
const family = inject(overlayFamilyKey, null)
const elevation = useLayerElevation()
const blockedByExclusive = computed(() => family == null && overlayStore.value.hasExclusive())
const hasOverlay = computed(() => !!slots.overlay)
const hasContent = computed(() => !!props.content?.trim() || hasOverlay.value)
const canShow = computed(() => hasContent.value && !props.disabled && !blockedByExclusive.value)
// One thing under the pointer gets one tip. A tip inside this one says the more
// specific thing (why a control is disabled, under the tip that names it), so
// while an inner one shows, this one gives way.
const innerShowing = ref(0)
const outer = inject(tooltipNestKey, null)
provide(tooltipNestKey, {
  showing(delta) {
    innerShowing.value += delta
  },
})
const visible = computed(
  () => isOpen.value && canShow.value && !hiddenByScroll.value && innerShowing.value === 0,
)
// To the tip around this one, a tip showing anywhere inside counts the same.
const occupied = computed(() => visible.value || innerShowing.value > 0)
watch(occupied, (now) => outer?.showing(now ? 1 : -1))
onBeforeUnmount(() => {
  if (occupied.value) {
    outer?.showing(-1)
  }
})

/** The side the tip opens on when it names none, as the component around it says. */
const surroundingPlacement = injectTooltipPlacement()
/** Hands the side to `placeBy`, each time the tip is positioned. */
const placer: Middleware = {
  name: 'placeBy',
  fn({ placement, elements }) {
    const reference = triggerRef.value
    const floating = elements.floating
    if (!props.placeBy || !reference)
      return {}
    const next = props.placeBy({ reference, floating, placement })
    return next === placement ? {} : { reset: { placement: next } }
  },
}
const { floatingStyles } = useFloating(triggerRef, floatingRef, {
  placement: computed(() => props.placement ?? surroundingPlacement),
  strategy: 'fixed',
  middleware: computed(() => [
    ...(props.placeBy ? [placer] : []),
    offsetMiddleware(props.offset),
    ...(props.placeBy ? [] : [flip()]),
    shift({ padding: 8, limiter: limitShift() }),
  ]),
  whileElementsMounted: autoUpdate,
  transform: false,
})

/** How long a tip that was asked for, rather than pointed at, stays. */
const ASKED_MS = 3000

let openTimer: ReturnType<typeof setTimeout> | null = null
let closeTimer: ReturnType<typeof setTimeout> | null = null
let askedTimer: ReturnType<typeof setTimeout> | null = null
let scrollTargets: EventTarget[] = []
let stopClickOutside: (() => void) | undefined

function onAnchorScroll() {
  hiddenByScroll.value = true
  closeNow()
}

/** Escape dismissed the tip: none again until the pointer or the focus leaves its trigger. */
let dismissed = false

/** Escape closes a showing tip, and goes on to whatever else it closes: the tip never takes the key. */
function onEscape(event: KeyboardEvent) {
  if (event.key !== 'Escape')
    return
  dismissed = true
  clearTimers()
  closeNow()
}

function unbindDismiss() {
  document.removeEventListener('keydown', onEscape, { capture: true })
  for (const target of scrollTargets) {
    target.removeEventListener('scroll', onAnchorScroll)
  }
  scrollTargets = []
  stopClickOutside?.()
  stopClickOutside = undefined
}

// Listeners exist only while the tip is showing: dozens of tooltips sit idle on a screen.
function bindDismiss() {
  unbindDismiss()
  const el = triggerRef.value
  if (!el)
    return
  document.addEventListener('keydown', onEscape, { capture: true })
  scrollTargets = getOverflowAncestors(el)
  for (const target of scrollTargets) {
    target.addEventListener('scroll', onAnchorScroll, { passive: true })
  }
  stopClickOutside = onClickOutside(triggerRef, () => {
    clearTimers()
    closeNow()
  })
}

function clearOpenTimer() {
  if (!openTimer)
    return
  clearTimeout(openTimer)
  openTimer = null
}

function clearCloseTimer() {
  if (!closeTimer)
    return
  clearTimeout(closeTimer)
  closeTimer = null
}

function clearAskedTimer() {
  if (!askedTimer)
    return
  clearTimeout(askedTimer)
  askedTimer = null
}

function clearTimers() {
  clearOpenTimer()
  clearCloseTimer()
  clearAskedTimer()
}

/** Whether the tip may show now: its standing conditions, then the trigger's answer. */
function wanted(): boolean {
  if (!canShow.value)
    return false
  const trigger = triggerRef.value
  return !props.showIf || (trigger != null && props.showIf(trigger))
}

function openNow() {
  if (!wanted())
    return
  hiddenByScroll.value = false
  isOpen.value = true
}

const warming = { close: closeNow }
// Of the tips with a warm window, the one that shows closes the one that did.
watch(visible, (now) => {
  if (now)
    emit('open')
  else emit('close')
  if (props.warmMs === undefined)
    return
  if (now) {
    if (warmOpen && warmOpen !== warming)
      warmOpen.close()
    warmOpen = warming
  } else if (warmOpen === warming) {
    warmOpen = null
    warmClosedAt = performance.now()
  }
})

function closeNow() {
  isOpen.value = false
}

function scheduleOpen() {
  clearCloseTimer()
  clearOpenTimer()
  if (dismissed || !wanted())
    return
  hiddenByScroll.value = false
  const warm = props.warmMs !== undefined && (warmOpen !== null || performance.now() - warmClosedAt < props.warmMs)
  if (props.openDelayMs <= 0 || warm) {
    openNow()
    return
  }
  openTimer = setTimeout(() => {
    openTimer = null
    openNow()
  }, props.openDelayMs)
}

/** Where the pointer entered the trigger without moving, as when the trigger slid under it; null otherwise. */
let enteredAtRest: { x: number; y: number } | null = null
/** The trigger was pressed: no tip until the pointer leaves it. */
let pressed = false

function onPointerenter(event: PointerEvent) {
  if (pressed || event.pointerType === 'touch')
    return
  if (pointerRestsAt(event.clientX, event.clientY)) {
    enteredAtRest = { x: event.clientX, y: event.clientY }
    return
  }
  scheduleOpen()
}

function onPointermove(event: PointerEvent) {
  if (pressed || !enteredAtRest)
    return
  if (enteredAtRest.x === event.clientX && enteredAtRest.y === event.clientY)
    return
  enteredAtRest = null
  scheduleOpen()
}

function onPointerleave() {
  enteredAtRest = null
  pressed = false
  scheduleClose()
}

function onPointerdown() {
  pressed = true
  clearTimers()
  closeNow()
}

function onFocusin(event: FocusEvent) {
  const target = event.target
  if (pressed || !(target instanceof Element) || !target.matches(':focus-visible'))
    return
  scheduleOpen()
}

function scheduleClose() {
  dismissed = false
  clearOpenTimer()
  clearCloseTimer()
  if (props.closeDelayMs <= 0) {
    closeNow()
    return
  }
  closeTimer = setTimeout(() => {
    closeTimer = null
    closeNow()
  }, props.closeDelayMs)
}

watch(isOpen, (open) => {
  if (open)
    bindDismiss()
  else unbindDismiss()
})

watch(canShow, (nextCanShow) => {
  if (nextCanShow)
    return
  clearTimers()
  closeNow()
})

useOverlay(overlayStore.value, isOpen, closeNow, 'hint')

defineExpose({
  /**
   * Shows the tip without the pointer, to answer something the user just
   * tried — Enter on a send that cannot send. It goes by itself, and sooner
   * if anything that dismisses a tip happens first.
   */
  show(): void {
    clearTimers()
    openNow()
    if (isOpen.value) {
      askedTimer = setTimeout(() => {
        askedTimer = null
        closeNow()
      }, ASKED_MS)
    }
  },
})

/** Gives a `trigger` element the listeners the slot's wrapper has in the template, or takes them away. */
function listen(element: HTMLElement, on: boolean): void {
  if (on) {
    element.addEventListener('pointerenter', onPointerenter)
    element.addEventListener('pointermove', onPointermove)
    element.addEventListener('pointerleave', onPointerleave)
    element.addEventListener('pointerdown', onPointerdown, { capture: true })
    element.addEventListener('focusin', onFocusin)
    element.addEventListener('focusout', scheduleClose)
    return
  }
  element.removeEventListener('pointerenter', onPointerenter)
  element.removeEventListener('pointermove', onPointermove)
  element.removeEventListener('pointerleave', onPointerleave)
  element.removeEventListener('pointerdown', onPointerdown, { capture: true })
  element.removeEventListener('focusin', onFocusin)
  element.removeEventListener('focusout', scheduleClose)
}

watch(() => props.trigger, (element, before) => {
  if (before)
    listen(before, false)
  if (element) {
    listen(element, true)
    triggerRef.value = element
  }
}, { immediate: true })

onBeforeUnmount(() => {
  clearTimers()
  unbindDismiss()
  // A tip that goes with its page closes as any does, for the warm window and for the owner's `close`.
  if (visible.value && warmOpen === warming) {
    warmOpen = null
    warmClosedAt = performance.now()
  }
  if (props.trigger)
    listen(props.trigger, false)
})
</script>

<template>
  <component
    :is="props.tag"
    v-if="!props.trigger"
    ref="triggerRef"
    v-bind="attrs"
    @pointerenter="onPointerenter"
    @pointermove="onPointermove"
    @pointerleave="onPointerleave"
    @pointerdown.capture="onPointerdown"
    @focusin="onFocusin"
    @focusout="scheduleClose"
  >
    <slot />
  </component>

  <Teleport to="body">
    <Transition
      enter-active-class="transition-[opacity,scale] duration-120 ease-out"
      leave-active-class="transition-[opacity,scale] duration-100 ease-out"
      enter-from-class="opacity-0 scale-95"
      leave-to-class="opacity-0 scale-95"
    >
      <!-- A hint describes whatever is under the pointer, so it sits above
           dialogs, popovers and toasts, including hints opened from those surfaces. -->
      <div
        v-if="visible"
        ref="floatingRef"
        class="overlay-shell select-none pointer-events-none z-[60] w-max min-w-max text-fg"
        :class="card
          ? 'rounded-lg'
          : [
              'rounded-md',
              hasOverlay
                ? [wide ? 'max-w-2xl' : 'max-w-xs', 'px-3 py-2 text-xs leading-relaxed']
                : 'line-clamp-2 max-w-sm px-2.5 py-1.5 text-[12px] leading-4',
            ]"
        :style="{ ...floatingStyles, '--elevation': elevation }"
        role="tooltip"
      >
        <slot v-if="hasOverlay" name="overlay" />
        <template v-else>{{ props.content }}</template>
      </div>
    </Transition>
  </Teleport>
</template>

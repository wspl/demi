<script setup lang="ts">
import { computed, inject, provide, ref, watch } from 'vue'
import {
  useFloating,
  offset as offsetMiddleware,
  flip,
  shift,
  limitShift,
  size,
  autoUpdate
} from '@floating-ui/vue'
import type { Alignment, Placement } from '@floating-ui/vue'
import { onClickOutside, onKeyStroke } from '@vueuse/core'
import type { OverlayStore } from '../overlay/overlayStore'
import { useOverlayTarget } from '../overlay/overlayContainer'
import { createOverlayFamily, overlayFamilyKey } from '../overlay/overlayFamily'
import { useFocusReturn } from '../overlay/focusReturn'
import { useOverlay } from '../composables/useOverlay'
import { inwardAlignment, regionSpan } from './region'

const props = withDefaults(defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** Live trigger. When set, the panel follows this rect. */
  anchorEl?: HTMLElement | null
  anchorInset?: number
  /** Viewport point at open (context menu). Follows `anchorContextEl` while open. */
  anchorX?: number
  anchorY?: number
  anchorWidth?: number
  anchorHeight?: number
  /** Overflow-ancestor root for a point anchor so autoUpdate tracks scroll. */
  anchorContextEl?: HTMLElement | null
  /** Whether the panel opens below its anchor or above it; it flips where it would not fit. */
  side?: 'top' | 'bottom'
  /**
   * A placement the inward rule does not cover, such as a submenu's beside
   * its item. The caller says why at the call; without one the panel grows
   * toward the inside of its trigger's region.
   */
  placement?: Placement
  offset?: number
  shiftPadding?: number
  ignoreEls?: HTMLElement[]
  /** Skip enter/leave motion, e.g. when one submenu replaces a sibling. */
  instant?: boolean
}>(), {
  anchorInset: 0,
  anchorX: 0,
  anchorY: 0,
  anchorWidth: 0,
  anchorHeight: 0,
  side: 'bottom',
  offset: 6,
  shiftPadding: 8,
})

const emit = defineEmits<{
  close: []
}>()

const floatingRef = ref<HTMLElement | null>(null)
const pointOffsetX = ref(0)
const pointOffsetY = ref(0)

function clientRect(x: number, y: number, width: number, height: number) {
  return {
    x,
    y,
    width,
    height,
    top: y,
    left: x,
    right: x + width,
    bottom: y + height,
  }
}

function snapshotPointOffset() {
  const context = props.anchorContextEl
  if (context) {
    const rect = context.getBoundingClientRect()
    pointOffsetX.value = props.anchorX - rect.left
    pointOffsetY.value = props.anchorY - rect.top
    return
  }
  pointOffsetX.value = props.anchorX
  pointOffsetY.value = props.anchorY
}

watch(
  () =>
    [props.isOpen, props.anchorX, props.anchorY, props.anchorContextEl] as const,
  ([open]) => {
    if (!open || props.anchorEl)
      return
    snapshotPointOffset()
  },
  { immediate: true },
)

const virtualRef = computed(() => {
  const el = props.anchorEl
  const inset = props.anchorInset
  if (el) {
    return {
      contextElement: el,
      getBoundingClientRect: () => {
        const rect = el.getBoundingClientRect()
        return clientRect(
          rect.left + inset,
          rect.top,
          Math.max(0, rect.width - inset * 2),
          rect.height,
        )
      },
    }
  }
  const context = props.anchorContextEl
  const offsetX = pointOffsetX.value
  const offsetY = pointOffsetY.value
  const width = props.anchorWidth
  const height = props.anchorHeight
  if (context) {
    return {
      contextElement: context,
      getBoundingClientRect: () => {
        const rect = context.getBoundingClientRect()
        return clientRect(rect.left + offsetX, rect.top + offsetY, width, height)
      },
    }
  }
  return {
    getBoundingClientRect: () => clientRect(props.anchorX, props.anchorY, width, height),
  }
})

/**
 * The trigger's edge the panel lines up with, decided each time it opens
 * from a trigger (a new trigger while open is a new opening): a trigger in
 * the end half of its region gets a panel that grows back across the region.
 */
const alignment = ref<Alignment>('start')

watch(
  () => [props.isOpen, props.anchorEl] as const,
  ([open, el]) => {
    if (!open || !el)
      return
    const rect = virtualRef.value.getBoundingClientRect()
    alignment.value = inwardAlignment({ left: rect.left, width: rect.width }, regionSpan(el))
  },
  { immediate: true },
)

// A menu at the pointer has no trigger to grow inward from: it opens toward the pointer's lower
// right, as macOS's context menus do, and flips where it would not fit.
const placement = computed<Placement>(() => (
  props.placement ?? `${props.side}-${props.anchorEl ? alignment.value : 'start'}`
))

// A panel confined to a host container never owns the page, so it is not exclusive, and the
// container stands in for the viewport: the panel stays inside it the way it stays on screen.
const { container, to: teleportTarget, ready } = useOverlayTarget()
const boundary = computed(() => container?.value ?? undefined)

// Flip and size must agree on the edge inset: a smaller flip inset lets a panel that
// almost fits be shrunk by size instead of flipped, and the last item gets cut off.
const EDGE_PADDING = 16

const { floatingStyles, placement: resolvedPlacement } = useFloating(virtualRef, floatingRef, {
  placement,
  strategy: 'fixed',
  middleware: computed(() => [
    offsetMiddleware(props.offset),
    flip({ padding: EDGE_PADDING, boundary: boundary.value }),
    // Keep the panel on-screen, but stop following once the trigger scrolls away.
    shift(
      {
        padding: props.shiftPadding,
        limiter: limitShift(),
        boundary: boundary.value
      }
    ),
    size({
      padding: EDGE_PADDING,
      boundary: boundary.value,
      apply({ availableHeight, elements }) {
        elements.floating.style.setProperty(
          '--overlay-available-height',
          `${Math.max(0, Math.floor(availableHeight))}px`,
        )
      },
    }),
  ]),
  whileElementsMounted: autoUpdate,
  transform: false,
})

const transformOrigin = computed(() => {
  const p = resolvedPlacement.value
  const y = p.startsWith('top') ? 'bottom' : 'top'
  const x = p.endsWith('start')
    ? 'left'
    : p.endsWith('end')
    ? 'right'
    : 'center'
  return `${y} ${x}`
})

const inheritedFamily = inject(overlayFamilyKey, null)
const family = inheritedFamily ?? createOverlayFamily()
const nested = inheritedFamily != null
provide(overlayFamilyKey, family)


watch(floatingRef, (el, _prev, onCleanup) => {
  if (!el)
    return
  onCleanup(family.register(el))
})

/**
 * A panel that held the focus gives it back to its opener when it closes, by
 * a choice, Escape or a click outside on nothing that takes the focus
 * (`useFocusReturn`); a panel that never held it, such as a hover card, gives
 * none back. A submenu gives the keys back to its own menu (MenuItem).
 */
const focusReturn = useFocusReturn(inFamily)
let heldFocus = false

function inFamily(el: EventTarget | null): boolean {
  return el instanceof Node && family.panels.some((panel) => panel.contains(el))
}

function panelFocusIn(): void {
  heldFocus = true
}

function panelFocusOut(event: FocusEvent): void {
  // Focus that moved to a control outside the tree stays there; into a submenu or to nothing, it was still the panel's.
  if (event.relatedTarget !== null && !inFamily(event.relatedTarget))
    heldFocus = false
}

// Immediate: a panel can mount already open, as a context menu keyed to each opening does.
watch(() => props.isOpen, (open) => {
  if (open) {
    focusReturn.open()
  } else {
    focusReturn.close(!nested && heldFocus)
  }
  heldFocus = false
}, { flush: 'sync', immediate: true })

onClickOutside(floatingRef, () => {
  if (props.isOpen)
    emit('close')
}, {
  ignore: () => [
    ...(props.ignoreEls ?? []),
    ...family.panels,
  ],
})

const overlayId = useOverlay(props.overlayStore, () => (
  nested || container ? false : props.isOpen
), () => emit('close'))

/**
 * Escape closes the panel, as it closes a menu or popover on macOS: the
 * innermost one the keys are in, and, with the keys elsewhere on the page,
 * the one on top. An Escape something inside used first, such as a filter
 * it cleared or a type-select it ended (it called preventDefault), closes
 * nothing, and one this panel takes reaches no dialog behind it.
 */
function closeOnEscape(event: KeyboardEvent): void {
  if (!props.isOpen || event.key !== 'Escape' || event.defaultPrevented)
    return
  event.preventDefault()
  emit('close')
}

onKeyStroke('Escape', (event) => {
  if (!nested && !container && props.overlayStore.isTop(overlayId))
    closeOnEscape(event)
})

const overlayMotion = {
  enterActiveClass: 'transition-[opacity,scale] duration-150 ease-out',
  leaveActiveClass: 'transition-[opacity,scale] duration-150 ease-out',
  enterFromClass: 'opacity-0 scale-95',
  leaveToClass: 'opacity-0 scale-95',
}
</script>

<template>
  <Teleport v-if="ready" :to="teleportTarget">
    <Transition v-bind="overlayMotion" :css="!instant">
      <div
        v-if="isOpen"
        ref="floatingRef"
        data-overlay-panel
        class="popover-floating z-50 w-max"
        :style="{ ...floatingStyles, transformOrigin: transformOrigin }"
        @keydown="closeOnEscape"
        @focusin="panelFocusIn"
        @focusout="panelFocusOut"
      >
        <slot />
      </div>
    </Transition>
  </Teleport>
</template>

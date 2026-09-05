<script setup lang="ts">
import { computed, inject, provide, ref, watch } from 'vue'
import { useFloating, offset as offsetMiddleware, flip, shift, limitShift, size, autoUpdate } from '@floating-ui/vue'
import type { Placement } from '@floating-ui/vue'
import { onClickOutside } from '@vueuse/core'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayContainerKey } from '../overlay/overlayContainer'
import { createOverlayFamily, overlayFamilyKey } from '../overlay/overlayFamily'
import { useOverlay } from '../composables/useOverlay'

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
  placement?: Placement
  offset?: number
  shiftPadding?: number
  ignoreEls?: HTMLElement[]
}>(), {
  anchorInset: 0,
  anchorX: 0,
  anchorY: 0,
  anchorWidth: 0,
  anchorHeight: 0,
  placement: 'bottom-start',
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
  () => [props.isOpen, props.anchorX, props.anchorY, props.anchorContextEl] as const,
  ([open]) => {
    if (!open || props.anchorEl) return
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

const { floatingStyles, placement: resolvedPlacement } = useFloating(virtualRef, floatingRef, {
  placement: computed(() => props.placement),
  strategy: 'fixed',
  middleware: computed(() => [
    offsetMiddleware(props.offset),
    flip(),
    // Keep the panel on-screen, but stop following once the trigger scrolls away.
    shift({ padding: props.shiftPadding, limiter: limitShift() }),
    size({
      padding: 16,
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
  const x = p.endsWith('start') ? 'left' : p.endsWith('end') ? 'right' : 'center'
  return `${y} ${x}`
})

const inheritedFamily = inject(overlayFamilyKey, null)
const family = inheritedFamily ?? createOverlayFamily()
const nested = inheritedFamily != null
provide(overlayFamilyKey, family)

// A panel confined to a host container never owns the page, so it is not exclusive.
const container = inject(overlayContainerKey, null)
const teleportTarget = computed(() => container?.value ?? 'body')

watch(floatingRef, (el, _prev, onCleanup) => {
  if (!el) return
  onCleanup(family.register(el))
})

onClickOutside(floatingRef, () => {
  if (props.isOpen) emit('close')
}, {
  ignore: () => [
    ...(props.ignoreEls ?? []),
    ...family.panels,
  ],
})

useOverlay(props.overlayStore, () => (
  nested || container ? false : props.isOpen
), () => emit('close'))

const overlayMotion = {
  enterActiveClass: 'transition-[opacity,scale] duration-150 ease-out',
  leaveActiveClass: 'transition-[opacity,scale] duration-150 ease-out',
  enterFromClass: 'opacity-0 scale-95',
  leaveToClass: 'opacity-0 scale-95',
}
</script>

<template>
  <Teleport :to="teleportTarget">
    <Transition v-bind="overlayMotion">
      <div
        v-if="isOpen"
        ref="floatingRef"
        data-overlay-panel
        class="popover-floating z-50 w-max"
        :style="{ ...floatingStyles, transformOrigin: transformOrigin }"
      >
        <slot />
      </div>
    </Transition>
  </Teleport>
</template>

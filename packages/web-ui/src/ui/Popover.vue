<script setup lang="ts">
import { computed, ref } from 'vue'
import { useFloating, offset as offsetMiddleware, flip, shift, autoUpdate } from '@floating-ui/vue'
import type { Placement } from '@floating-ui/vue'
import { onClickOutside } from '@vueuse/core'
import type { OverlayStore } from '../overlay/overlayStore'
import { useOverlay } from '../composables/useOverlay'

const props = withDefaults(defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  anchorX: number
  anchorY: number
  anchorWidth?: number
  anchorHeight?: number
  placement?: Placement
  offset?: number
  shiftPadding?: number
  ignoreEls?: HTMLElement[]
}>(), {
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

const virtualRef = computed(() => ({
  getBoundingClientRect: () => ({
    x: props.anchorX,
    y: props.anchorY,
    width: props.anchorWidth,
    height: props.anchorHeight,
    top: props.anchorY,
    left: props.anchorX,
    right: props.anchorX + props.anchorWidth,
    bottom: props.anchorY + props.anchorHeight,
  }),
}))

const { floatingStyles, placement: resolvedPlacement } = useFloating(virtualRef, floatingRef, {
  placement: computed(() => props.placement),
  middleware: computed(() => [
    offsetMiddleware(props.offset),
    flip(),
    shift({ padding: props.shiftPadding }),
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

onClickOutside(floatingRef, () => {
  if (props.isOpen) emit('close')
}, { ignore: computed(() => props.ignoreEls ?? []) })

useOverlay(props.overlayStore, () => props.isOpen, () => emit('close'))
</script>

<template>
  <Teleport to="body">
    <Transition
      enter-active-class="transition-[opacity,transform] duration-150 ease-out"
      leave-active-class="transition-[opacity,transform] duration-150 ease-out"
      enter-from-class="opacity-0 scale-95"
      leave-to-class="opacity-0 scale-95"
    >
      <div
        v-if="isOpen"
        ref="floatingRef"
        class="popover-floating z-50"
        :style="{ ...floatingStyles, transformOrigin: transformOrigin }"
      >
        <slot />
      </div>
    </Transition>
  </Teleport>
</template>

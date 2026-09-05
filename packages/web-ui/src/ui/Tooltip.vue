<script setup lang="ts">
import { computed, inject, onBeforeUnmount, ref, useAttrs, useSlots, watch } from 'vue'
import { useFloating, offset as offsetMiddleware, flip, shift, limitShift, autoUpdate, getOverflowAncestors } from '@floating-ui/vue'
import type { Placement } from '@floating-ui/vue'
import { onClickOutside } from '@vueuse/core'
import { appOverlayStore } from '../overlay/appOverlay'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayFamilyKey } from '../overlay/overlayFamily'
import { useOverlay } from '../composables/useOverlay'

defineOptions({
  inheritAttrs: false,
})

const props = withDefaults(defineProps<{
  content?: string
  placement?: Placement
  offset?: number
  disabled?: boolean
  openDelayMs?: number
  closeDelayMs?: number
  tag?: 'span' | 'div'
  overlayStore?: OverlayStore
}>(), {
  placement: 'top',
  offset: 8,
  disabled: false,
  openDelayMs: 120,
  closeDelayMs: 0,
  tag: 'span',
})

const triggerRef = ref<HTMLElement | null>(null)
const floatingRef = ref<HTMLElement | null>(null)
const isOpen = ref(false)
const hiddenByScroll = ref(false)
const attrs = useAttrs()
const slots = useSlots()
const overlayStore = computed(() => props.overlayStore ?? appOverlayStore)
// A trigger inside an exclusive panel (a menu row) may still hint; triggers outside yield to it.
const family = inject(overlayFamilyKey, null)
const blockedByExclusive = computed(() => family == null && overlayStore.value.hasExclusive())
const hasOverlay = computed(() => !!slots.overlay)
const hasContent = computed(() => !!props.content?.trim() || hasOverlay.value)
const canShow = computed(() => hasContent.value && !props.disabled && !blockedByExclusive.value)
const visible = computed(() => isOpen.value && canShow.value && !hiddenByScroll.value)

const { floatingStyles } = useFloating(triggerRef, floatingRef, {
  placement: computed(() => props.placement),
  strategy: 'fixed',
  middleware: computed(() => [
    offsetMiddleware(props.offset),
    flip(),
    shift({ padding: 8, limiter: limitShift() }),
  ]),
  whileElementsMounted: autoUpdate,
  transform: false,
})

let openTimer: ReturnType<typeof setTimeout> | null = null
let closeTimer: ReturnType<typeof setTimeout> | null = null
let scrollTargets: EventTarget[] = []
let stopClickOutside: (() => void) | undefined

function onAnchorScroll() {
  hiddenByScroll.value = true
  closeNow()
}

function unbindDismiss() {
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
  if (!el) return
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
  if (!openTimer) return
  clearTimeout(openTimer)
  openTimer = null
}

function clearCloseTimer() {
  if (!closeTimer) return
  clearTimeout(closeTimer)
  closeTimer = null
}

function clearTimers() {
  clearOpenTimer()
  clearCloseTimer()
}

function openNow() {
  if (!canShow.value) return
  hiddenByScroll.value = false
  isOpen.value = true
}

function closeNow() {
  isOpen.value = false
}

function scheduleOpen() {
  clearCloseTimer()
  clearOpenTimer()
  if (!canShow.value) return
  hiddenByScroll.value = false
  if (props.openDelayMs <= 0) {
    openNow()
    return
  }
  openTimer = setTimeout(() => {
    openTimer = null
    openNow()
  }, props.openDelayMs)
}

function scheduleClose() {
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
  if (open) bindDismiss()
  else unbindDismiss()
})

watch(canShow, (nextCanShow) => {
  if (nextCanShow) return
  clearTimers()
  closeNow()
})

useOverlay(overlayStore.value, isOpen, closeNow, 'hint')

onBeforeUnmount(() => {
  clearTimers()
  unbindDismiss()
})
</script>

<template>
  <component
    :is="props.tag"
    ref="triggerRef"
    v-bind="attrs"
    @mouseenter="scheduleOpen"
    @mouseleave="scheduleClose"
    @focusin="scheduleOpen"
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
      <div
        v-if="visible"
        ref="floatingRef"
        class="overlay-shell select-none pointer-events-none z-40 w-max min-w-max rounded-md text-fg"
        :class="hasOverlay
          ? 'max-w-xs px-3 py-2 text-xs leading-relaxed'
          : 'line-clamp-2 max-w-sm px-2.5 py-1.5 text-[12px] leading-4'"
        :style="floatingStyles"
        role="tooltip"
      >
        <slot v-if="hasOverlay" name="overlay" />
        <template v-else>{{ props.content }}</template>
      </div>
    </Transition>
  </Teleport>
</template>

<script lang="ts">
import type { InjectionKey } from 'vue'

/** How a tip tells the tip around it that something inside is showing. One key for every instance. */
const tooltipNestKey: InjectionKey<{ showing(delta: 1 | -1): void }> = Symbol('tooltip-nest')
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
import type { Placement } from '@floating-ui/vue'
import { onClickOutside } from '@vueuse/core'
import { appOverlayStore } from '../overlay/appOverlay'
import type { OverlayStore } from '../overlay/overlayStore'
import { overlayFamilyKey } from '../overlay/overlayFamily'
import { useOverlay } from '../composables/useOverlay'

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
 */
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
  /**
   * The overlay is a picture, not text: the tip frames it with the same space
   * on every side, where text keeps the wider space beside it that its lines
   * need.
   */
  picture?: boolean
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
  if (!el)
    return
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

function openNow() {
  if (!canShow.value)
    return
  hiddenByScroll.value = false
  isOpen.value = true
}

function closeNow() {
  isOpen.value = false
}

function scheduleOpen() {
  clearCloseTimer()
  clearOpenTimer()
  if (!canShow.value)
    return
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
      <!-- A hint describes whatever is under the pointer, so it sits above every other
           layer: dialogs, popovers and toasts are z-50, and a tip inside one of them
           would otherwise open behind it. -->
      <div
        v-if="visible"
        ref="floatingRef"
        class="overlay-shell select-none pointer-events-none z-[60] w-max min-w-max rounded-md text-fg"
        :class="hasOverlay
          ? picture
            ? 'max-w-xs p-2'
            : 'max-w-xs px-3 py-2 text-xs leading-relaxed'
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

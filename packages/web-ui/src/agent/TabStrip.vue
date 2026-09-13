<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, type ComponentPublicInstance } from 'vue'
import {
  TAB_TRANSITION,
  TAB_WIDTH_MS,
  afterEnterTab,
  afterLeaveTab,
  beforeEnterTab,
  beforeLeaveTab,
  enterTab,
  leaveTab,
  settledTabBounds,
} from './tab-strip'

/** The edge fades' width; a revealed tab is scrolled clear of them. */
const FADE_PX = 24

/**
 * The row every tab bar uses. Tabs are all one width; when they outgrow the
 * row the strip scrolls with no scrollbar and fades out at whichever edge
 * has more behind it. Whenever the active tab changes, or a tab enters, the
 * strip scrolls until that tab shows whole and clear of the fades. The
 * scroll is the strip's own, timed and eased like the tab motion, and aimed
 * at the settled layout: a tab on its way out takes no room and a tab on its
 * way in its full width, so one motion lands the tab where it ends up. Close
 * and insert collapse or grow from the current width. The wheel scrolls the
 * strip along its axis, so a vertical wheel over the tabs moves them
 * sideways instead of the page.
 * The host passes its `TabItem`s as children and sizes the strip in its row.
 * The `trailing` slot (a New tab control) follows the last tab while the
 * tabs fit, and stays at the strip's right edge once they scroll.
 *
 * `surface` is what the row sits on. On the base surface the active tab is
 * raised to the surface color; on a raised surface it is pressed to the base
 * color. Hover uses the active color either way, so the close control's
 * masks stay opaque. The tabs and the edge fades take their colors from the
 * strip, so a host sets this once.
 */
const props = withDefaults(defineProps<{ surface?: 'base' | 'raised' }>(), {
  surface: 'base',
})
const colors = computed(() =>
  props.surface === 'base'
    ? {
        '--tab-row': 'var(--surface-base)',
        '--tab-active': 'var(--surface)',
        '--tab-hover': 'var(--surface)',
      }
    : {
        '--tab-row': 'var(--surface)',
        '--tab-active': 'var(--surface-base)',
        '--tab-hover': 'var(--surface-base)',
      },
)
const el = ref<HTMLElement | null>(null)
const moreBefore = ref(false)
const moreAfter = ref(false)
let resizeObserver: ResizeObserver | null = null
let mutationObserver: MutationObserver | null = null
let scrollFrame: number | null = null

function cancelScroll(): void {
  if (scrollFrame !== null) {
    cancelAnimationFrame(scrollFrame)
    scrollFrame = null
  }
}

// The same length and ease-out as the tab width motion, so the two read as one.
function scrollStripTo(strip: HTMLElement, target: number): void {
  cancelScroll()
  if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
    strip.scrollLeft = target
    return
  }
  const start = strip.scrollLeft
  const startedAt = performance.now()
  const step = (now: number): void => {
    const progress = Math.min(1, (now - startedAt) / TAB_WIDTH_MS)
    const eased = 1 - (1 - progress) ** 3
    strip.scrollLeft = start + (target - start) * eased
    scrollFrame = progress < 1 ? requestAnimationFrame(step) : null
  }
  scrollFrame = requestAnimationFrame(step)
}

function bindEl(instance: Element | ComponentPublicInstance | null): void {
  el.value =
    instance instanceof HTMLElement
      ? instance
      : instance && '$el' in instance
        ? instance.$el
        : null
}

function updateEdges(): void {
  const strip = el.value
  if (!strip) {
    return
  }
  moreBefore.value = strip.scrollLeft > 0
  moreAfter.value = strip.scrollLeft + strip.clientWidth < strip.scrollWidth - 1
}

// Only the strip scrolls, never the page: scrollIntoView would move both.
function revealActive(): void {
  const strip = el.value
  const active = strip?.querySelector<HTMLElement>('[aria-selected="true"]')
  if (strip && active) {
    const { left, right } = settledTabBounds(strip, active)
    const viewLeft = strip.scrollLeft
    const viewRight = viewLeft + strip.clientWidth
    if (left - FADE_PX < viewLeft) {
      scrollStripTo(strip, Math.max(0, left - FADE_PX))
    } else if (right + FADE_PX > viewRight) {
      scrollStripTo(strip, right + FADE_PX - strip.clientWidth)
    }
  }
  updateEdges()
}

// A wheel over the tabs moves them, whichever axis it turns on; the page
// keeps the event only when the strip has nothing to scroll that way.
function onWheel(event: WheelEvent): void {
  const strip = el.value
  if (!strip || strip.scrollWidth <= strip.clientWidth) {
    return
  }
  const delta = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? event.deltaX : event.deltaY
  const max = strip.scrollWidth - strip.clientWidth
  const atEdge = delta < 0 ? strip.scrollLeft <= 0 : strip.scrollLeft >= max
  if (atEdge) {
    return
  }
  event.preventDefault()
  cancelScroll()
  strip.scrollLeft += delta
}

// The strip's extent settles only after the motion; reveal again then.
function onAfterEnter(el: Element): void {
  afterEnterTab(el)
  revealActive()
}

function onAfterLeave(el: Element): void {
  afterLeaveTab(el)
  revealActive()
}

onMounted(() => {
  const strip = el.value
  if (!strip) {
    return
  }
  resizeObserver = new ResizeObserver(updateEdges)
  resizeObserver.observe(strip)
  // A tab becoming active, or tabs coming and going, is what moves the view.
  mutationObserver = new MutationObserver(revealActive)
  mutationObserver.observe(strip, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['aria-selected'],
  })
  revealActive()
})

onBeforeUnmount(() => {
  resizeObserver?.disconnect()
  mutationObserver?.disconnect()
  cancelScroll()
})

defineExpose({ el })
</script>

<template>
  <div class="flex min-w-0 items-center" :style="colors">
    <div class="relative min-w-0 shrink">
    <TransitionGroup
      :ref="bindEl"
      :name="TAB_TRANSITION"
      tag="div"
      class="flex items-center gap-0.5 overflow-x-auto [scrollbar-width:none]"
      role="tablist"
      @scroll.passive="updateEdges"
      @wheel="onWheel"
      @before-enter="beforeEnterTab"
      @enter="enterTab"
      @after-enter="onAfterEnter"
      @enter-cancelled="onAfterEnter"
      @before-leave="beforeLeaveTab"
      @leave="leaveTab"
      @after-leave="onAfterLeave"
      @leave-cancelled="onAfterLeave"
    >
      <slot />
    </TransitionGroup>
    <div
      v-if="moreBefore"
      aria-hidden="true"
      class="pointer-events-none absolute inset-y-0 left-0 w-6 bg-linear-to-r from-(--tab-row) to-transparent"
    />
    <div
      v-if="moreAfter"
      aria-hidden="true"
      class="pointer-events-none absolute inset-y-0 right-0 w-6 bg-linear-to-l from-(--tab-row) to-transparent"
    />
    </div>
    <slot name="trailing" />
  </div>
</template>

<style>
.tabs-enter-active,
.tabs-leave-active {
  transition:
    width 200ms ease-out,
    margin 200ms ease-out,
    opacity 150ms ease;
}
.tabs-leave-active {
  pointer-events: none;
}
.tabs-enter-from,
.tabs-leave-to {
  opacity: 0;
}
@media (prefers-reduced-motion: reduce) {
  .tabs-enter-active,
  .tabs-leave-active {
    transition: none;
  }
}
</style>

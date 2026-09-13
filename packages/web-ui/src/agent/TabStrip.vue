<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, type ComponentPublicInstance } from 'vue'
import {
  TAB_TRANSITION,
  afterEnterTab,
  afterLeaveTab,
  beforeEnterTab,
  beforeLeaveTab,
  enterTab,
  leaveTab,
} from './tab-strip'

/**
 * The row every tab bar uses. Tabs are all one width; when they outgrow the
 * row the strip scrolls with no scrollbar and fades out at whichever edge
 * has more behind it, and the active tab is scrolled into view when it
 * changes. Close and insert collapse or grow from the current width.
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
    const left = active.offsetLeft
    const right = left + active.offsetWidth
    if (left < strip.scrollLeft) {
      strip.scrollLeft = left
    } else if (right > strip.scrollLeft + strip.clientWidth) {
      strip.scrollLeft = right - strip.clientWidth
    }
  }
  updateEdges()
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
      @before-enter="beforeEnterTab"
      @enter="enterTab"
      @after-enter="afterEnterTab"
      @enter-cancelled="afterEnterTab"
      @before-leave="beforeLeaveTab"
      @leave="leaveTab"
      @after-leave="afterLeaveTab"
      @leave-cancelled="afterLeaveTab"
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

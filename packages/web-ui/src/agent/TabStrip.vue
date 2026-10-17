<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch, type ComponentPublicInstance } from 'vue'
import {
  TAB_TRANSITION,
  TAB_WIDTH_MS,
  afterEnterTab,
  afterLeaveTab,
  beforeEnterTab,
  beforeLeaveTab,
  cutMarkCover,
  enterTab,
  isLeavingTab,
  fadeRoom,
  leaveTab,
  revealScroll,
  settledTabBounds,
  stripGap,
  tabsChanged,
} from './tab-strip'
import { DRAG_START_PX, dragShift, dragTarget } from '../ui/drag-order'
import { usePhone } from '../ui/phone'

/** The edge fades' width; a revealed tab is scrolled clear of them. */
const FADE_PX = 24

/**
 * The row every tab bar uses. Tabs fit their content up to their maximum width; when they outgrow the
 * row the strip scrolls with no scrollbar and fades out at whichever edge
 * has more behind it. A tab mark that edge cuts stays under the fade's solid
 * start, so a mark at an edge shows whole or not at all, never sliced.
 * The selected tab always shows whole, its close control included, and no
 * fade lies over it: whenever the selected tab changes, a tab enters, the
 * strip resizes or the selected tab does (as when its title arrives), the
 * strip scrolls until that tab shows whole with a fade's width beside it,
 * and where the view has no room for that, the fades give way beside the
 * tab. No tab is wider than the strip has room for (`--tab-room`), so a
 * narrow strip truncates its title instead of cutting it. Unselected tabs
 * may lie under the fades. The
 * scroll is the strip's own, timed and eased like the tab motion, and aimed
 * at the settled layout: a tab on its way out takes no room and a tab on its
 * way in its full width, so one motion lands the tab where it ends up. Close
 * and insert collapse or grow from the current width. The wheel scrolls the
 * strip along its axis, so a vertical wheel over the tabs moves them
 * sideways instead of the page.
 * The host passes its `TabItem`s as children and sizes the strip in its row.
 * The `leading` slot holds tabs that stand ahead of them and never scroll,
 * such as the work panel's pinned tabs: elements of `role="tab"` like the
 * children, in the same tab list, which give way before the strip's own tabs,
 * their titles truncating down to what their own minimum width keeps.
 *
 * `surface` is what the row sits on. On the base surface the active tab is
 * raised to the surface color; on a raised surface it is pressed to the base
 * color, and hover uses the active color either way. The tabs and the edge
 * fades take their colors from the strip, so a host sets this once.
 *
 * The arrow keys, Home and End move the keyboard's focus between the tabs,
 * the leading ones included.
 *
 * On a phone the tabs never shrink: each keeps its content's width up to its
 * maximum, the leading tabs too, and the leading tabs and the strip's own
 * scroll as one row under a finger, with no scrollbar and their overscroll
 * kept to the row; the selected tab is revealed as above, a leading one
 * included. A touch there scrolls the row and drags no tab.
 *
 * A host that listens to `reorder` lets the user drag a tab along the strip,
 * as in a web browser: the others step aside as it passes their middle, and
 * on release `reorder` names where it was and its index among the others,
 * which the host applies; Escape puts it back.
 */
const props = withDefaults(defineProps<{
  surface?: 'base' | 'raised'
  /** A tab dragged from `from` to `to`, its index among the others; without it tabs do not drag. */
  onReorder?: (from: number, to: number) => void
  /**
   * Tabs come and go at once, with no motion, while this holds: as when a
   * menu replaces the tab shown with another, so nothing opened or closed.
   */
  still?: boolean
}>(), {
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
const phone = usePhone()
const root = ref<HTMLElement | null>(null)
const list = ref<HTMLElement | null>(null)
const leading = ref<HTMLElement | null>(null)
const el = ref<HTMLElement | null>(null)
const fades = ref<HTMLElement | null>(null)
/** The selected tab the resize observer watches, so a title that widens it reveals it again. */
let watchedTab: HTMLElement | null = null
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

/** What scrolls: the strip's own tabs, or on a phone the whole tab list, the leading tabs with them. */
function scroller(): HTMLElement | null {
  return phone.value ? list.value : el.value
}

/** Where the selected `tab` sits in the scroller's content once the strip's motion ends. */
function activeBounds(scrolling: HTMLElement, tab: HTMLElement): { left: number; right: number } {
  const strip = el.value
  if (!strip || scrolling === strip) {
    return settledTabBounds(scrolling, tab)
  }
  // On a phone: a leading tab where it stands, a tab of the strip after the leading ones.
  const origin = scrolling.getBoundingClientRect().left - scrolling.scrollLeft
  if (!strip.contains(tab)) {
    const rect = tab.getBoundingClientRect()
    return { left: rect.left - origin, right: rect.right - origin }
  }
  const offset = strip.getBoundingClientRect().left - origin
  const own = settledTabBounds(strip, tab)
  return { left: own.left + offset, right: own.right + offset }
}

function updateEdges(): void {
  const strip = scroller()
  if (!strip) {
    return
  }
  moreBefore.value = strip.scrollLeft > 0
  moreAfter.value = strip.scrollLeft + strip.clientWidth < strip.scrollWidth - 1
  const view = strip.getBoundingClientRect()
  const marks = [...strip.querySelectorAll('[data-tab-mark]')].map((mark) => {
    const rect = mark.getBoundingClientRect()
    return { left: rect.left - view.left, right: rect.right - view.left }
  })
  const selected = strip.querySelector<HTMLElement>('[aria-selected="true"]')?.getBoundingClientRect()
  const room = fadeRoom(
    strip.clientWidth,
    selected ? { left: selected.left - view.left, right: selected.right - view.left } : null,
    scrollFrame !== null,
  )
  // Written to the style, not kept as state: they change on every frame of
  // a scroll, and a render of the strip then would disturb its tabs' motion.
  const style = fades.value?.style
  style?.setProperty('--fade-before', `${cutMarkCover(0, 'start', marks)}px`)
  style?.setProperty('--fade-after', `${cutMarkCover(strip.clientWidth, 'end', marks)}px`)
  style?.setProperty('--fade-before-room', Number.isFinite(room.before) ? `${room.before}px` : '100%')
  style?.setProperty('--fade-after-room', Number.isFinite(room.after) ? `${room.after}px` : '100%')
}

/**
 * The least the leading tabs take once they have given way, with the gap
 * after them. Their present width would not do: it is what the strip's tabs
 * leave them, so a room measured from it would shrink the tabs in their place.
 */
function leadingMinimum(): number {
  const lead = leading.value
  if (!lead || !list.value) {
    return 0
  }
  // Read in one layout and put back before anything paints or observes it.
  lead.style.width = 'min-content'
  const width = lead.offsetWidth
  lead.style.width = ''
  return width + stripGap(list.value)
}

/** The width a tab may take: the strip's own, less the leading tabs at their least. */
function measureRoom(): void {
  const strip = root.value
  if (!strip) {
    return
  }
  // On a phone the leading tabs scroll away with the rest, so a tab may take the whole view.
  const leadingTabs = phone.value ? 0 : leadingMinimum()
  const room = `${Math.max(0, strip.clientWidth - leadingTabs)}px`
  // Unchanged, it is not written again: writing it resizes the tabs, which the observer reports.
  if (strip.style.getPropertyValue('--tab-room') !== room) {
    strip.style.setProperty('--tab-room', room)
  }
}

/** The strip or its selected tab changed size: the selected tab is revealed again. */
function resized(): void {
  measureRoom()
  revealActive()
}

// Only the strip scrolls, never the page: scrollIntoView would move both.
function revealActive(): void {
  const strip = scroller()
  const active = strip?.querySelector<HTMLElement>('[aria-selected="true"]') ?? null
  if (active !== watchedTab) {
    if (watchedTab) {
      resizeObserver?.unobserve(watchedTab)
    }
    if (active) {
      resizeObserver?.observe(active)
    }
    watchedTab = active
  }
  if (strip && active) {
    const target = revealScroll(strip, activeBounds(strip, active), FADE_PX)
    if (target !== null) {
      scrollStripTo(strip, target)
    }
  }
  updateEdges()
}

// A wheel over the tabs moves them, whichever axis it turns on; the page
// keeps the event only when the strip has nothing to scroll that way.
function onWheel(event: WheelEvent): void {
  const strip = scroller()
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

/** The strip's own tabs in order, the ones a drag moves among, without those on their way out. */
function tabElements(): HTMLElement[] {
  const strip = el.value
  if (!strip) {
    return []
  }
  return [...strip.querySelectorAll<HTMLElement>(':scope > [role="tab"]')].filter((tab) => !isLeavingTab(tab))
}

// The arrows move the focus along the tabs, the leading ones first, wrapping
// at the ends; Enter or Space on a tab is the tab's own.
function onKeydown(event: KeyboardEvent): void {
  const tabs = [...leading.value?.querySelectorAll<HTMLElement>('[role="tab"]') ?? [], ...tabElements()]
  const at = tabs.findIndex((tab) => tab === event.target)
  if (at < 0) {
    return
  }
  const next = {
    ArrowLeft: (at - 1 + tabs.length) % tabs.length,
    ArrowRight: (at + 1) % tabs.length,
    Home: 0,
    End: tabs.length - 1,
  }[event.key]
  if (next === undefined) {
    return
  }
  event.preventDefault()
  tabs[next]?.focus()
}

/** A tab pressed with the primary button, which drags once the pointer moves far enough. */
interface Drag {
  pointerId: number
  startX: number
  from: number
  tabs: HTMLElement[]
  /** Each tab's center when the press began, in strip order. */
  centers: number[]
  /** How far the dragged tab may move each way, so it stays within the tabs. */
  min: number
  max: number
  /** The width the others step aside by: the dragged tab's and a gap. */
  step: number
  to: number
  moving: boolean
}
let drag: Drag | null = null

function onPointerdown(event: PointerEvent): void {
  const strip = el.value
  const tab = event.target instanceof Element ? event.target.closest<HTMLElement>('[role="tab"]') : null
  if (!props.onReorder || event.button !== 0 || !strip || !tab) {
    return
  }
  // A finger on a phone's strip scrolls it; the browser's pan would cut a drag short anyway.
  if (phone.value && event.pointerType === 'touch') {
    return
  }
  const tabs = tabElements()
  const from = tabs.indexOf(tab)
  if (from < 0) {
    return
  }
  const rects = tabs.map((each) => each.getBoundingClientRect())
  const own = rects[from]!
  drag = {
    pointerId: event.pointerId,
    startX: event.clientX,
    from,
    tabs,
    centers: rects.map((rect) => rect.left + rect.width / 2),
    min: rects[0]!.left - own.left,
    max: rects.at(-1)!.right - own.right,
    step: own.width + stripGap(strip),
    to: from,
    moving: false,
  }
}

function onPointermove(event: PointerEvent): void {
  const held = drag
  if (!held || event.pointerId !== held.pointerId) {
    return
  }
  const dx = event.clientX - held.startX
  if (!held.moving) {
    if (Math.abs(dx) < DRAG_START_PX) {
      return
    }
    held.moving = true
    el.value?.setPointerCapture(event.pointerId)
    cancelScroll()
  }
  const offset = Math.min(held.max, Math.max(held.min, dx))
  held.to = dragTarget(held.centers, held.from, held.centers[held.from]! + offset)
  held.tabs.forEach((tab, index) => {
    if (index === held.from) {
      tab.style.transition = 'none'
      tab.style.zIndex = '2'
      tab.style.transform = `translateX(${offset}px)`
      return
    }
    tab.style.transition = 'transform 150ms ease-out'
    tab.style.transform = `translateX(${dragShift(index, held.from, held.to) * held.step}px)`
  })
}

/** Puts every tab of `ended` back in its own place, without motion. */
function release(ended: Drag): void {
  for (const tab of ended.tabs) {
    tab.style.transition = ''
    tab.style.zIndex = ''
    tab.style.transform = ''
  }
}

function onPointerup(event: PointerEvent): void {
  const ended = drag
  if (!ended || event.pointerId !== ended.pointerId) {
    return
  }
  drag = null
  if (!ended.moving) {
    return
  }
  if (ended.to === ended.from) {
    release(ended)
    return
  }
  props.onReorder?.(ended.from, ended.to)
  // The host's order lands with the next render; until then the tabs hold where the drag left them.
  void nextTick(() => release(ended))
}

function cancelDrag(): void {
  const ended = drag
  drag = null
  if (ended?.moving) {
    release(ended)
  }
}

function onDragKeydown(event: KeyboardEvent): void {
  if (event.key === 'Escape' && drag?.moving) {
    event.preventDefault()
    cancelDrag()
  }
}

onMounted(() => {
  window.addEventListener('keydown', onDragKeydown)
  const strip = el.value
  if (!strip) {
    return
  }
  resizeObserver = new ResizeObserver(resized)
  for (const watched of [root.value, leading.value]) {
    if (watched) {
      resizeObserver.observe(watched)
    }
  }
  // A tab becoming active, a leading one included, or tabs coming and going,
  // is what moves the view; any other change, such as a title that widens a
  // tab, moves the marks.
  mutationObserver = new MutationObserver((records) => {
    if (tabsChanged(strip, records, (node) => node instanceof HTMLElement)) {
      revealActive()
    } else {
      updateEdges()
    }
  })
  mutationObserver.observe(list.value ?? strip, {
    childList: true,
    subtree: true,
    attributes: true,
    attributeFilter: ['aria-selected'],
  })
  measureRoom()
  revealActive()
})

// The leading tabs' least width changes with their marks and badges, and they come and go with the host's.
watch(leading, (now, before) => {
  if (before) {
    resizeObserver?.unobserve(before)
  }
  if (now) {
    resizeObserver?.observe(now)
  }
}, { flush: 'post' })

// Crossing the phone's width changes what scrolls and the room a tab has.
watch(phone, () => {
  cancelScroll()
  void nextTick(resized)
})

onBeforeUnmount(() => {
  window.removeEventListener('keydown', onDragKeydown)
  cancelDrag()
  resizeObserver?.disconnect()
  watchedTab = null
  mutationObserver?.disconnect()
  cancelScroll()
})

defineExpose({ el })
</script>

<template>
  <div ref="root" class="flex min-w-0 items-center" :style="colors">
    <!-- On a phone the fades' frame: the whole tab list scrolls inside it, so the fades stay put
         over the row's ends. -->
    <div class="flex min-w-0 shrink phone:relative phone:isolate">
    <!-- One tab list holds the leading tabs and the strip's own, so the keyboard and assistive
         technology meet them as one row of tabs. On a phone it is what scrolls. -->
    <div
      ref="list"
      role="tablist"
      class="flex min-w-0 shrink items-center gap-1 phone:overflow-x-auto phone:overscroll-x-contain [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      @keydown="onKeydown"
      @scroll.passive="updateEdges"
    >
    <!-- The leading tabs give way first: their shrink outweighs the strip's a hundredfold. On a
         phone they keep their width and scroll with the rest. -->
    <div v-if="$slots.leading" ref="leading" class="flex shrink-[100] items-center gap-1 phone:shrink-0">
      <slot name="leading" />
    </div>
    <!-- Its own stacking context, so the fades lie over the scrolled tabs and nothing else. On a
         phone it takes its tabs' whole width and the view holds the fades. -->
    <div
      ref="fades"
      class="relative isolate min-w-0 shrink [--fade-after:0px] [--fade-before:0px] phone:static phone:isolation-auto phone:shrink-0"
    >
    <TransitionGroup
      :ref="bindEl"
      :name="TAB_TRANSITION"
      :css="!still"
      tag="div"
      class="flex items-center gap-0.5 overflow-x-auto phone:overflow-x-visible [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
      @scroll.passive="updateEdges"
      @wheel="onWheel"
      @pointerdown="onPointerdown"
      @pointermove="onPointermove"
      @pointerup="onPointerup"
      @pointercancel="cancelDrag"
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
      class="pointer-events-none absolute inset-y-0 left-0 z-1 w-[min(calc(var(--fade-before)_+_--spacing(6)),var(--fade-before-room))] bg-[linear-gradient(to_right,var(--tab-row)_var(--fade-before),transparent)]"
    />
    <div
      v-if="moreAfter"
      aria-hidden="true"
      class="pointer-events-none absolute inset-y-0 right-0 z-1 w-[min(calc(var(--fade-after)_+_--spacing(6)),var(--fade-after-room))] bg-[linear-gradient(to_left,var(--tab-row)_var(--fade-after),transparent)]"
    />
    </div>
    </div>
    </div>
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

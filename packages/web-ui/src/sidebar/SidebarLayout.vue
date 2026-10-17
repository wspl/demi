<script setup lang="ts">
import { t } from './locales/text'
import { computed, provide, ref, watch } from 'vue'
import { useElementSize, useEventListener } from '@vueuse/core'
import { useEscapeClose } from '../composables/useEscapeClose'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { clampSize } from '../ui/resize-handle'
import { usePhone } from '../ui/phone'
import { dragCloses, dragVelocity } from '../ui/drag-close'
import { useBackClose } from '../overlay/back-close'
import { appOverlayStore } from '../overlay/appOverlay'
import { ASIDE_SHARE, SIDEBAR_WIDTH, asideBounds, asideShareFor, asideWidthFor } from './sidebar-width'
import { sidebarDrawerKey } from './sidebar-drawer'

/**
 * The app frame: the sidebar, the main pane, and the work panel (aside) on
 * the right, with a divider sizing each side pane. The sidebar keeps a px
 * width; the aside keeps a share of the width it splits with the main pane,
 * so a window resize scales the two together and leaves the sidebar alone.
 * The host keeps both sizes (v-model) and hears `resizeEnd` /
 * `asideResizeEnd` when one has settled, which is the moment to persist it.
 * The aside is only there while `asideOpen` is true and the host gave the slot.
 *
 * Each of the three panes is a region (`data-region`, `ui/region.ts`): a
 * menu opened from a trigger in it grows toward its inside.
 *
 * On a phone (`ui/phone.ts`) one pane shows at a time. The sidebar is a
 * drawer from the left, 85% of the screen and at most 320 px, over a scrim
 * that a tap closes it by, as do a swipe left on it, Escape and Back; the
 * work panel opens full screen over the conversation, sliding in from the
 * right, and Back closes it too (`useBackClose`). Opening one closes the other. The frame shows
 * one top bar there: a page with its own puts the sidebar's button in it
 * (`useSidebarDrawer`), and the frame's own, the button and `label`, shows
 * only while none does.
 *
 * The main pane stacks what it holds within itself (`isolate`): a page's
 * own layers, such as the conversation's dock over its transcript, stay
 * under the drawer and its scrim, whatever their z-index.
 */
withDefaults(defineProps<{ label?: string }>(), { label: 'Demi' })
const emit = defineEmits<{
  resizeEnd: [width: number]
  asideResizeEnd: [share: number]
}>()
const open = defineModel<boolean>('open', { default: false })
const width = defineModel<number>('width', { default: SIDEBAR_WIDTH.default })
const asideOpen = defineModel<boolean>('asideOpen', { default: false })
const asideShare = defineModel<number>('asideShare', { default: ASIDE_SHARE.default })
// A stored width outside today's bounds is shown at the bound, not rewritten.
const shownWidth = computed(() => clampSize(width.value, SIDEBAR_WIDTH))

// The main pane and the aside split what the sidebar leaves; a hidden sidebar measures 0.
const frame = ref<HTMLElement | null>(null)
const sidebar = ref<HTMLElement | null>(null)
const { width: frameWidth } = useElementSize(frame, { width: window.innerWidth, height: 0 })
const { width: sidebarWidth } = useElementSize(sidebar)
const sharedWidth = computed(() => frameWidth.value - sidebarWidth.value)
const shownAsideWidth = computed(() => asideWidthFor(asideShare.value, sharedWidth.value))
const asideDragBounds = computed(() => asideBounds(sharedWidth.value))

const phone = usePhone()

// On a phone the two overlays never stack: the newly opened one wins.
watch(open, (value) => {
  if (value && phone.value) {
    asideOpen.value = false
  }
})
watch(asideOpen, (value) => {
  if (value && phone.value) {
    open.value = false
  }
})

/** The pages that show their own top bar on a phone, which hides the frame's. */
const barOwners = ref(0)
provide(sidebarDrawerKey, {
  open: () => {
    open.value = true
  },
  claimBar: () => {
    barOwners.value += 1
    return () => {
      barOwners.value -= 1
    }
  },
})

// The full-screen panel is a place Back leaves, as a pushed page is, and the drawer a layer Back closes.
useBackClose(() => asideOpen.value, () => {
  asideOpen.value = false
}, () => phone.value)
useBackClose(() => open.value, () => {
  open.value = false
}, () => phone.value)

// Escape closes the drawer once no menu or dialog over it takes the key first.
useEscapeClose(() => open.value && phone.value && !appOverlayStore.hasExclusive(), () => {
  open.value = false
})

/** The drawer's width on a phone: 85% of the screen, at most 320 px. */
const DRAWER_WIDTH = 'min(85vw, 320px)'

/**
 * How far a swipe has pulled the drawer left of its place, in px; null at
 * rest. A swipe that closes it leaves it here, so the drawer and the scrim
 * go on closing from where they stand, until the drawer has left.
 */
const drawerPull = ref<number | null>(null)
/** Whether a finger holds the drawer now, which then follows it without easing. */
const pulling = ref(false)
/** How dark the scrim is, from 1 with the drawer in place to 0 with it pulled its whole width away. */
const scrimShade = computed(() => drawerPull.value === null || sidebarWidth.value <= 0
  ? 1
  : Math.max(0, 1 - drawerPull.value / sidebarWidth.value))
let swipe: { startX: number; startY: number; holding: boolean; samples: { at: number; t: number }[] } | null = null
/** A finger that moves this far settles whether it swipes the drawer or scrolls its list. */
const SWIPE_SLOP_PX = 8

/** A swipe that leaves the drawer open: it springs back, and the scrim darkens with it. */
function springBack(): void {
  swipe = null
  pulling.value = false
  drawerPull.value = null
}

function swipeStart(event: TouchEvent): void {
  const finger = event.touches[0]
  // A second finger takes the gesture from the first: the drawer goes back to its place.
  if (event.touches.length !== 1 || !finger) {
    if (swipe?.holding)
      springBack()
    swipe = null
    return
  }
  // The first touch is the pull's first sample, so a flick of a single move has a speed.
  swipe = { startX: finger.clientX, startY: finger.clientY, holding: false, samples: [{ at: 0, t: event.timeStamp }] }
}

function swipeMove(event: TouchEvent): void {
  const finger = event.touches[0]
  if (!swipe || !finger)
    return
  // `touches` lists every finger on the screen: one put down anywhere else, such as on the scrim, takes the gesture too.
  if (event.touches.length > 1) {
    swipeCancel()
    return
  }
  const dx = finger.clientX - swipe.startX
  const dy = finger.clientY - swipe.startY
  if (!swipe.holding) {
    if (Math.hypot(dx, dy) < SWIPE_SLOP_PX)
      return
    // A move mostly to the left swipes the drawer; any other scrolls its list.
    if (!(dx < 0 && Math.abs(dx) > Math.abs(dy))) {
      swipe = null
      return
    }
    swipe.holding = true
    pulling.value = true
  }
  if (event.cancelable)
    event.preventDefault()
  // The drawer follows the finger from where it first touched, the slop included.
  const pulled = Math.max(0, swipe.startX - finger.clientX)
  swipe.samples = [...swipe.samples.filter((sample) => event.timeStamp - sample.t < 120), { at: pulled, t: event.timeStamp }]
  drawerPull.value = pulled
}

/** The finger lifts: a long or flicked pull closes the drawer from where it stands, a short or still one springs back. */
function swipeEnd(event: TouchEvent): void {
  const ended = swipe
  if (!ended?.holding) {
    swipe = null
    return
  }
  // A finger still on the screen, wherever it went down, means the lift was not the gesture's end.
  if (event.touches.length > 0) {
    springBack()
    return
  }
  const width = sidebar.value?.offsetWidth ?? 0
  if (dragCloses(drawerPull.value ?? 0, width, dragVelocity(ended.samples, event.timeStamp))) {
    swipe = null
    pulling.value = false
    open.value = false
  } else {
    springBack()
  }
}

/** The system took the touch, as for a call or a gesture of its own: only a lift closes the drawer. */
function swipeCancel(): void {
  if (swipe?.holding)
    springBack()
  swipe = null
}

/**
 * The scrim starts to leave: it drops its following of the finger, which a
 * leaving node is no longer patched to drop, so it fades from where it stands.
 */
function scrimLeaving(element: Element): void {
  element.removeAttribute('data-pulled')
}

/** The drawer has left, or came back before it had: the next one opens in place. */
function drawerSettled(): void {
  drawerPull.value = null
}

// Touch moves are listened to without `passive`, so that a swipe the drawer takes scrolls nothing.
useEventListener(sidebar, 'touchmove', swipeMove, { passive: false })
</script>

<template>
  <div ref="frame" class="flex h-full bg-surface-base text-fg">
    <Transition name="frame-scrim" @before-leave="scrimLeaving">
      <div
        v-if="open && phone"
        class="frame-scrim fixed inset-0 z-30 bg-black/50"
        :style="{ '--scrim-shade': scrimShade }"
        :data-pulled="pulling ? '' : undefined"
        @click="open = false"
      />
    </Transition>
    <!-- On a phone the sidebar is a drawer, which slides in over the scrim. -->
    <Transition name="frame-drawer" :css="phone" @after-leave="drawerSettled" @leave-cancelled="drawerSettled">
      <div
        v-show="open || !phone"
        ref="sidebar"
        data-region
        class="h-full shrink-0"
        :class="phone ? 'frame-drawer fixed inset-y-0 left-0 z-40 bg-surface-base pt-[var(--safe-top)] pb-[var(--safe-bottom)]' : ''"
        :style="{
          '--sidebar-width': phone ? DRAWER_WIDTH : `${shownWidth}px`,
          '--drawer-pull': drawerPull === null ? undefined : `${-drawerPull}px`,
        }"
        :data-pulled="pulling ? '' : undefined"
        @touchstart.passive="swipeStart"
        @touchend="swipeEnd"
        @touchcancel="swipeCancel"
      >
        <slot name="sidebar" />
      </div>
    </Transition>
    <ResizeHandle
      v-model="width"
      class="hidden md:block"
      :min="SIDEBAR_WIDTH.min"
      :max="SIDEBAR_WIDTH.max"
      :default-value="SIDEBAR_WIDTH.default"
      :label="t('layout.sidebarWidth')"
      @commit="emit('resizeEnd', $event)"
    />
    <!-- On a phone the pane is the conversation's surface from the first frame, its top bar too, which stands
         behind the status bar: a page that has not drawn its own yet shows no band of the base surface. -->
    <main data-region class="isolate flex min-w-0 flex-1 flex-col overflow-hidden phone:bg-surface">
      <!-- The frame's own top bar, while no page shows its own (`useSidebarDrawer`). -->
      <div
        v-if="phone && barOwners === 0"
        class="flex h-[calc(2.75rem+var(--safe-top))] shrink-0 select-none items-center px-1.5 pt-[var(--safe-top)]"
      >
        <IconButton
          :icon="PanelLeft"
          variant="ghost"
          :aria-label="t('layout.openSidebar')"
          @click="open = true"
        />
        <span class="px-2 text-chrome text-fg-muted">{{ label }}</span>
      </div>
      <slot />
    </main>
    <!-- The handle works in px; the model keeps the share those px are of the shared width. -->
    <ResizeHandle
      v-if="asideOpen && $slots.aside && !phone"
      :model-value="shownAsideWidth"
      side="end"
      :min="asideDragBounds.min"
      :max="asideDragBounds.max"
      :default-value="asideWidthFor(ASIDE_SHARE.default, sharedWidth)"
      :label="t('layout.workPanelWidth')"
      @update:model-value="asideShare = asideShareFor($event, sharedWidth)"
      @commit="emit('asideResizeEnd', asideShareFor($event, sharedWidth))"
    />
    <!-- On a phone the panel takes the whole screen, sliding in from the right. -->
    <Transition name="frame-aside" :css="phone">
      <div
        v-if="asideOpen && $slots.aside"
        data-region
        :class="phone ? 'fixed inset-0 z-40' : 'h-full max-w-full shrink-0'"
        :style="phone ? undefined : { width: `${shownAsideWidth}px` }"
      >
        <slot name="aside" />
      </div>
    </Transition>
    <slot name="dialogs" />
  </div>
</template>

<style scoped>
.frame-drawer {
  translate: var(--drawer-pull, 0px) 0;
  transition: translate 300ms cubic-bezier(0.32, 0.72, 0, 1);
}

.frame-drawer[data-pulled] {
  transition: none;
}

.frame-drawer-enter-from,
.frame-drawer-leave-to {
  translate: -100% 0 !important;
}

.frame-aside-enter-active,
.frame-aside-leave-active {
  transition: translate 300ms cubic-bezier(0.32, 0.72, 0, 1);
}

.frame-aside-enter-from,
.frame-aside-leave-to {
  translate: 100% 0;
}

/* The scrim lightens as the drawer is pulled, and eases back with it when it springs back. */
.frame-scrim {
  opacity: var(--scrim-shade, 1);
  transition: opacity 300ms cubic-bezier(0.32, 0.72, 0, 1);
}

.frame-scrim[data-pulled] {
  transition: none;
}

.frame-scrim-enter-from,
.frame-scrim-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .frame-drawer,
  .frame-aside-enter-active,
  .frame-aside-leave-active,
  .frame-scrim {
    transition: none;
  }
}
</style>

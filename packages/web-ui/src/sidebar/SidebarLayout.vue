<script setup lang="ts">
import { computed, provide, ref, watch } from 'vue'
import { useElementSize, useEventListener } from '@vueuse/core'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { clampSize } from '../ui/resize-handle'
import { usePhone } from '../ui/phone'
import { dragCloses, dragVelocity } from '../ui/drag-close'
import { useBackClose } from '../overlay/back-close'
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
 * that a tap closes it by, as does a swipe left on it; the work panel opens
 * full screen over the conversation, sliding in from the right, and Back
 * closes it (`useBackClose`). Opening one closes the other. The frame shows
 * one top bar there: a page with its own puts the sidebar's button in it
 * (`useSidebarDrawer`), and the frame's own, the button and `label`, shows
 * only while none does.
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

// The full-screen panel is a place Back leaves, as a pushed page is.
useBackClose(() => asideOpen.value, () => {
  asideOpen.value = false
}, () => phone.value)

/** The drawer's width on a phone: 85% of the screen, at most 320 px. */
const DRAWER_WIDTH = 'min(85vw, 320px)'

/** How far a swipe has pulled the drawer left of its place, in px; null at rest. */
const drawerPull = ref<number | null>(null)
let swipe: { startX: number; startY: number; holding: boolean; samples: { at: number; t: number }[] } | null = null
/** A finger that moves this far settles whether it swipes the drawer or scrolls its list. */
const SWIPE_SLOP_PX = 8

function swipeStart(event: TouchEvent): void {
  const finger = event.touches[0]
  swipe = event.touches.length === 1 && finger
    ? { startX: finger.clientX, startY: finger.clientY, holding: false, samples: [] }
    : null
}

function swipeMove(event: TouchEvent): void {
  const finger = event.touches[0]
  if (!swipe || !finger)
    return
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
    swipe.startX = finger.clientX
  }
  if (event.cancelable)
    event.preventDefault()
  const pulled = Math.max(0, swipe.startX - finger.clientX)
  const now = performance.now()
  swipe.samples = [...swipe.samples.filter((sample) => now - sample.t < 120), { at: pulled, t: now }]
  drawerPull.value = pulled
}

function swipeEnd(): void {
  const ended = swipe
  swipe = null
  if (!ended?.holding)
    return
  const width = sidebar.value?.offsetWidth ?? 0
  if (dragCloses(drawerPull.value ?? 0, width, dragVelocity(ended.samples)))
    open.value = false
  drawerPull.value = null
}

// Touch moves are listened to without `passive`, so that a swipe the drawer takes scrolls nothing.
useEventListener(sidebar, 'touchmove', swipeMove, { passive: false })
</script>

<template>
  <div ref="frame" class="flex h-full bg-surface-base text-fg">
    <Transition name="frame-scrim">
      <div
        v-if="open && phone"
        class="frame-scrim fixed inset-0 z-30 bg-black/50"
        @click="open = false"
      />
    </Transition>
    <!-- On a phone the sidebar is a drawer, which slides in over the scrim. -->
    <Transition name="frame-drawer" :css="phone">
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
        :data-pulled="drawerPull === null ? undefined : ''"
        @touchstart.passive="swipeStart"
        @touchend="swipeEnd"
        @touchcancel="swipeEnd"
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
      label="Sidebar width"
      @commit="emit('resizeEnd', $event)"
    />
    <main data-region class="flex min-w-0 flex-1 flex-col overflow-hidden">
      <!-- The frame's own top bar, while no page shows its own (`useSidebarDrawer`). -->
      <div
        v-if="phone && barOwners === 0"
        class="flex h-[calc(2.75rem+var(--safe-top))] shrink-0 select-none items-center px-1.5 pt-[var(--safe-top)]"
      >
        <IconButton
          :icon="PanelLeft"
          variant="ghost"
          aria-label="Open sidebar"
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
      label="Work panel width"
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

.frame-scrim-enter-active,
.frame-scrim-leave-active {
  transition: opacity 300ms cubic-bezier(0.32, 0.72, 0, 1);
}

.frame-scrim-enter-from,
.frame-scrim-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .frame-drawer,
  .frame-aside-enter-active,
  .frame-aside-leave-active,
  .frame-scrim-enter-active,
  .frame-scrim-leave-active {
    transition: none;
  }
}
</style>

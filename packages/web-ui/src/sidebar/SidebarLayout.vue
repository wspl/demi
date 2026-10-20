<script setup lang="ts">
import { t } from './locales/text'
import { computed, onBeforeUnmount, provide, ref, useSlots, watch } from 'vue'
import { useElementSize, useEventListener } from '@vueuse/core'
import { useEscapeClose } from '../composables/useEscapeClose'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { clampSize } from '../ui/resize-handle'
import { SCREEN_EDGE_PX, usePhone } from '../ui/phone'
import { createFingerDrag } from '../ui/finger-drag'
import { useBackClose } from '../overlay/back-close'
import { appOverlayStore } from '../overlay/appOverlay'
import { ASIDE_SHARE, SIDEBAR_WIDTH, asideBounds, asideShareFor, asideWidthFor } from './sidebar-width'
import { sidebarDrawerKey } from './sidebar-drawer'
import { KEEPS_WINDOWS } from '../ui/window-stack'

/**
 * The app frame: the sidebar, the main pane, and the work panel (aside) on the
 * right, with a divider sizing each side pane.
 *
 * Sizes:
 *
 * - The sidebar keeps a px width. The aside keeps a share of the width it splits
 *   with the main pane, so a window resize scales the two together and leaves the
 *   sidebar alone.
 * - The host keeps both sizes (v-model) and hears `resizeEnd` / `asideResizeEnd`
 *   when one has settled, which is the moment to persist it.
 * - The aside is only there while `asideOpen` is true and the host gave the slot.
 *
 * Each of the three panes is a region (`data-region`, `ui/region.ts`): a menu
 * opened from a trigger in it grows toward its inside.
 *
 * The work panel and its divider keep the session's windows open (`KEEPS_WINDOWS`,
 * `ui/window-stack.ts`): only a press in the sidebar or the main pane closes one,
 * so a subagent's changes can be read in the panel beside its Agents window.
 *
 * On a phone (`ui/phone.ts`) one pane shows at a time:
 *
 * - The sidebar is a drawer from the left, 85% of the screen and at most 320 px,
 *   over a scrim. A tap on the scrim closes it, as do a swipe left on it, Escape
 *   and Back. A swipe right from the screen's left edge (`SCREEN_EDGE_PX`) opens
 *   it, following the finger in as the closing swipe follows it out.
 * - The work panel opens full screen over the conversation, sliding in from the
 *   right. A swipe right from that edge takes it back, as do its back button and
 *   Back (`useBackClose`). While it is open the edge is its swipe, not the
 *   drawer's.
 * - Every swipe follows the finger and settles by one rule (`finger-drag.ts`).
 * - Opening one closes the other.
 * - The frame shows one top bar there: a page with its own puts the sidebar's
 *   button in it (`useSidebarDrawer`), and the frame's own, the button and
 *   `label`, shows only while none does.
 *
 * The main pane stacks what it holds within itself (`isolate`): a page's own
 * layers, such as the conversation's dock over its transcript, stay under the
 * drawer and its scrim, whatever their z-index.
 */
withDefaults(defineProps<{ label?: string }>(), { label: 'demi' })
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
const slots = useSlots()

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

/** The drawer's share of a phone's width, and the most it takes. */
const DRAWER_SHARE = 0.85
const DRAWER_MAX_PX = 320
/** The drawer's width on a phone: 85% of the screen, at most 320 px. */
const DRAWER_WIDTH = `min(${DRAWER_SHARE * 100}vw, ${DRAWER_MAX_PX}px)`

/** The drawer's width in px, which a swipe measures itself against: known before the drawer shows. */
function drawerWidth(): number {
  return Math.min(window.innerWidth * DRAWER_SHARE, DRAWER_MAX_PX)
}

/**
 * How far a swipe has pulled the drawer left of its place, in px; null at rest.
 *
 * - A swipe that closes it leaves it here, so the drawer and the scrim go on closing
 *   from where they stand, until the drawer has left.
 * - One that opens it from the edge starts it here a whole width away.
 */
const drawerPull = ref<number | null>(null)
/** Whether a finger holds the drawer now, which then follows it without easing. */
const pulling = ref(false)
/** Whether a finger is drawing the closed drawer in from the screen's edge, which shows it while it does. */
const drawingIn = ref(false)
/** How dark the scrim is, from 1 with the drawer in place to 0 with it pulled its whole width away. */
const scrimShade = computed(() => drawerPull.value === null
  ? 1
  : Math.max(0, 1 - drawerPull.value / drawerWidth()))

/** A swipe left on the open drawer, which closes it. */
const drawerOut = createFingerDrag({
  way: -1,
  size: drawerWidth,
  hold: () => {
    pulling.value = true
  },
  follow: (moved) => {
    drawerPull.value = moved
  },
  settle: (goes) => {
    pulling.value = false
    if (goes) {
      // It goes on closing from where it stands; once it has left, it rests (`drawerSettled`).
      open.value = false
    } else {
      drawerPull.value = null
    }
  },
})

/** A swipe right from the screen's edge over the conversation, which draws the closed drawer in. */
const drawerIn = createFingerDrag({
  way: 1,
  size: drawerWidth,
  hold: () => {
    pulling.value = true
    drawingIn.value = true
  },
  follow: (moved) => {
    drawerPull.value = Math.max(0, drawerWidth() - moved)
  },
  settle: (goes) => {
    pulling.value = false
    drawingIn.value = false
    if (goes) {
      // It goes on into its place from where the finger left it.
      open.value = true
      drawerPull.value = null
    }
    // Otherwise it slides back out from where the finger left it, and rests once it has left (`drawerSettled`).
  },
})

/**
 * How far a swipe has pulled the full-screen panel right of its place, in
 * px; null at rest. A swipe that takes it back leaves it here, so it goes on
 * sliding away from where it stands, until it has left.
 */
const panelPull = ref<number | null>(null)
/** Whether a finger holds the panel now, which then follows it without easing. */
const panelPulling = ref(false)

/** A swipe right from the screen's edge over the full-screen panel, which goes back to the conversation beneath. */
const panelBack = createFingerDrag({
  way: 1,
  size: () => window.innerWidth,
  hold: () => {
    panelPulling.value = true
  },
  follow: (moved) => {
    panelPull.value = moved
  },
  settle: (goes) => {
    panelPulling.value = false
    if (goes) {
      asideOpen.value = false
    } else {
      panelPull.value = null
    }
  },
})

const drags = [drawerOut, drawerIn, panelBack]
onBeforeUnmount(() => {
  for (const drag of drags)
    drag.dispose()
})

/**
 * Which drag a finger put down at `x` belongs to. On a phone the screen's left edge
 * is one drag's at a time:
 *
 * - Over the full-screen panel, it takes the panel back.
 * - Over the conversation, with the drawer closed, it draws the drawer in.
 */
function edgeOwner(x: number): 'panel' | 'drawer' | null {
  if (!phone.value || open.value || x > SCREEN_EDGE_PX)
    return null
  return asideOpen.value && slots.aside ? 'panel' : 'drawer'
}

/** A finger down anywhere on the frame. A second finger takes any drag under way from the first. */
function frameTouchStart(event: TouchEvent): void {
  if (event.touches.length !== 1) {
    for (const drag of drags)
      drag.cancel()
    return
  }
  const owner = event.touches[0] ? edgeOwner(event.touches[0].clientX) : null
  if (owner === 'panel')
    panelBack.start(event)
  else if (owner === 'drawer')
    drawerIn.start(event)
}

/**
 * The touch pointer the panel's edge swipe owns, from its press to its lift.
 *
 * A browser sends a finger's pointer events before its touch events, and what the
 * panel shows may act on them, as a live browser tab presses, moves and types into
 * its page. They stop here, on their way down, so the panel sees none of a touch
 * that is the swipe's, whether it drags or not.
 */
let edgePointer: number | null = null
function claimEdgePointer(event: PointerEvent): void {
  if (event.pointerType !== 'touch')
    return
  if (event.type === 'pointerdown') {
    edgePointer = edgeOwner(event.clientX) === 'panel' ? event.pointerId : null
  }
  if (event.pointerId !== edgePointer)
    return
  event.stopPropagation()
  if (event.type === 'pointerup' || event.type === 'pointercancel')
    edgePointer = null
}

/**
 * The scrim or the panel starts to leave: it drops its following of the
 * finger, which a leaving node is no longer patched to drop, so it eases
 * from where it stands.
 */
function dropPulled(element: Element): void {
  element.removeAttribute('data-pulled')
}

/** The drawer has left, or came back before it had: the next one opens in place. */
function drawerSettled(): void {
  drawerPull.value = null
}

/** The panel has left: the next one opens in place. */
function panelSettled(): void {
  panelPull.value = null
}

useEventListener(frame, 'touchstart', frameTouchStart, { passive: true })
for (const type of ['pointerdown', 'pointermove', 'pointerup', 'pointercancel'] as const)
  useEventListener(frame, type, claimEdgePointer, { capture: true })
</script>

<template>
  <div ref="frame" class="flex h-full bg-surface-base text-fg">
    <Transition name="frame-scrim" @before-leave="dropPulled">
      <div
        v-if="(open || drawingIn) && phone"
        class="frame-scrim fixed inset-0 z-30 bg-black/50"
        :style="{ '--scrim-shade': scrimShade }"
        :data-pulled="pulling ? '' : undefined"
        @click="open = false"
      />
    </Transition>
    <!-- On a phone the sidebar is a drawer, which slides in over the scrim. -->
    <Transition name="frame-drawer" :css="phone" @after-leave="drawerSettled" @leave-cancelled="drawerSettled">
      <div
        v-show="open || drawingIn || !phone"
        ref="sidebar"
        data-region
        class="h-full shrink-0"
        :class="phone ? 'frame-drawer fixed inset-y-0 left-0 z-40 bg-surface-base pt-[var(--safe-top)] pb-[var(--safe-bottom)]' : ''"
        :style="{
          '--sidebar-width': phone ? DRAWER_WIDTH : `${shownWidth}px`,
          '--drawer-pull': drawerPull === null ? undefined : `${-drawerPull}px`,
        }"
        :data-pulled="pulling ? '' : undefined"
        @touchstart.passive="drawerOut.start"
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
      v-bind="{ [KEEPS_WINDOWS]: '' }"
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
    <Transition name="frame-aside" :css="phone" @before-leave="dropPulled" @after-leave="panelSettled">
      <div
        v-if="asideOpen && $slots.aside"
        data-region
        v-bind="{ [KEEPS_WINDOWS]: '' }"
        :class="phone ? 'frame-aside fixed inset-0 z-40' : 'h-full max-w-full shrink-0'"
        :style="phone
          ? { '--aside-pull': panelPull === null ? undefined : `${panelPull}px` }
          : { width: `${shownAsideWidth}px` }"
        :data-pulled="phone && panelPulling ? '' : undefined"
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

/* The panel follows a finger swiping it back, the conversation showing beneath it, and eases back or away from
   where the finger left it. Its shadow lies off the screen's edge until it is pulled. */
.frame-aside {
  translate: var(--aside-pull, 0px) 0;
  transition: translate 300ms cubic-bezier(0.32, 0.72, 0, 1);
  box-shadow: -4px 0 16px rgb(0 0 0 / 0.12);
}

.frame-aside[data-pulled] {
  transition: none;
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
  .frame-aside,
  .frame-aside-enter-active,
  .frame-aside-leave-active,
  .frame-scrim {
    transition: none;
  }
}
</style>

<script setup lang="ts">
import { t } from './locales/text'
import { computed, onBeforeUnmount, provide, ref } from 'vue'
import { useEventListener } from '@vueuse/core'
import { useEscapeClose } from '../composables/useEscapeClose'
import { PanelLeft } from '@lucide/vue'
import IconButton from '../ui/IconButton.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import { clampSize } from '../ui/resize-handle'
import { SCREEN_EDGE_PX, usePhone } from '../ui/phone'
import { createFingerDrag } from '../ui/finger-drag'
import { useBackClose } from '../overlay/back-close'
import { appOverlayStore } from '../overlay/appOverlay'
import { SIDEBAR_WIDTH } from './sidebar-width'
import { sidebarDrawerKey } from './sidebar-drawer'

/**
 * The app frame: the sidebar and the main pane, with a divider sizing the
 * sidebar. The sidebar keeps a px width, which the host keeps (v-model) and
 * hears `resizeEnd` for when it has settled, the moment to persist it.
 *
 * Each pane is a region (`data-region`, `ui/region.ts`): a menu opened from a
 * trigger in it grows toward its inside.
 *
 * On a computer the sidebar collapses (`collapsed`, v-model, which the host keeps in
 * the browser): it slides out to the left over 200 ms while the main pane widens with
 * it, and back in the same way, at once with reduced motion. A collapsed sidebar is
 * inert, and its divider is gone. The host's `sidebar` shortcut flips `collapsed`;
 * the buttons that do are `SidebarToggle.vue`'s, reaching the frame through
 * `sidebar-drawer.ts`.
 *
 * On a phone (`ui/phone.ts`) one pane shows at a time:
 *
 * - The sidebar is a drawer from the left, 85% of the screen and at most 320 px,
 *   over a scrim. A tap on the scrim closes it, as do a swipe left on it, Escape
 *   and Back. A swipe right from the screen's left edge (`SCREEN_EDGE_PX`) opens
 *   it, following the finger in as the closing swipe follows it out.
 * - Every swipe follows the finger and settles by one rule (`finger-drag.ts`).
 * - The frame shows one top bar there: a page with its own puts the sidebar's
 *   button in it (`useSidebarDrawer`), and the frame's own, the button and
 *   `label`, shows only while none does.
 *
 * The main pane stacks what it holds within itself (`isolate`): a page's own
 * layers, such as the conversation's dock over its transcript, stay under the
 * drawer and its scrim, whatever their z-index.
 */
const props = withDefaults(defineProps<{
  label?: string
  /** The keys of the `sidebar` action, shown in the collapse button's tip. */
  toggleShortcut?: string
}>(), { label: 'demi' })
const emit = defineEmits<{
  resizeEnd: [width: number]
}>()
const open = defineModel<boolean>('open', { default: false })
const collapsed = defineModel<boolean>('collapsed', { default: false })
const width = defineModel<number>('width', { default: SIDEBAR_WIDTH.default })
// A stored width outside today's bounds is shown at the bound, not rewritten.
const shownWidth = computed(() => clampSize(width.value, SIDEBAR_WIDTH))

const frame = ref<HTMLElement | null>(null)

const phone = usePhone()
/** Whether the sidebar is out of the way on a computer, which a phone never is: its drawer is `open`. */
const hidden = computed(() => collapsed.value && !phone.value)

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
  collapsed: () => collapsed.value,
  toggle: () => {
    collapsed.value = !collapsed.value
  },
  shortcut: () => props.toggleShortcut,
})

// The drawer is a layer Back closes.
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

const drags = [drawerOut, drawerIn]
onBeforeUnmount(() => {
  for (const drag of drags)
    drag.dispose()
})

/**
 * A finger down anywhere on the frame: on a phone, one at the screen's left
 * edge over the conversation, with the drawer closed, draws the drawer in. A
 * second finger takes any drag under way from the first.
 */
function frameTouchStart(event: TouchEvent): void {
  if (event.touches.length !== 1) {
    for (const drag of drags)
      drag.cancel()
    return
  }
  const x = event.touches[0]?.clientX
  if (phone.value && !open.value && x !== undefined && x <= SCREEN_EDGE_PX)
    drawerIn.start(event)
}

/**
 * The scrim starts to leave: it drops its following of the finger, which a
 * leaving node is no longer patched to drop, so it eases from where it stands.
 */
function dropPulled(element: Element): void {
  element.removeAttribute('data-pulled')
}

/** The drawer has left, or came back before it had: the next one opens in place. */
function drawerSettled(): void {
  drawerPull.value = null
}

useEventListener(frame, 'touchstart', frameTouchStart, { passive: true })
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
        data-region
        data-sidebar
        class="h-full shrink-0"
        :class="phone ? 'frame-drawer fixed inset-y-0 left-0 z-40 bg-surface-base pt-[var(--safe-top)] pb-[var(--safe-bottom)]' : 'frame-sidebar overflow-hidden'"
        :style="{
          '--sidebar-width': phone ? DRAWER_WIDTH : `${shownWidth}px`,
          '--sidebar-shown': hidden ? 0 : 1,
          '--drawer-pull': drawerPull === null ? undefined : `${-drawerPull}px`,
        }"
        :data-pulled="pulling ? '' : undefined"
        :inert="hidden"
        @touchstart.passive="drawerOut.start"
      >
        <!-- On a computer the sidebar keeps its width and slides out of the wrapper, which narrows to nothing. -->
        <div class="frame-sidebar-slide h-full">
          <slot name="sidebar" />
        </div>
      </div>
    </Transition>
    <ResizeHandle
      v-show="!hidden"
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
    <slot name="dialogs" />
  </div>
</template>

<style scoped>
/* One number, 1 shown to 0 collapsed, eases the wrapper's width and the sidebar's slide together. */
@property --sidebar-shown {
  syntax: '<number>';
  inherits: true;
  initial-value: 1;
}

.frame-sidebar {
  width: calc(var(--sidebar-width) * var(--sidebar-shown));
  transition: --sidebar-shown 200ms ease-out;
}

.frame-sidebar-slide {
  width: var(--sidebar-width);
}

.frame-sidebar .frame-sidebar-slide {
  translate: calc((1 - var(--sidebar-shown)) * -100%) 0;
}

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
  .frame-sidebar,
  .frame-drawer,
  .frame-scrim {
    transition: none;
  }
}
</style>

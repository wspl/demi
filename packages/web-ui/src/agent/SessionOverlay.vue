<script setup lang="ts">
import { t } from './locales/text'
import { t as shared } from '../locales/text'
import { computed, inject, provide, ref, watch } from 'vue'
import { useElementSize, useMutationObserver } from '@vueuse/core'
import { Ellipsis, X } from '@lucide/vue'
import { useStackedWindow } from '../ui/window-stack'
import { appOverlayStore } from '../overlay/appOverlay'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import { useEscapeClose } from '../composables/useEscapeClose'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import ResizeHandle from '../ui/ResizeHandle.vue'
import ContextMenu from '../ui/ContextMenu.vue'
import Tooltip from '../ui/Tooltip.vue'
import TabStrip from './TabStrip.vue'
import { usePhone } from '../ui/phone'
import { useBackClose } from '../overlay/back-close'
import { iconTabsKey } from './tab-strip'
import { DOCK_WINDOW_SHARE, dockWindowBounds, dockWindowHeight, dockWindowKey, dockWindowShare } from './dock-window'

/**
 * The window over a session the dock opens, such as Agents and Running: a
 * strip of tabs, the strip's controls, and the selected tab's content.
 *
 * Its tabs carry no controls of their own, so a click only selects one. What
 * acts on a tab, such as stopping it, is in the tab's menu, which a
 * right-click on it opens (`tabMenu`: the items for the tab `id`). What acts
 * on all of them is in the strip's menu (`stripMenu`), which the More button
 * at the strip's end and a right-click on the strip's empty area open; a
 * window with no such action has neither. The window is a region
 * (`ui/region.ts`): its menus grow toward its inside.
 *
 * It stands in the page's stack of windows with what it opens, such as the
 * media viewer (`turns.md`, the dock's chips under § Recovering an
 * unfinished turn; `window-stack.ts`): while it is the newest, a press
 * outside every window, the composer and the dock's empty space included,
 * closes it, though not one on its chip. It also closes by its chip, its close control, or Escape while the
 * focus is in it; the panel takes the focus from a click in it. Escape
 * leaves it open when something in it used the key, a menu is open, or a
 * newer window, such as the media viewer, stands in front of it and takes
 * the key first.
 *
 * Its top edge resizes it (`ResizeHandle`): the height is a share of the
 * session area's, kept by the `DockWindowArea` it stands in and shared by the
 * dock's windows. The least it takes is its strip, its bar (a `[data-window-bar]`
 * in its content, such as the status bar) and three lines of the transcript,
 * measured from the layout; the most is the room above the dock.
 *
 * On a phone it takes the whole screen, rising from the bottom, under a bar
 * of the strip, its controls and Close, with no edge to drag; Back closes it
 * there, as a page the phone pushed (`useBackClose`).
 */
const props = defineProps<{
  open: boolean
  /** The tabs change at once, with no motion, as `TabStrip`'s `still` says. */
  stillTabs?: boolean
}>()

const emit = defineEmits<{
  close: []
}>()

const slots = defineSlots<{
  /** The tabs; each opens its menu with `openTabMenu` on its `contextmenu`. */
  tabs(props: { openTabMenu: (event: MouseEvent, id: string) => void }): unknown
  /** The menu items for the tab `id`. */
  tabMenu?(props: { id: string }): unknown
  /** The menu items that act on every tab. */
  stripMenu?(props: Record<string, never>): unknown
  /** Controls after the More button, before the window's close. */
  trailing?(props: Record<string, never>): unknown
  default?(props: Record<string, never>): unknown
}>()

/** The tab whose menu is open. */
const menuTabId = ref<string | null>(null)
const tabMenu = useContextMenuOwner(() => {
  menuTabId.value = null
})
const stripMenu = useContextMenuOwner()

const root = ref<HTMLElement>()
const stacked = useStackedWindow({
  isOpen: () => props.open,
  element: () => root.value,
  otherLayerOpen: () => appOverlayStore.hasExclusive(),
  close: () => emit('close'),
})

const phone = usePhone()
// On a phone this window's tabs narrow to their icons (`TabItem`).
provide(iconTabsKey, true)

const space = inject(dockWindowKey)
if (!space) {
  throw new Error('A dock window stands in a DockWindowArea')
}
const stripRef = ref<HTMLElement>()
const bodyRef = ref<HTMLElement>()
const lineRef = ref<HTMLElement>()
const { height: header } = useElementSize(stripRef, { width: 0, height: 0 }, { box: 'border-box' })
const { height: line } = useElementSize(lineRef, { width: 0, height: 0 }, { box: 'border-box' })
const bar = ref(0)
function measureBar(): void {
  bar.value = bodyRef.value?.querySelector<HTMLElement>('[data-window-bar]')?.offsetHeight ?? 0
}
// The bar comes and goes with the tab shown, so the content is watched, not only measured once.
watch(bodyRef, measureBar, { flush: 'post' })
useMutationObserver(bodyRef, measureBar, { childList: true, subtree: true })
const bounds = computed(() => dockWindowBounds({ header: header.value, bar: bar.value, line: line.value }, space.room.value))
const height = computed(() => dockWindowHeight(space.share.value, space.area.value, bounds.value))
const defaultHeight = computed(() => dockWindowHeight(DOCK_WINDOW_SHARE.default, space.area.value, bounds.value))

useBackClose(() => props.open, () => emit('close'), () => phone.value)

// As the page's other windows: an Escape typed in a field (a terminal, the composer) is the field's.
useEscapeClose(
  () => props.open && !appOverlayStore.hasExclusive() && stacked.isNewest(),
  () => emit('close'),
  { ignoreFields: true },
)

function openTabMenu(event: MouseEvent, id: string): void {
  if (!slots.tabMenu) {
    return
  }
  menuTabId.value = id
  tabMenu.open(event)
}

/** A right-click on the strip's empty area, not on a tab or a control, opens the strip's menu. */
function openStripMenu(event: MouseEvent): void {
  if (!slots.stripMenu || (event.target instanceof Element && event.target.closest('[role=tab],[role=button],button'))) {
    return
  }
  stripMenu.open(event)
}
</script>

<template>
  <!-- On a phone the window leaves the dock's column for the whole screen. -->
  <Teleport to="body" :disabled="!phone">
  <Transition
    :enter-active-class="phone ? 'transition-[translate] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none' : 'origin-bottom transition-[opacity,scale] duration-150 ease-out motion-reduce:transition-none'"
    :leave-active-class="phone ? 'transition-[translate] duration-300 ease-[cubic-bezier(0.32,0.72,0,1)] motion-reduce:transition-none' : 'origin-bottom transition-[opacity,scale] duration-150 ease-out motion-reduce:transition-none'"
    :enter-from-class="phone ? 'translate-y-full' : 'opacity-0 scale-95'"
    :leave-to-class="phone ? 'translate-y-full' : 'opacity-0 scale-95'"
    appear
  >
    <div
      v-if="open"
      ref="root"
      class="pointer-events-auto origin-bottom"
      :class="phone ? 'fixed inset-0 z-40 pb-[var(--safe-bottom)]' : 'absolute inset-x-0 bottom-0 z-10'"
      :style="phone ? undefined : { height: `${height}px` }"
    >
      <!-- The root holds the handle too, so a press on the edge is a press in the window. It straddles the window's top, inset from its rounded corners, in a wrapper that takes no room. -->
      <div v-if="!phone" class="absolute inset-x-3 top-0 z-10 h-0">
        <ResizeHandle
          :model-value="height"
          :min="bounds.min"
          :max="bounds.max"
          :default-value="defaultHeight"
          orientation="horizontal"
          side="end"
          :label="t('panel.windowHeight')"
          @update:model-value="space.share.value = dockWindowShare($event, space.area.value)"
          @commit="space.commit(dockWindowShare($event, space.area.value))"
        />
      </div>
      <div
        data-region
        tabindex="-1"
        class="flex h-full min-h-0 flex-col overflow-hidden bg-surface outline-none"
        :class="phone ? '' : 'overlay-window rounded-xl'"
      >
        <!-- One line of the transcript's text, to measure the window's least height by. -->
        <span ref="lineRef" aria-hidden="true" class="pointer-events-none invisible absolute left-0 top-0 text-conversation">A</span>
        <div
          ref="stripRef"
          class="flex shrink-0 items-center gap-1 bg-surface-base px-1.5"
          :class="phone ? 'h-[calc(2.75rem+var(--safe-top))] pt-[var(--safe-top)]' : 'h-10'"
          data-touch-menu
          @contextmenu="openStripMenu"
        >
          <TabStrip class="flex-1" :still="stillTabs">
            <slot name="tabs" :open-tab-menu="openTabMenu" />
          </TabStrip>
          <Dropdown v-if="$slots.stripMenu" :overlay-store="appOverlayStore">
            <template #trigger="{ isOpen }">
              <Tooltip :content="t('panel.more')">
                <IconButton
                  :icon="Ellipsis"
                  size="sm"
                  variant="ghost"
                  :aria-label="t('panel.more')"
                  :pressed="isOpen"
                />
              </Tooltip>
            </template>
            <template #content="{ close }">
              <Menu @click="close">
                <slot name="stripMenu" />
              </Menu>
            </template>
          </Dropdown>
          <slot name="trailing" />
          <Tooltip :content="shared('common.close')">
            <IconButton
              :icon="X"
              size="sm"
              variant="ghost"
              :aria-label="shared('common.close')"
              @click="emit('close')"
            />
          </Tooltip>
        </div>
        <div ref="bodyRef" class="min-h-0 flex-1 overflow-hidden bg-surface">
          <slot />
        </div>
        <ContextMenu :overlay-store="appOverlayStore" :owner="tabMenu">
          <template #menu>
            <slot v-if="menuTabId !== null" name="tabMenu" :id="menuTabId" />
          </template>
        </ContextMenu>
        <ContextMenu :overlay-store="appOverlayStore" :owner="stripMenu">
          <template #menu>
            <slot name="stripMenu" />
          </template>
        </ContextMenu>
      </div>
    </div>
  </Transition>
  </Teleport>
</template>

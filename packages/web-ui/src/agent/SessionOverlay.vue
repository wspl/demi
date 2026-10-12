<script setup lang="ts">
import { ref } from 'vue'
import { Ellipsis, X } from '@lucide/vue'
import { useStackedWindow } from '../ui/window-stack'
import { appOverlayStore } from '../overlay/appOverlay'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import ContextMenu from '../ui/ContextMenu.vue'
import Tooltip from '../ui/Tooltip.vue'
import TabStrip from './TabStrip.vue'
import { usePhone } from '../ui/phone'
import { useBackClose } from '../overlay/back-close'

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
 * media viewer (`product.md`, the dock's chips under § Recovering an
 * unfinished turn; `window-stack.ts`): while it is the newest, a press
 * outside every window closes it, though not one in the composer or on its
 * chips. It also closes by its chip, its close control, or Escape while the
 * focus is in it; the panel takes the focus from a click in it. Escape
 * leaves it open when something in it used the key, a menu is open, or a
 * newer window, such as the media viewer, stands in front of it and takes
 * the key first.
 *
 * On a phone it takes the whole screen, rising from the bottom, under a bar
 * of the strip, its controls and Close; Back closes it there, as a page the
 * phone pushed (`useBackClose`).
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
useBackClose(() => props.open, () => emit('close'), () => phone.value)

function closeOnEscape(event: KeyboardEvent): void {
  if (event.key !== 'Escape' || event.defaultPrevented || event.isComposing || appOverlayStore.hasExclusive() || !stacked.isNewest())
    return
  event.preventDefault()
  emit('close')
}

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
      data-region
      tabindex="-1"
      class="pointer-events-auto flex min-h-0 flex-col overflow-hidden bg-surface outline-none"
      :class="phone ? 'fixed inset-0 z-40 pb-[var(--safe-bottom)]' : 'overlay-window absolute inset-0 z-10 origin-bottom rounded-xl'"
      @keydown="closeOnEscape"
    >
      <div
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
            <Tooltip content="More">
              <IconButton
                :icon="Ellipsis"
                size="sm"
                variant="ghost"
                aria-label="More"
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
        <Tooltip content="Close">
          <IconButton
            :icon="X"
            size="sm"
            variant="ghost"
            aria-label="Close"
            @click="emit('close')"
          />
        </Tooltip>
      </div>
      <div class="min-h-0 flex-1 overflow-hidden bg-surface">
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
  </Transition>
  </Teleport>
</template>

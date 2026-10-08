<script setup lang="ts">
import { ref } from 'vue'
import { Ellipsis, X } from '@lucide/vue'
import { onDismissOutside } from '../overlay/dismissOutside'
import { appOverlayStore } from '../overlay/appOverlay'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import Dropdown from '../ui/Dropdown.vue'
import IconButton from '../ui/IconButton.vue'
import Menu from '../ui/Menu.vue'
import Popover from '../ui/Popover.vue'
import Tooltip from '../ui/Tooltip.vue'
import TabStrip from './TabStrip.vue'

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
 */
const props = withDefaults(
  defineProps<{
    open: boolean
    /** False for a pinned catalog specimen: the window does not steal page clicks. */
    dismissOutside?: boolean
  }>(),
  {
    dismissOutside: true,
  },
)

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

const panelRef = ref<HTMLElement | null>(null)
/** The tab whose menu is open. */
const menuTabId = ref<string | null>(null)
const tabMenu = useContextMenuOwner(() => {
  menuTabId.value = null
})
const stripMenu = useContextMenuOwner()

onDismissOutside(
  () => (props.open && props.dismissOutside ? panelRef.value : null),
  () => emit('close'),
  { ignore: ['[data-overlay-panel]', '[data-session-overlay-toggle]'] },
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
  <Transition
    enter-active-class="origin-bottom transition-[opacity,scale] duration-150 ease-out motion-reduce:transition-none"
    leave-active-class="origin-bottom transition-[opacity,scale] duration-150 ease-out motion-reduce:transition-none"
    enter-from-class="opacity-0 scale-95"
    leave-to-class="opacity-0 scale-95"
    appear
  >
    <div
      v-if="open"
      ref="panelRef"
      data-region
      class="overlay-window pointer-events-auto absolute inset-0 z-10 flex min-h-0 origin-bottom flex-col overflow-hidden rounded-xl bg-surface"
    >
      <div class="flex h-10 shrink-0 items-center gap-1 bg-surface-base px-1.5" @contextmenu="openStripMenu">
        <TabStrip class="flex-1">
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
      <Popover
        :key="tabMenu.menuKey.value"
        :overlay-store="appOverlayStore"
        :is-open="tabMenu.isOpen.value && menuTabId !== null"
        :anchor-x="tabMenu.anchorX.value"
        :anchor-y="tabMenu.anchorY.value"
        :anchor-context-el="tabMenu.anchorContextEl.value"
        :offset="0"
        @close="tabMenu.close()"
      >
        <Menu v-if="menuTabId !== null" @click="tabMenu.close()">
          <slot name="tabMenu" :id="menuTabId" />
        </Menu>
      </Popover>
      <Popover
        :key="stripMenu.menuKey.value"
        :overlay-store="appOverlayStore"
        :is-open="stripMenu.isOpen.value"
        :anchor-x="stripMenu.anchorX.value"
        :anchor-y="stripMenu.anchorY.value"
        :anchor-context-el="stripMenu.anchorContextEl.value"
        :offset="0"
        @close="stripMenu.close()"
      >
        <Menu @click="stripMenu.close()">
          <slot name="stripMenu" />
        </Menu>
      </Popover>
    </div>
  </Transition>
</template>

<script setup lang="ts">
import { provide } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { useContextMenuOwner, type ContextMenuOwner } from '../composables/useContextMenuOwner'
import Popover from './Popover.vue'
import Menu from './Menu.vue'
import { menuRootKey } from './menu-context'

/**
 * The menu a right-click (or, on a phone, a long press) opens at the pointer;
 * on a phone it is a sheet. A chosen row dismisses it itself (`menuRootKey`),
 * so a handler never closes it.
 *
 * Two ways to open it. With a `trigger` slot, a right-click anywhere in the
 * trigger opens it, anchored to the trigger. Without one, the caller owns the
 * opening: it makes the `owner` (`useContextMenuOwner`) and calls
 * `owner.open(event, scrollEl)` from its own handlers, which is how many rows
 * share one menu or a click on some places is left to the browser's own menu;
 * `scrollEl` is the element the menu follows when it scrolls.
 */
const props = defineProps<{
  overlayStore: OverlayStore
  /** The caller's own opener; without one the trigger slot's right-click is the opener. */
  owner?: ContextMenuOwner
}>()

const emit = defineEmits<{
  open: [event: MouseEvent]
  close: []
}>()

defineSlots<{
  trigger?(): unknown
  /** The rows, in a Menu. */
  menu(): unknown
}>()

// Fixed for the component's life, as a composable's owner is.
const { isOpen, anchorX, anchorY, anchorContextEl, menuKey, open, close } =
  props.owner ?? useContextMenuOwner(() => emit('close'))

provide(menuRootKey, { dismiss: close })

function handleContextMenu(event: MouseEvent) {
  open(event)
  emit('open', event)
}
</script>

<template>
  <div v-if="$slots.trigger" data-touch-menu @contextmenu="handleContextMenu">
    <slot name="trigger" />
  </div>
  <Popover
    :key="menuKey"
    :overlay-store="overlayStore"
    :is-open="isOpen"
    :anchor-x="anchorX"
    :anchor-y="anchorY"
    :anchor-context-el="anchorContextEl"
    :offset="0"
    sheet
    @close="close"
  >
    <Menu>
      <slot name="menu" />
    </Menu>
  </Popover>
</template>

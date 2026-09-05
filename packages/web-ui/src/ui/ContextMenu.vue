<script setup lang="ts">
import { provide } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import { useContextMenuOwner } from '../composables/useContextMenuOwner'
import Popover from './Popover.vue'
import Menu from './Menu.vue'
import { menuRootKey } from './menu-context'

defineProps<{
  overlayStore: OverlayStore
}>()

const emit = defineEmits<{
  open: [event: MouseEvent]
  close: []
}>()

const { isOpen, anchorX, anchorY, anchorContextEl, menuKey, open, close } = useContextMenuOwner(() => emit('close'))

provide(menuRootKey, { dismiss: close })

function handleContextMenu(event: MouseEvent) {
  open(event)
  emit('open', event)
}
</script>

<template>
  <div @contextmenu="handleContextMenu">
    <slot name="trigger" />
    <Popover
      :key="menuKey"
      :overlay-store="overlayStore"
      :is-open="isOpen"
      :anchor-x="anchorX"
      :anchor-y="anchorY"
      :anchor-context-el="anchorContextEl"
      :offset="0"
      @close="close"
    >
      <Menu @click="close">
        <slot name="menu" />
      </Menu>
    </Popover>
  </div>
</template>

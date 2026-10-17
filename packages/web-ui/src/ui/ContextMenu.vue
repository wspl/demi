<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, provide, ref, watch } from 'vue'
import { appOverlayStore } from '../overlay/appOverlay'
import { useCopy, type CopySource } from '../composables/useCopy'
import { downloadUrl } from '../files/download'
import { isPhone } from './phone'
import Popover from './Popover.vue'
import Menu from './Menu.vue'
import MenuEntries from './MenuEntries.vue'
import ContextMenuLayer from './ContextMenuLayer.vue'
import { menuRootKey } from './menu-context'
import { closeContextMenu, installContextMenus, showContextMenu, shownContextMenu, type ContextMenuServices } from './context-menu'

/**
 * The page's one context menu, which the app mounts once at its root: it
 * installs the root handler (`installContextMenus`) and draws what it
 * opens, at the pointer, or on a phone as a sheet, standing on the dialog
 * or sheet it opened in (ContextMenuLayer). A chosen row dismisses it
 * itself (`menuRootKey`).
 */

let copied: CopySource
const copier = useCopy(() => copied)

function copy(source: CopySource): void {
  copied = source
  void copier.copy()
}

/** The picture as PNG, the one image type every browser's clipboard takes. */
async function pngOf(image: HTMLImageElement): Promise<Blob> {
  const blob = await (await fetch(image.currentSrc || image.src)).blob()
  if (blob.type === 'image/png')
    return blob
  const bitmap = await createImageBitmap(blob)
  const canvas = document.createElement('canvas')
  canvas.width = bitmap.width
  canvas.height = bitmap.height
  canvas.getContext('2d')?.drawImage(bitmap, 0, 0)
  bitmap.close()
  return new Promise((resolve, reject) => canvas.toBlob((png) => (png ? resolve(png) : reject(new Error('The picture could not be drawn.'))), 'image/png'))
}

const services: ContextMenuServices = {
  copy,
  // Written inside the choice that asks for it, as Safari requires, with the picture still to come.
  copyImage: (image) => copy(new ClipboardItem({ 'image/png': pngOf(image) })),
  save: (url, name) => downloadUrl(url, name),
  openLink: (url) => window.open(url, '_blank', 'noopener,noreferrer'),
}

let uninstall: (() => void) | null = null
onMounted(() => {
  uninstall = installContextMenus({ services, phone: isPhone, open: showContextMenu })
})
onBeforeUnmount(() => {
  uninstall?.()
  closeContextMenu()
})

const shown = shownContextMenu
const groups = computed(() => shown.value?.menu.groups() ?? [])
// Each opening is a new panel, placed where it opened, on the layer it opened in.
const opening = ref(0)
watch(shown, (next) => {
  if (next)
    opening.value++
})
const anchor = computed(() => (shown.value?.anchor instanceof HTMLElement ? shown.value.anchor : null))

provide(menuRootKey, { dismiss: closeContextMenu })
</script>

<template>
  <ContextMenuLayer :key="opening" :layer="shown?.layer ?? null">
    <Popover
      :overlay-store="appOverlayStore"
      :is-open="shown !== null"
      :anchor-x="shown?.x ?? 0"
      :anchor-y="shown?.y ?? 0"
      :anchor-context-el="anchor"
      :offset="0"
      sheet
      @close="closeContextMenu"
    >
      <Menu>
        <MenuEntries :groups="groups" />
      </Menu>
    </Popover>
  </ContextMenuLayer>
</template>

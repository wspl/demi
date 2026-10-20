<script setup lang="ts">
import { provide, shallowRef } from 'vue'
import { appOverlayStore } from '../overlay/appOverlay'
import Button from './Button.vue'
import Popover from './Popover.vue'
import { ICON_PX } from './icon-metrics'
import { menuRootKey } from './menu-context'
import type { ContextMenuEntry } from './context-menu'

/**
 * The row of a text selection's tools (SelectionToolbar), on the overlay surface, a
 * divider between tools as the system's edit menu has.
 *
 * - A tool with a submenu opens it from its button, on a phone as a sheet. `holding`
 *   says while one is open, so the owner keeps the row where it is.
 * - The row takes no press from the selection: a press on it keeps the text selected.
 */
defineProps<{
  entries: readonly ContextMenuEntry[]
}>()

const emit = defineEmits<{
  holding: [holding: boolean]
}>()

/** The tool whose submenu is open. */
const opened = shallowRef<{ entry: ContextMenuEntry; button: HTMLElement } | null>(null)

/** A tool chosen: one with a submenu opens it, any other acts. */
function choose(entry: ContextMenuEntry, event: Event): void {
  if (entry.disabled)
    return
  if (entry.submenu) {
    opened.value = { entry, button: event.currentTarget as HTMLElement }
    emit('holding', true)
    return
  }
  void entry.run?.()
}

function closeSubmenu(): void {
  opened.value = null
  emit('holding', false)
}
// A row chosen in the submenu ends it, as a menu's row does.
provide(menuRootKey, { dismiss: closeSubmenu })
</script>

<template>
  <div
    class="overlay-shell flex max-w-[calc(100vw-1rem)] items-center gap-0.5 overflow-x-auto rounded-lg p-1"
    @mousedown.prevent
    @touchstart.prevent
  >
    <template v-for="(entry, index) in entries" :key="entry.label">
      <div v-if="index > 0" class="h-4 w-px shrink-0 bg-line-subtle" aria-hidden="true" />
      <Button
        class="shrink-0"
        size="sm"
        variant="ghost"
        :pressed="opened?.entry === entry"
        :disabled="entry.disabled"
        :disabled-reason="entry.disabledReason"
        @pointerup="(event: PointerEvent) => event.pointerType === 'touch' && choose(entry, event)"
        @click="(event: MouseEvent) => choose(entry, event)"
      >
        <component :is="entry.icon" v-if="entry.icon" :size="ICON_PX.in28" aria-hidden="true" />
        {{ entry.label }}
      </Button>
    </template>
  </div>
  <Popover
    v-if="opened?.entry.submenu"
    :overlay-store="appOverlayStore"
    is-open
    :anchor-el="opened.button"
    sheet
    @close="closeSubmenu"
  >
    <component :is="opened.entry.submenu" />
  </Popover>
</template>

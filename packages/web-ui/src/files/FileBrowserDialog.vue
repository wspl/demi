<script setup lang="ts">
import { computed } from 'vue'
import Dialog from '../ui/Dialog.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import FileBrowser from './FileBrowser.vue'
import type { FileBrowserHost, FileBrowserMode, FileBrowserPlaceGroup, FileBrowserSource } from './types'

/**
 * The file browser as a modal: a title over the browser, which fills a fixed height so
 * the list has room whatever the folder holds. Cancel and the corner both close.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  mode: FileBrowserMode
  source: FileBrowserSource
  /** Defaults to `Open file` or `Select folder`. */
  title?: string
  /** One line under the title: what the chosen path is for. */
  description?: string
  initialPath?: string
  places?: FileBrowserPlaceGroup[]
  hosts?: FileBrowserHost[]
  hostId?: string
  confirmLabel?: string
  confirmDisabled?: boolean
}>()

const emit = defineEmits<{
  close: []
  select: [path: string]
  'update:hostId': [id: string]
}>()

const showHidden = defineModel<boolean>('showHidden', { default: false })

const title = computed(() => props.title ?? (props.mode === 'file' ? 'Open file' : 'Select folder'))
</script>

<template>
  <Dialog :is-open="isOpen" :overlay-store="overlayStore" size="lg" :label="title" @close="emit('close')">
    <div class="flex h-[32rem] min-h-0 flex-col">
      <header class="select-none px-5 pb-2 pt-5 pr-12">
        <h3 class="text-[15px] font-medium text-fg-emphasis">{{ title }}</h3>
        <p v-if="description" class="mt-0.5 text-[13px] leading-5 text-fg-muted">{{ description }}</p>
      </header>
      <FileBrowser
        v-model:show-hidden="showHidden"
        :mode="mode"
        :source="source"
        :initial-path="initialPath"
        :places="places"
        :hosts="hosts"
        :host-id="hostId"
        :confirm-label="confirmLabel"
        :confirm-disabled="confirmDisabled"
        @select="emit('select', $event)"
        @cancel="emit('close')"
        @update:host-id="emit('update:hostId', $event)"
      />
    </div>
  </Dialog>
</template>

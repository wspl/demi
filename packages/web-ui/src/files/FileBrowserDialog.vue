<script setup lang="ts">
import { computed } from 'vue'
import { X } from '@lucide/vue'
import Dialog from '../ui/Dialog.vue'
import IconButton from '../ui/IconButton.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import FileBrowser from './FileBrowser.vue'
import type {
  FileBrowserHost,
  FileBrowserMode,
  FileBrowserPlaceGroup,
  FileBrowserSource,
} from './types'

/**
 * The file browser as a modal: a plain title bar over the browser, which fills a
 * fixed height so the list has room whatever the folder holds. Cancel and the
 * corner both close.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  mode: FileBrowserMode
  source: FileBrowserSource
  /** Defaults to `Open file` or `Select folder`. */
  title?: string
  initialPath?: string
  places?: FileBrowserPlaceGroup[]
  hosts?: FileBrowserHost[]
  hostId?: string
  confirmLabel?: string
  confirmDisabled?: boolean
  confirmPending?: boolean
}>()

const emit = defineEmits<{
  close: []
  select: [path: string]
  'update:hostId': [id: string]
}>()

const showHidden = defineModel<boolean>('showHidden', { default: false })

const title = computed(
  () => props.title ?? (props.mode === 'file' ? 'Open file' : 'Select folder'),
)
</script>

<template>
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    size="lg"
    :label="title"
    hide-close
    @close="emit('close')"
  >
    <div class="flex h-[32rem] min-h-0 flex-col">
      <!-- The title bar carries its own close so the button centers on the bar. -->
      <header
        class="flex h-11 shrink-0 select-none items-center justify-between gap-2 border-b border-line pl-4 pr-2"
      >
        <h3 class="truncate text-[15px] font-medium text-fg-emphasis">
          {{ title }}
        </h3>
        <IconButton
          :icon="X"
          variant="ghost"
          aria-label="Close"
          @click="emit('close')"
        />
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
        :confirm-pending="confirmPending"
        @select="emit('select', $event)"
        @cancel="emit('close')"
        @update:host-id="emit('update:hostId', $event)"
      />
    </div>
  </Dialog>
</template>

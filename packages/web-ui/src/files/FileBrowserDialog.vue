<script setup lang="ts">
import { computed } from 'vue'
import type { Operation } from '../ui/operations'
import { X } from '@lucide/vue'
import Dialog from '../ui/Dialog.vue'
import IconButton from '../ui/IconButton.vue'
import type { OverlayStore } from '../overlay/overlayStore'
import FileBrowser from './FileBrowser.vue'
import type {
  FileBrowserDevice,
  FileBrowserMode,
  FileBrowserPlaceGroup,
  FileBrowserSource,
} from './types'
import type { HeadlineText, TitleText } from '../ui/ui-text'
import { t as shared } from '../locales/text'
import { t } from './locales/text'

/**
 * The file browser as a modal: a plain title bar over the file browser, which
 * fills a fixed height so the list has room whatever the folder holds. Cancel
 * and the corner both close.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  mode: FileBrowserMode
  source: FileBrowserSource
  /** Defaults to `Open File` or `Select Folder`. */
  title?: HeadlineText
  initialPath?: string
  places?: FileBrowserPlaceGroup[]
  devices?: FileBrowserDevice[]
  deviceId?: string
  confirmLabel?: TitleText
  confirmDisabled?: boolean
  /** The operation the caller starts with what is chosen, which the confirm button shows (`FileBrowser.vue`). */
  confirmOperation?: Operation
}>()

const emit = defineEmits<{
  close: []
  select: [path: string]
  'update:deviceId': [id: string]
}>()

const showHidden = defineModel<boolean>('showHidden', { default: false })

const title = computed(
  () => props.title ?? (props.mode === 'file' ? t('browser.openFile') : t('browser.selectFolder')),
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
          :aria-label="shared('common.close')"
          @click="emit('close')"
        />
      </header>
      <FileBrowser
        v-model:show-hidden="showHidden"
        :mode="mode"
        :source="source"
        :initial-path="initialPath"
        :places="places"
        :devices="devices"
        :device-id="deviceId"
        :confirm-label="confirmLabel"
        :confirm-disabled="confirmDisabled"
        :confirm-operation="confirmOperation"
        @select="emit('select', $event)"
        @cancel="emit('close')"
        @update:device-id="emit('update:deviceId', $event)"
      />
    </div>
  </Dialog>
</template>

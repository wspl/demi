<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import FileBrowserDialog from './FileBrowserDialog.vue'
import { t } from './locales/text'
import type {
  FileBrowserDevice,
  FileBrowserSource,
  FileBrowserPlaceGroup,
} from './types'

export interface RemoteFileDevice extends FileBrowserDevice {
  source: FileBrowserSource
  places: FileBrowserPlaceGroup[]
  cwd?: string
}
const props = defineProps<{
  overlayStore: OverlayStore
  devices: RemoteFileDevice[]
}>()
const emit = defineEmits<{
  select: [
    file: {
      deviceId: string
      device: string
      path: string
    },
  ]
}>()
const deviceId = ref<string | null>(null)
const isOpen = ref(false)
const device = computed(() => props.devices.find((item) => item.id === deviceId.value))
function open(): void {
  deviceId.value = props.devices[0]?.id ?? null
  isOpen.value = deviceId.value !== null
}
function select(path: string): void {
  if (device.value) {
    emit('select', {
      deviceId: device.value.id,
      device: device.value.label,
      path,
    })
  }
  isOpen.value = false
}
defineExpose({ open })
</script>
<template>
  <FileBrowserDialog
    v-if="device"
    :is-open="isOpen"
    :overlay-store="overlayStore"
    mode="file"
    :title="t('browser.attachRemoteFile')"
    :source="device.source"
    :initial-path="device.cwd"
    :places="device.places"
    :devices="devices"
    :device-id="deviceId ?? undefined"
    :confirm-label="t('browser.attach')"
    @select="select"
    @close="isOpen = false"
    @update:device-id="deviceId = $event"
  />
</template>

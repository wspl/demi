<script setup lang="ts">
import { computed, ref } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import FileBrowserDialog from './FileBrowserDialog.vue'
import type {
  FileBrowserHost,
  FileBrowserSource,
  FileBrowserPlaceGroup,
} from './types'

export interface RemoteFileHost extends FileBrowserHost {
  source: FileBrowserSource
  places: FileBrowserPlaceGroup[]
  cwd?: string
}
const props = defineProps<{
  overlayStore: OverlayStore
  hosts: RemoteFileHost[]
}>()
const emit = defineEmits<{
  select: [
    file: {
      deviceId: string
      host: string
      path: string
    },
  ]
}>()
const hostId = ref<string | null>(null)
const isOpen = ref(false)
const host = computed(() => props.hosts.find((item) => item.id === hostId.value))
function open(): void {
  hostId.value = props.hosts[0]?.id ?? null
  isOpen.value = hostId.value !== null
}
function select(path: string): void {
  if (host.value) {
    emit('select', {
      deviceId: host.value.id,
      host: host.value.label,
      path,
    })
  }
  isOpen.value = false
}
defineExpose({ open })
</script>
<template>
  <FileBrowserDialog
    v-if="host"
    :is-open="isOpen"
    :overlay-store="overlayStore"
    mode="file"
    title="Attach remote file"
    :source="host.source"
    :initial-path="host.cwd"
    :places="host.places"
    :hosts="hosts"
    :host-id="hostId ?? undefined"
    confirm-label="Attach"
    @select="select"
    @close="isOpen = false"
    @update:host-id="hostId = $event"
  />
</template>

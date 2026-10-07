<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import RenameDialog from '../ui/RenameDialog.vue'

/**
 * A paired device's new name, which every page of the user's then shows; the
 * names conversations gave the device as an attached host stay theirs
 * (`web-api.md` § Workspaces, devices, and attached hosts). The Cloud keeps
 * its name and is never asked about.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The device's current name. */
  name: string
  /** The most characters a device's name has; null for no limit. */
  maxLength: number | null
}>()
const emit = defineEmits<{
  close: []
  rename: [name: string]
}>()
</script>

<template>
  <RenameDialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    title="Rename Device"
    label="Device name"
    :name="name"
    :max-length="maxLength"
    @close="emit('close')"
    @rename="emit('rename', $event)"
  />
</template>

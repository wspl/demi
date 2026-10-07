<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import ConfirmDialog from '../ui/ConfirmDialog.vue'

/**
 * The confirmation before a device is revoked: the device leaves the user's
 * devices and its connected runner removes itself, and the projects on it go
 * with it, named here, while their files and conversations stay
 * (`web-api.md` § Workspaces, devices, and attached hosts).
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The device's name. */
  device: string
  /** The names of the projects on the device, which go with it. */
  projects: readonly string[]
}>()
const emit = defineEmits<{
  close: []
  revoke: []
}>()
</script>

<template>
  <ConfirmDialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    title="Revoke this device?"
    action="Revoke"
    :goes="projects"
    @close="emit('close')"
    @confirm="emit('revoke')"
  >
    <p>
      “{{ device }}” leaves your devices. If its runner is connected, it
      removes itself from the device.
    </p>
    <p v-if="projects.length">
      {{ projects.length === 1 ? 'This project goes' : 'These projects go' }}
      with it. Their files stay on the device, and their conversations stay
      in the sidebar.
    </p>
  </ConfirmDialog>
</template>

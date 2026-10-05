<script setup lang="ts">
import type { OverlayStore } from '../overlay/overlayStore'
import Button from '../ui/Button.vue'
import Dialog from '../ui/Dialog.vue'

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
  <Dialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    label="Revoke this device?"
    @close="emit('close')"
  >
    <div class="flex flex-col gap-4 p-5">
      <h3 class="pr-8 text-[15px] font-medium text-fg-emphasis">
        Revoke this device?
      </h3>
      <p class="text-[13px] leading-5 text-fg-muted">
        “{{ device }}” leaves your devices. If its runner is connected, it
        removes itself from the device.
      </p>
      <template v-if="projects.length">
        <p class="text-[13px] leading-5 text-fg-muted">
          {{ projects.length === 1 ? 'This project goes' : 'These projects go' }}
          with it. Their files stay on the device, and their conversations stay
          in the sidebar.
        </p>
        <ul class="flex flex-col gap-1 pl-4 text-[13px] leading-5 text-fg-emphasis">
          <li v-for="(project, index) in projects" :key="index" class="list-disc">
            {{ project }}
          </li>
        </ul>
      </template>
      <div class="flex justify-end gap-2">
        <Button @click="emit('close')">Cancel</Button>
        <Button variant="danger" @click="emit('revoke')">Revoke</Button>
      </div>
    </div>
  </Dialog>
</template>

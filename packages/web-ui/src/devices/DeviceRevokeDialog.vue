<script setup lang="ts">
import { t } from './locales/text'
import type { OverlayStore } from '../overlay/overlayStore'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import type { Action, Operation } from '../ui/operations'
import DirectoryLabel from '../files/DirectoryLabel.vue'

/**
 * The confirmation before a device is revoked: the device leaves the user's
 * devices and its connected runner removes itself, and the projects on it go
 * with it, named here, while their files and conversations stay.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The device's name. */
  device: string
  /** The directories of the projects on the device, which go with them, read by name and location. */
  projects: readonly string[]
  /** The device's home, which `~` writes in each location. */
  home?: string | null
  onRevoke?: Action
  /** What Revoke does, to which device (`operations.ts`). */
  operation?: Operation
}>()
const emit = defineEmits<{
  close: []
}>()
</script>

<template>
  <ConfirmDialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :title="t('revoke.title')"
    :action="t('revoke.action')"
    :operation="operation"
    :goes="projects"
    @close="emit('close')"
    @confirm="onRevoke?.()"
  >
    <template #entry="{ entry }">
      <DirectoryLabel :path="entry" :home="home" />
    </template>
    <p>
      {{ t('revoke.body', { device }) }}
    </p>
    <p v-if="projects.length">
      {{ t('revoke.projects', {}, projects.length) }}
    </p>
  </ConfirmDialog>
</template>

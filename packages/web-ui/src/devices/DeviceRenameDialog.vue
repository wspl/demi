<script setup lang="ts">
import { t } from './locales/text'
import type { OverlayStore } from '../overlay/overlayStore'
import RenameDialog from '../ui/RenameDialog.vue'

/**
 * A paired device's new name, which every page of the user's then shows,
 * the conversations it is attached to included (`sessions-and-targets.md`
 * § Attached hosts). The Cloud keeps its name and is never asked about.
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
    :title="t('rename.title')"
    :label="t('rename.label')"
    :name="name"
    :max-length="maxLength"
    @close="emit('close')"
    @rename="emit('rename', $event)"
  />
</template>

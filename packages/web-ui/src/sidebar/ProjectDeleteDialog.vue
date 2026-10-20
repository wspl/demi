<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import type { OverlayStore } from '../overlay/overlayStore'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import type { Action } from '../ui/operations'
import { projectTarget } from './project-hosts'

/**
 * The question before a project is deleted: the title names it, the body says
 * that its conversations are archived, a sentence a project without one leaves
 * out, and that its files on its host stay. Delete is the danger button and
 * shows the deletion while it runs, as the project's row does.
 */
defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The project's id, the target of the deletion. */
  id: string
  /** The project's name; content, not UI text. */
  name: string
  /** How many of its conversations the deletion archives. */
  count: number
  /** The name of the host its files are on. */
  host: string
  onDelete?: Action
}>()

const emit = defineEmits<{
  close: []
}>()
</script>

<template>
  <ConfirmDialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :title="t('deleteProject.title', { name })"
    :action="uiText('common.delete')"
    :operation="{ action: 'remove', target: projectTarget(id) }"
    @close="emit('close')"
    @confirm="onDelete?.()"
  >
    <p v-if="count > 0">
      {{ t('deleteProject.archived', { count }, count) }}
    </p>
    <p>
      {{ t('deleteProject.files', { host }) }}
    </p>
  </ConfirmDialog>
</template>

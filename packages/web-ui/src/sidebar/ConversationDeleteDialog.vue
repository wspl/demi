<script setup lang="ts">
import { computed } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import type { Action } from '../ui/operations'

/**
 * The question before conversations are deleted for good (`product.md`
 * § Conversations and projects): one is named in the title, several are
 * counted. The body says what goes with them, that it cannot be undone, and
 * that the files they changed in their projects stay. Delete is the
 * danger button; Cancel, the default, closes.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The titles of the conversations to delete; content, not UI text. */
  titles: readonly string[]
  onDelete?: Action
}>()

const emit = defineEmits<{
  close: []
}>()

const one = computed(() => props.titles.length === 1)
const title = computed(() =>
  one.value ? `Delete “${props.titles[0]}”?` : `Delete ${props.titles.length} Conversations?`,
)
</script>

<template>
  <ConfirmDialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :title="title"
    action="Delete"
    @close="emit('close')"
    @confirm="onDelete?.()"
  >
    <p v-if="one">
      Its messages and files are deleted. This cannot be undone.
    </p>
    <p v-else>
      Their messages and files are deleted. This cannot be undone.
    </p>
    <p>
      {{ one ? 'Files it changed stay in the project.' : 'Files they changed stay in their projects.' }}
    </p>
  </ConfirmDialog>
</template>

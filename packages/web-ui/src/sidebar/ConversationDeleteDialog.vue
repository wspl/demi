<script setup lang="ts">
import { computed } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import ConfirmDialog from '../ui/ConfirmDialog.vue'

/**
 * The question before conversations are deleted for good (`product.md`
 * § Conversations and projects): one is named in the title, several are
 * counted. The body says what goes with them, that it cannot be undone, and
 * that the files they changed in their projects stay. Delete is the
 * destructive button; Cancel closes.
 */
const props = defineProps<{
  isOpen: boolean
  overlayStore: OverlayStore
  /** The titles of the conversations to delete; content, not UI text. */
  titles: readonly string[]
}>()

const emit = defineEmits<{
  close: []
  delete: []
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
    @confirm="emit('delete')"
  >
    <p v-if="one">
      The conversation is deleted with its messages and the files it holds.
      This cannot be undone.
    </p>
    <p v-else>
      The conversations are deleted with their messages and the files they
      hold. This cannot be undone.
    </p>
    <p>
      {{ one ? 'Files it changed in its project stay.' : 'Files they changed in their projects stay.' }}
    </p>
  </ConfirmDialog>
</template>

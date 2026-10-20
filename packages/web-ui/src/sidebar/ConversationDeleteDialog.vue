<script setup lang="ts">
import { t as uiText } from '../locales/text'
import { t } from './locales/text'
import { computed } from 'vue'
import type { OverlayStore } from '../overlay/overlayStore'
import ConfirmDialog from '../ui/ConfirmDialog.vue'
import type { Action } from '../ui/operations'

/**
 * The question before conversations are deleted for good: one is named in the
 * title, several are counted. The body says what goes with them, that it cannot
 * be undone, and that the files they changed in their projects stay. Delete is
 * the danger button; Cancel, the default, closes.
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
  one.value ? t('delete.title', { title: props.titles[0] ?? '' }) : t('delete.titleMany', { count: props.titles.length }),
)
</script>

<template>
  <ConfirmDialog
    :is-open="isOpen"
    :overlay-store="overlayStore"
    :title="title"
    :action="uiText('common.delete')"
    @close="emit('close')"
    @confirm="onDelete?.()"
  >
    <p v-if="one">
      {{ t('delete.one') }}
    </p>
    <p v-else>
      {{ t('delete.many') }}
    </p>
    <p>
      {{ one ? t('delete.filesOne') : t('delete.filesMany') }}
    </p>
  </ConfirmDialog>
</template>

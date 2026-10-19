<script setup lang="ts">
import { computed, ref } from 'vue'
import { t } from '../locales/text'
import { t as shared } from '../../locales/text'
import ErrorNotice, { type NoticeAction } from '../../ui/ErrorNotice.vue'
import ConfirmDialog from '../../ui/ConfirmDialog.vue'
import { appOverlayStore } from '../../overlay/appOverlay'
import type { PendingSubmissionState } from '../types'

/**
 * The failure under a message that was not delivered (`turns.md` § Following
 * the agent's work): why, with Retry, Edit and Delete, as Messages offers
 * Try Again and Delete for one it could not send. A refusal sending again
 * cannot change offers Edit first and no Retry. Delete asks first, since the
 * text goes with the message.
 */
const props = defineProps<{
  submission: PendingSubmissionState
}>()

const emit = defineEmits<{
  retry: []
  edit: []
  delete: []
}>()

const confirming = ref(false)

const actions = computed<NoticeAction[]>(() => [
  ...(props.submission.retryable === false ? [] : [{ label: shared('common.retry'), run: () => emit('retry') }]),
  { label: t('transcript.user.edit'), run: () => emit('edit') },
  { label: shared('common.delete'), run: () => { confirming.value = true } },
])

function confirmDelete(): void {
  confirming.value = false
  emit('delete')
}
</script>

<template>
  <ErrorNotice
    :label="t('transcript.user.notDelivered')"
    :detail="submission.error"
    :request-id="submission.requestId ?? undefined"
    :actions="actions"
  />
  <ConfirmDialog
    :is-open="confirming"
    :overlay-store="appOverlayStore"
    :title="t('transcript.user.deleteTitle')"
    :action="shared('common.delete')"
    @close="confirming = false"
    @confirm="confirmDelete"
  >
    <p>{{ t('transcript.user.deleteBody') }}</p>
  </ConfirmDialog>
</template>

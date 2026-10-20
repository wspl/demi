<script setup lang="ts">
import { t } from '../locales/text'
import ErrorNotice from '../../ui/ErrorNotice.vue'
import type { PendingSubmissionState } from '../types'

/**
 * The failure under a message that was not delivered: why, with the request ID,
 * as every failure says it, and nothing to act on. The message's own hover
 * actions offer Edit and Delete, and the dock's Retry sends it again.
 */
defineProps<{
  submission: PendingSubmissionState
}>()
</script>

<template>
  <ErrorNotice
    :label="t('transcript.user.notDelivered')"
    :detail="submission.error"
    :request-id="submission.requestId ?? undefined"
  />
</template>

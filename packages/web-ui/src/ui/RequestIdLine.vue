<script setup lang="ts">
import CopyButton from './CopyButton.vue'
import { t } from './locales/text'

/**
 * The request an error came from, in secondary text under the error:
 * *Request ID: req_…* and a Copy button, so the user can paste the ID into
 * a message to whoever runs the server, who finds the request's lines in the
 * server log by it (`backend.md` § Request IDs and the server log). Its
 * color is the place's secondary text, which the caller gives.
 */
defineProps<{
  requestId: string
}>()
</script>

<template>
  <div class="flex min-w-0 items-center gap-0.5 text-detail leading-4">
    <span class="min-w-0 truncate select-text">{{ t('requestId', { id: requestId }) }}</span>
    <CopyButton :source="requestId" size="xs" />
  </div>
</template>

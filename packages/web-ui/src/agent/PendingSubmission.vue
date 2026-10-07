<script setup lang="ts">
import { Clock } from '@lucide/vue'
import ErrorNotice from '../ui/ErrorNotice.vue'
import UserBlock from './blocks/UserBlock.vue'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it, shown as the
 * message it will become: the same bubble in the same place, its attachments
 * among the media. Its delivery is part of the wait for the answer, which the
 * tail row tells. A message the page cannot send yet, while the backend
 * cannot be reached, says it waits; it never shows as failed for that. Only
 * a failed delivery shows here as one, in the transcript flow with Retry.
 */
defineProps<PendingSubmissionState>()
const emit = defineEmits<{ retry: [] }>()
</script>

<template>
  <div aria-live="polite">
    <UserBlock
      :content="text ? [{ type: 'text', text }] : []"
      :attachments="attachments"
      :editable="false"
    />
    <div
      v-if="waiting && !error"
      class="-mt-1 flex select-none items-center justify-end gap-1 px-[var(--agent-pad-x,2rem)] text-[12px] leading-4 text-fg-subtle"
    >
      <Clock :size="12" aria-hidden="true" />
      Waiting to send
    </div>
    <div v-if="error" class="mt-2 px-[var(--agent-pad-x,2rem)]">
      <ErrorNotice
        label="This message was not delivered."
        :detail="error"
        action="Retry"
        @action="emit('retry')"
      />
    </div>
  </div>
</template>

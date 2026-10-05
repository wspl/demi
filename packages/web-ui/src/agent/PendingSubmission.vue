<script setup lang="ts">
import ErrorNotice from '../ui/ErrorNotice.vue'
import UserBlock from './blocks/UserBlock.vue'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it, shown as the
 * message it will become: the same bubble in the same place, its attachments
 * among the media. Its delivery is part of the wait for the answer, which the
 * tail row tells; only a failed delivery shows here, in the transcript flow
 * with Retry.
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

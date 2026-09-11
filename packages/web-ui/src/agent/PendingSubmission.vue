<script setup lang="ts">
import ErrorNotice from '../ui/ErrorNotice.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import UserBlock from './blocks/UserBlock.vue'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it, shown as the
 * message it will become: the same bubble, its attachments among the media.
 * Under it, the sending state or, when delivery failed, the failure in the
 * transcript flow with Retry.
 */
defineProps<PendingSubmissionState>()
const emit = defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="shrink-0 py-3" aria-live="polite">
    <UserBlock
      :content="text ? [{ type: 'text', text }] : []"
      :attachments="attachments"
      :editable="false"
    />
    <p
      v-if="sending"
      class="mt-2 flex items-center justify-end gap-2 px-[var(--agent-pad-x,2rem)] text-chrome text-fg-muted"
    >
      <IndeterminateSpinner /> Sending…
    </p>
    <div v-else-if="error" class="mt-2 px-[var(--agent-pad-x,2rem)]">
      <ErrorNotice
        label="This message was not delivered."
        :detail="error"
        action="Retry"
        @action="emit('retry')"
      />
    </div>
  </div>
</template>

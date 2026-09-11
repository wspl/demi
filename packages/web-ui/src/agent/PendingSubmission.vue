<script setup lang="ts">
import ErrorNotice from '../ui/ErrorNotice.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import AttachmentTile from './AttachmentTile.vue'
import UserBlock from './blocks/UserBlock.vue'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it: the bubble, its
 * attachments as tiles, and under them either the sending state or, when
 * delivery failed, the failure in the transcript flow with Retry.
 */
defineProps<PendingSubmissionState>()
const emit = defineEmits<{ retry: [] }>()
</script>

<template>
  <div class="shrink-0 py-3" aria-live="polite">
    <UserBlock
      :content="text ? [{ type: 'text', text }] : []"
      :editable="false"
    />
    <div
      v-if="fileNames.length"
      class="mt-1.5 flex flex-wrap justify-end gap-1.5 px-[var(--agent-pad-x,2rem)]"
    >
      <AttachmentTile v-for="name in fileNames" :key="name" :name="name" />
    </div>
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

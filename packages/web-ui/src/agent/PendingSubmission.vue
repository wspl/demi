<script setup lang="ts">
import InlineError from '../ui/InlineError.vue'
import IndeterminateSpinner from '../ui/IndeterminateSpinner.vue'
import UserBlock from './blocks/UserBlock.vue'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it. While sending it
 * shows progress; when delivery fails the exact text and files stay on screen
 * with the failure under them, so nothing has to be typed again.
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
    <p v-if="fileNames.length" class="mt-1 px-[var(--agent-pad-x,2rem)] text-right text-chrome text-fg-muted">
      {{ fileNames.join(', ') }}
    </p>
    <div class="mt-2 flex justify-end px-[var(--agent-pad-x,2rem)]">
      <span
        v-if="sending"
        class="flex items-center gap-2 text-chrome text-fg-muted"
        ><IndeterminateSpinner /> Sending…</span
      >
      <InlineError
        v-else-if="error"
        class="max-w-[80%]"
        :message="error"
        action="Retry"
        @action="emit('retry')"
      />
    </div>
  </div>
</template>

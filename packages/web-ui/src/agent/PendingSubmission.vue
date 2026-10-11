<script setup lang="ts">
import { computed } from 'vue'
import ErrorNotice from '../ui/ErrorNotice.vue'
import UserBlock from './blocks/UserBlock.vue'
import { sentMarkdown } from './message-input/message-content'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it, shown as the
 * message it will become: the same bubble in the same place, at its height,
 * its attachments among the media. Its delivery is part of the wait for the
 * answer, which the tail row tells, and a lost connection the Connecting row
 * or the connection banner; the bubble itself never says either, so it does
 * not change height when the message arrives (`product.md` § Following the
 * agent's work). Only a failed delivery shows here, in the transcript flow
 * with Retry. A steer or a queued message shows as the one it will be from
 * the send, with its line and its controls (`product.md` § Steer or queue),
 * which the host keeps as the user's wish until the session holds it; one
 * that was not delivered is a message to try again, without them.
 */
const props = defineProps<PendingSubmissionState & {
  /** What it will be when it waits for the agent, which its line says (`waitingAs`). */
  pending?: 'steer' | 'queued'
}>()
const emit = defineEmits<{ retry: []; remove: []; sendNow: [] }>()
/** The text as the message the server confirms holds it. */
const markdown = computed(() => sentMarkdown(props.text, props.attachments.length))
</script>

<template>
  <div aria-live="polite">
    <UserBlock
      :content="markdown ? [{ type: 'text', text: markdown }] : []"
      :attachments="attachments"
      :editable="false"
      :pending="pending"
      @remove="emit('remove')"
      @send-now="emit('sendNow')"
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

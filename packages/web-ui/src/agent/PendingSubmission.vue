<script setup lang="ts">
import { computed } from 'vue'
import UserBlock from './blocks/UserBlock.vue'
import { sentMarkdown } from './message-input/message-content'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it, shown as the
 * message it will become: the same bubble in the same place, at its height,
 * its attachments among the media. Its delivery is part of the wait for the
 * answer, which the tail row tells, and a lost connection the Connecting row
 * or the connection banner; the bubble itself never says either, so it does
 * not change height when the message arrives (`turns.md` § Following the
 * agent's work). A failed delivery is told by the row under it, with Retry
 * (`DeliveryFailureRow`), which takes the Requesting row's place. A steer or a queued message shows as the one it will be from
 * the send, with its line and its controls (`turns.md` § Steer or queue),
 * which the host keeps as the user's wish until the session holds it; one
 * that was not delivered is a message to try again, without them.
 */
const props = defineProps<PendingSubmissionState & {
  /** What it will be when it waits for the agent, which its line says (`waitingAs`). */
  pending?: 'steer' | 'queued'
}>()
const emit = defineEmits<{ retry: []; recall: []; sendNow: [] }>()
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
      @recall="emit('recall')"
      @send-now="emit('sendNow')"
      @retry="emit('retry')"
    />
  </div>
</template>

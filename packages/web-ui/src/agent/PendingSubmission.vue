<script setup lang="ts">
import { computed } from 'vue'
import UserBlock from './blocks/UserBlock.vue'
import { sentMarkdown } from './message-input/message-content'
import type { PendingSubmissionState } from './types'

/**
 * The message the user sent before the server confirmed it, shown as the
 * message it will become: the same bubble in the same place, at its height, its
 * attachments among the media.
 *
 * The bubble never says whether the delivery or the connection is the problem,
 * so it does not change height when the message arrives:
 *
 * - Its delivery is part of the wait for the answer, which the tail row tells.
 * - A lost connection is told by the Connecting row or the connection banner.
 *
 * A failed delivery is told by the row under the bubble (`DeliveryFailureRow`),
 * which takes the Requesting row's place. The message's own hover actions offer
 * Edit and Delete. The dock's Retry sends it again.
 *
 * A steer or a queued message shows as the one it will be from the send, with
 * its line and its controls. The host keeps them as the user's wish until the
 * session holds the message. A message that was not delivered is one to try
 * again, without them.
 */
const props = defineProps<PendingSubmissionState & {
  /**
   * What it will be when it waits for the agent, which its line says
   * (`waitingAs`).
   */
  pending?: 'steer' | 'queued'
}>()
const emit = defineEmits<{ edit: []; delete: []; recall: []; sendNow: [] }>()
/** The text as the message the server confirms holds it. */
const markdown = computed(() => sentMarkdown(props.text, props.attachments.length))
</script>

<template>
  <div aria-live="polite">
    <UserBlock
      :content="markdown ? [{ type: 'text', text: markdown }] : []"
      :attachments="attachments"
      :comments="comments"
      :editable="error !== null"
      :deletable="error !== null"
      :pending="pending"
      @recall="emit('recall')"
      @send-now="emit('sendNow')"
      @edit="emit('edit')"
      @delete="emit('delete')"
    />
  </div>
</template>

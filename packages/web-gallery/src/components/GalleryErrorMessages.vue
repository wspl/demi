<script setup lang="ts">
import AgentMessageVirtualBlock from '@demicodes/web-ui/agent/blocks/AgentMessageVirtualBlock.vue'
import PendingSubmission from '@demicodes/web-ui/agent/PendingSubmission.vue'
import type { PendingSubmissionState } from '@demicodes/web-ui/agent/types'
import { composerAttachment } from '@demicodes/web-ui/agent/message-input/attachments'
import type { DisplayedBlock as Block } from '@demicodes/web-ui/transport/protocol'
import { demoModel, errorTool } from '../fixtures/blocks'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Failures that live in the conversation, pinned. Every one is an ErrorNotice
 * in the transcript flow, rendered through the product's own block renderer so
 * the padding and rhythm are the product's. Retry sits on the record that
 * ended the conversation, and nowhere else.
 *
 * Nothing in or under the composer: a fork that failed, an edit the server
 * refused, a file that could not be read and an upload that failed are toasts,
 * and the failed file leaves the composer.
 */
function errorRecord(
  id: string,
  code: string,
  httpStatus: number,
  message: string,
): Block {
  return {
    type: 'error',
    id,
    createdAt: '2026-09-11T12:00:00Z',
    model: demoModel,
    message,
    code,
    diagnostics: { source: 'http', httpStatus, clientRequestId: 'req_01J8Y3Q6ZKX4' },
  }
}
const records: { block: Block; tail: boolean }[] = [
  {
    block: errorRecord(
      'rate_limit',
      'rate_limit',
      429,
      'Anthropic API request failed with HTTP 429: This request would exceed the rate limit of 50 requests per minute for your organization. Retry after 12 seconds.',
    ),
    tail: true,
  },
  {
    block: errorRecord('overloaded', 'overloaded', 529, 'Overloaded. The provider could not accept the request.'),
    tail: false,
  },
  {
    block: errorRecord('auth_expired', 'auth_expired', 401, 'The access token has expired. Sign in to the provider again.'),
    tail: false,
  },
  {
    block: errorRecord('auth_missing', 'auth_missing', 401, 'No credentials are configured for this provider.'),
    tail: false,
  },
  {
    block: errorRecord(
      'context_length_exceeded',
      'context_length_exceeded',
      400,
      'prompt is too long: 214,331 tokens > 200,000 maximum.',
    ),
    tail: false,
  },
  {
    block: errorRecord(
      'generation_failed',
      'generation_failed',
      502,
      'The upstream response ended before generation completed.',
    ),
    tail: false,
  },
]
const toolFailure = errorTool as Block
const pending: PendingSubmissionState = {
  id: 'failed-submission',
  text: 'Please keep this exact message and the attached plan.',
  attachments: [
    composerAttachment({ name: 'plan.pdf', phase: 'ready' }),
    composerAttachment({ name: 'reference.png', phase: 'ready' }),
  ],
  error: 'The server did not confirm the message. Retry checks whether it was accepted before sending it again.',
  sending: false,
}
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Turn failures · the record in the transcript"
      note="One sentence from the normalized code, the upstream message, the facts a support thread asks for, Copy. The record that ended the conversation carries Retry; the older ones are history. While a retry runs the record is hidden and the tail row shows the turn."
    >
      <GallerySpecimen
        v-for="{ block, tail } in records"
        :key="block.id"
        wide
        :variant="`${block.type === 'error' ? block.code : ''}${tail ? ' · tail, with Retry' : ''}`"
      >
        <div class="rounded-lg bg-surface py-3">
          <AgentMessageVirtualBlock
            :block="block"
            conversation-id="error-messages"
            :is-thinking-streaming="false"
            :thinking-ended-at="null"
            :retry="tail ? () => {} : undefined"
          />
        </div>
      </GallerySpecimen>
      <GallerySpecimen wide variant="Tool failed · the tool block is the record">
        <div class="rounded-lg bg-surface py-3">
          <AgentMessageVirtualBlock
            :block="toolFailure"
            conversation-id="error-messages"
            :is-thinking-streaming="false"
            :thinking-ended-at="null"
          />
        </div>
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="Undelivered message · the notice follows the message"
      note="The exact text and its attachments stay on screen; the failure follows them in flow, with Retry."
    >
      <GallerySpecimen wide variant="Send failed">
        <div class="rounded-lg bg-surface">
          <PendingSubmission v-bind="pending" />
        </div>
      </GallerySpecimen>
    </GallerySection>
  </div>
</template>

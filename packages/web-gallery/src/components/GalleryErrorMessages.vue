<script setup lang="ts">
import AgentMessageVirtualBlock from '@demicodes/web-ui/agent/blocks/AgentMessageVirtualBlock.vue'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { ATTACHMENT_MARK } from '@demicodes/web-ui/markdown/user-markdown'
import { composerAttachment } from '@demicodes/web-ui/agent/message-input/attachments'
import { joinMessageContent } from '@demicodes/web-ui/agent/message-input/message-content'
import type { Block, UserContentBlock } from '@demicodes/protocol'
import { useTurnFlow } from '../turn-flow'
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
// The exact message stays with the failure under it. Retry sends it again
// with the same id, and Requesting shows until its turn answers.
const undeliveredFlow = useTurnFlow({ id: 'gallery-undelivered-message' })
const undeliveredText = `Please keep this exact message, the plan ${ATTACHMENT_MARK} and the capture ${ATTACHMENT_MARK}.`
const undeliveredFiles = [
  { name: 'plan.pdf', mediaType: 'application/pdf' },
  { name: 'reference.png', mediaType: 'image/png' },
]

function failDelivery(): void {
  undeliveredFlow.undelivered(
    joinMessageContent(undeliveredText, undeliveredFiles.map(({ name, mediaType }) => [{
      type: 'attachment',
      name,
      path: `/home/demi/.demi/attachments/gallery/${name}`,
      mediaType,
      sizeBytes: 4096,
      sha256: '0'.repeat(64),
    } satisfies UserContentBlock])),
    {
      text: undeliveredText,
      attachments: undeliveredFiles.map(({ name }) => composerAttachment({ name, phase: 'ready' })),
    },
    'The server did not confirm the message. Retry checks whether it was accepted before sending it again.',
  )
}
failDelivery()
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Turn Failures · the Record in the Transcript"
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
            :is-thinking-streaming="false"
            :thinking-ended-at="null"
          />
        </div>
      </GallerySpecimen>
    </GallerySection>
    <GallerySection
      title="Undelivered Message · the Notice Follows the Message"
      note="The exact text and its attachments stay on screen; the failure follows them in flow, with Retry, which sends the message again and shows Requesting from then."
    >
      <GallerySpecimen wide variant="Send failed">
        <div class="flex h-[18rem] flex-col overflow-hidden rounded-lg bg-surface">
          <AgentMessageList
            class="min-h-0 flex-1"
            :conversation-id="undeliveredFlow.state.id"
            :blocks="undeliveredFlow.state.blocks"
            :pending-steers="[]"
            :queue="[]"
            :phase="undeliveredFlow.state.phase"
            :load="undeliveredFlow.state.load"
            :pending-submission="undeliveredFlow.pendingSubmission.value"
            :bottom-offset="0"
            :persisted-scroll-state="undefined"
            read-only
            @retry-submission="undeliveredFlow.retrySubmission"
          />
        </div>
        <Button size="sm" class="mt-2" @click="failDelivery">Fail Again</Button>
      </GallerySpecimen>
    </GallerySection>
  </div>
</template>

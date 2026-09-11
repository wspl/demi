<script setup lang="ts">
import { ref } from 'vue'
import ErrorBlock from '@demicodes/web-ui/agent/blocks/ErrorBlock.vue'
import AgentMessageVirtualBlock from '@demicodes/web-ui/agent/blocks/AgentMessageVirtualBlock.vue'
import AssistantMessageFooter from '@demicodes/web-ui/agent/blocks/AssistantMessageFooter.vue'
import PendingSubmission from '@demicodes/web-ui/agent/PendingSubmission.vue'
import type { MessageForkState } from '@demicodes/web-ui/agent/message-fork'
import type { MessageEditState } from '@demicodes/web-ui/agent/message-editing'
import type { PendingSubmissionState } from '@demicodes/web-ui/agent/types'
import type { Block } from '@demicodes/core'
import { errorTool } from '../fixtures/blocks'
import GalleryComposer from './GalleryComposer.vue'
import GallerySection from './GallerySection.vue'
import GallerySpecimen from './GallerySpecimen.vue'

/**
 * Failures that live next to a message, pinned.
 * - A turn that failed is a transcript record: ErrorBlock for the provider,
 *   the tool block itself for a tool.
 * - An action on a message that failed (send, fork, edit, upload) keeps the
 *   message and puts InlineError directly under it, with Retry where a retry exists.
 */
const failures = [
  {
    code: 'rate_limit',
    httpStatus: 429,
    message: 'Too many requests. Try again after 30 seconds.',
  },
  {
    code: 'overloaded',
    httpStatus: 503,
    message: 'The model provider is temporarily overloaded.',
  },
  {
    code: 'auth_expired',
    httpStatus: 401,
    message: 'Your provider session has expired. Sign in again.',
  },
  {
    code: 'auth_missing',
    httpStatus: 401,
    message: 'No credentials are configured for this model provider.',
  },
  {
    code: 'context_length_exceeded',
    httpStatus: 400,
    message: 'This conversation exceeds the model context limit.',
  },
  {
    code: 'generation_failed',
    httpStatus: 502,
    message:
      'The upstream response ended before generation completed. Request preview-7842 returned HTTP 502.\nThe draft and preceding conversation remain available.',
  },
]
// The first record starts open so the detail row and Copy are visible without a click.
const openCode = ref<string | null>('rate_limit')
const toolFailure = errorTool as Block
const pending: PendingSubmissionState = {
  id: 'failed-submission',
  text: 'Please keep this exact message and the attached plan.',
  fileNames: ['plan.pdf'],
  error:
    'Could not confirm delivery. Try again to check whether this message was accepted.',
  sending: false,
}
const fork: MessageForkState = {
  phase: 'failed',
  request: { id: 'failed-fork', blockId: 'answer' },
  error:
    'Could not create the conversation. Your source conversation is unchanged.',
}
const editing: MessageEditState = {
  phase: 'editing',
  error: 'The conversation changed. Review this edit and try again.',
  request: {
    operationId: 'failed-edit',
    targetBlockId: 'editable-message',
    version: { epoch: 'preview', revision: 2 },
    content: [
      {
        type: 'text',
        text: 'Keep the edited message after the request fails.',
      },
    ],
  },
}
const uncertain: MessageEditState = {
  phase: 'uncertain',
  error:
    'Could not confirm whether the edit was accepted. Retry to check its result.',
  request: {
    operationId: 'uncertain-edit',
    targetBlockId: 'uncertain-message',
    version: { epoch: 'preview', revision: 3 },
    content: [
      {
        type: 'text',
        text: 'This submitted edit is waiting for confirmation.',
      },
    ],
  },
}
</script>

<template>
  <div class="space-y-8">
    <GallerySection
      title="Turn failures · transcript records"
      note="A failed turn is a record in the transcript: one line in the chrome from the normalized code, the upstream message and the diagnostics in the body, Copy for a support thread."
    >
      <div class="grid gap-4 lg:grid-cols-2">
        <GallerySpecimen
          v-for="failure in failures"
          :key="failure.code"
          wide
          :variant="failure.code"
        >
          <div class="rounded-lg bg-surface p-3">
            <ErrorBlock
              :open="openCode === failure.code"
              :code="failure.code"
              :message="failure.message"
              :diagnostics="{
                source: 'http',
                httpStatus: failure.httpStatus,
                clientRequestId: 'preview-7842',
              }"
              @update:open="(open) => (openCode = open ? failure.code : null)"
            />
          </div>
        </GallerySpecimen>
      </div>
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
      title="Message actions · InlineError under the message"
      note="Send, fork, edit and upload keep the message on screen and put the failure directly under it. Retry sits in the same line when a retry exists."
    >
      <div class="grid gap-6 xl:grid-cols-2">
        <GallerySpecimen wide variant="Send failed · exact message retained">
          <div class="rounded-lg bg-surface">
            <PendingSubmission v-bind="pending" />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Fork failed · under the message footer">
          <div class="rounded-lg bg-surface p-4">
            <p class="text-conversation text-fg-body">
              The game is ready. This answer is the fork point.
            </p>
            <AssistantMessageFooter
              content="The game is ready."
              created-at="2026-09-11T12:00:00Z"
              :fork="async () => {}"
              :fork-state="fork"
            />
          </div>
        </GallerySpecimen>
        <GallerySpecimen wide variant="Edit rejected · draft remains editable">
          <GalleryComposer
            :message-edit="editing"
            placeholder="Edit message…"
            conversation-id="failed-edit"
          />
        </GallerySpecimen>
        <GallerySpecimen wide variant="Edit acceptance unknown · Retry edit">
          <GalleryComposer
            :message-edit="uncertain"
            placeholder="Edit message…"
            conversation-id="uncertain-edit"
          />
        </GallerySpecimen>
        <GallerySpecimen
          wide
          variant="Uploads failed · Retry on each tile, reasons under the input"
        >
          <GalleryComposer
            placeholder="Message with failed attachments…"
            conversation-id="failed-uploads"
            draft="The files must stay here while I retry."
            :attachments="[
              {
                name: 'reference.png',
                phase: 'failed',
                error: 'Connection lost during upload.',
              },
              {
                name: 'plan.pdf',
                phase: 'failed',
                error: 'The workspace is unavailable.',
              },
            ]"
          />
        </GallerySpecimen>
      </div>
    </GallerySection>
  </div>
</template>

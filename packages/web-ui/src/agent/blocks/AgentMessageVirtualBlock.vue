<script setup lang="ts">
import { computed, useAttrs } from 'vue'
import { isEditableUserMessage } from '@demicodes/agent/client'
import { CHROME_ENTER_MS } from '@demicodes/web-ui/ui/chrome-enter'
import type { MessageListBlock } from '../pending-steers'
import UserBlock from './UserBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import AssistantTextBlock from './AssistantTextBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import ErrorBlock from './ErrorBlock.vue'
import AbortedBlock from './AbortedBlock.vue'
import CompactionBlock from './CompactionBlock.vue'
import QueueDivider from './QueueDivider.vue'
import PendingSubmission from '../PendingSubmission.vue'
import type { MessageForkState } from '../message-fork'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  block: MessageListBlock
  conversationId: string
  isThinkingStreaming: boolean
  isTextStreaming?: boolean
  thinkingEndedAt?: string | null
  editable?: boolean
  fork?: () => Promise<void>
  forkState?: MessageForkState
  /** Retry on the error record that ended the conversation. */
  retry?: () => void
  /** The block just arrived in a live transcript: a chrome row slides in from the left as it fades in. */
  entering?: boolean
}>()

const emit = defineEmits<{
  deletePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  deleteQueued: [id: string]
  sendQueued: [id: string]
  editUser: [blockId: string]
  retrySubmission: []
}>()

const attrs = useAttrs()
/** Rows built on FunctionalBlock: the 28px chrome face. Text streams in on its own. */
const entersAsChrome = computed(() =>
  props.entering
  && (props.block.type === 'thinking' || props.block.type === 'tool_call' || props.block.type === 'abort'),
)
</script>

<template>
  <PendingSubmission
    v-if="block.type === 'pending_submission'"
    v-bind="{ ...attrs, ...block.submission }"
    @retry="emit('retrySubmission')"
  />
  <UserBlock
    v-else-if="block.type === 'user'"
    v-bind="attrs"
    :content="block.content"
    :editable="editable && isEditableUserMessage(block)"
    @edit="emit('editUser', block.id)"
  />
  <UserBlock
    v-else-if="block.type === 'steer'"
    v-bind="attrs"
    :content="block.content"
    variant="steer"
    :editable="false"
  />
  <UserBlock
    v-else-if="block.type === 'pending_steer'"
    v-bind="attrs"
    :content="block.content"
    variant="steer"
    pending
    deletable
    interruptible
    @delete="emit('deletePendingSteer', block.pendingSteerId)"
    @interrupt="emit('interruptPendingSteer', block.pendingSteerId)"
  />
  <UserBlock
    v-else-if="block.type === 'queued_message'"
    v-bind="attrs"
    :content="block.content"
    variant="steer"
    pending
    deletable
    sendable
    @delete="emit('deleteQueued', block.queueId)"
    @send-now="emit('sendQueued', block.queueId)"
  />
  <QueueDivider
    v-else-if="block.type === 'queue_divider'"
    v-bind="attrs"
    :count="block.count"
  />
  <div
    v-else
    v-bind="attrs"
    :class="entersAsChrome ? 'chrome-enter' : ''"
    :style="entersAsChrome ? { '--chrome-enter-ms': `${CHROME_ENTER_MS}ms` } : undefined"
  >
    <ThinkingBlock
      v-if="block.type === 'thinking'"
      :thinking="block.text"
      :is-streaming="isThinkingStreaming"
      :created-at="block.createdAt"
      :ended-at="thinkingEndedAt"
    />
    <AssistantTextBlock
      v-else-if="block.type === 'text'"
      :content="block.text"
      :is-streaming="isTextStreaming"
      :created-at="block.createdAt"
      :fork="block.forkable ? fork : undefined"
      :fork-state="forkState"
    />
    <div
      v-else-if="block.type === 'tool_call'"
      class="overflow-hidden px-[var(--agent-pad-x,2rem)]"
    >
      <ToolCallBlock
        :block="block"
        :conversation-id="props.conversationId"
        :is-streaming="block.status === 'executing'"
      />
    </div>
    <div
      v-else-if="block.type === 'error'"
      class="px-[var(--agent-pad-x,2rem)] py-1.5"
    >
      <ErrorBlock
        :message="block.message"
        :code="block.code"
        :diagnostics="block.diagnostics"
        :retry="retry"
      />
    </div>
    <div
      v-else-if="block.type === 'abort'"
      class="px-[var(--agent-pad-x,2rem)]"
    >
      <AbortedBlock />
    </div>
    <CompactionBlock
      v-else-if="block.type === 'compaction_boundary'"
      :summary="block.summary"
      :summary-tokens="block.summaryTokens"
      :is-compacting="false"
      :created-at="block.createdAt"
    />
  </div>
</template>

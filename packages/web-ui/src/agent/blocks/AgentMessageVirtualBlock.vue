<script setup lang="ts">
import { useAttrs } from 'vue'
import type { MessageListBlock } from '../pending-steers'
import UserBlock from './UserBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import AssistantTextBlock from './AssistantTextBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import ErrorBlock from './ErrorBlock.vue'
import AbortedBlock from './AbortedBlock.vue'
import CompactionBlock from './CompactionBlock.vue'
import QueueDivider from './QueueDivider.vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  block: MessageListBlock
  conversationId: string
  isThinkingStreaming: boolean
  isTextStreaming?: boolean
  thinkingEndedAt?: string | null
}>()

const emit = defineEmits<{
  deletePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  deleteQueued: [id: string]
  sendQueued: [id: string]
}>()

const attrs = useAttrs()
</script>

<template>
  <UserBlock
    v-if="block.type === 'user'"
    v-bind="attrs"
    :content="block.content"
  />
  <UserBlock
    v-else-if="block.type === 'steer'"
    v-bind="attrs"
    :content="block.content"
    variant="steer"
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
  <div v-else v-bind="attrs">
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
    />
    <div v-else-if="block.type === 'tool_call'" class="overflow-hidden px-[var(--agent-pad-x,2rem)]">
      <ToolCallBlock :block="block" :conversation-id="props.conversationId" :is-streaming="block.status === 'executing'" />
    </div>
    <div v-else-if="block.type === 'error'" class="px-[var(--agent-pad-x,2rem)]">
      <ErrorBlock
        :message="block.message"
        :code="block.code"
        :diagnostics="block.diagnostics"
      />
    </div>
    <div v-else-if="block.type === 'abort'" class="px-[var(--agent-pad-x,2rem)]">
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

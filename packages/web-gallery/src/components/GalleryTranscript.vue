<script setup lang="ts">
import AgentMessageVirtualBlock from '@demicodes/web-ui/agent/blocks/AgentMessageVirtualBlock.vue'
import type { MessageListBlock } from '@demicodes/web-ui/agent/pending-steers'
import ActivitySlot from './ActivitySlot.vue'
import type { ActivitySlotState } from '../turn-flow'

const props = defineProps<{
  blocks: readonly MessageListBlock[]
  streamingThinkingId?: string | null
  streamingTextId?: string | null
  endedAtById?: Readonly<Record<string, string>>
  activity?: ActivitySlotState | null
}>()

const emit = defineEmits<{
  deletePendingSteer: [id: string]
  interruptPendingSteer: [id: string]
  deleteQueued: [id: string]
  sendQueued: [id: string]
}>()

function isThinkingStreaming(blocks: readonly MessageListBlock[], index: number): boolean {
  const block = blocks[index]
  return block?.type === 'thinking' && block.id === props.streamingThinkingId
}

function isTextStreaming(blocks: readonly MessageListBlock[], index: number): boolean {
  const block = blocks[index]
  return block?.type === 'text' && block.id === props.streamingTextId
}

function thinkingEndedAt(blocks: readonly MessageListBlock[], index: number): string | null {
  const block = blocks[index]
  if (block?.type !== 'thinking' || block.id === props.streamingThinkingId) return null
  if (props.endedAtById?.[block.id]) return props.endedAtById[block.id]
  const next = blocks[index + 1]
  return next && 'createdAt' in next ? next.createdAt : null
}
</script>

<template>
  <AgentMessageVirtualBlock
    v-for="(block, index) in blocks"
    :key="block.id"
    :block="block"
    conversation-id="demo"
    :is-thinking-streaming="isThinkingStreaming(blocks, index)"
    :is-text-streaming="isTextStreaming(blocks, index)"
    :thinking-ended-at="thinkingEndedAt(blocks, index)"
    @delete-pending-steer="emit('deletePendingSteer', $event)"
    @interrupt-pending-steer="emit('interruptPendingSteer', $event)"
    @delete-queued="emit('deleteQueued', $event)"
    @send-queued="emit('sendQueued', $event)"
  />
  <ActivitySlot
    v-if="activity"
    :kind="activity.kind"
    :label="activity.label"
    :incoming="activity.incoming"
  />
</template>

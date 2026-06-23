<script setup lang="ts">
import { useAttrs } from 'vue'
import type { Block } from '@demi/core'
import UserBlock from './UserBlock.vue'
import ThinkingBlock from './ThinkingBlock.vue'
import AssistantTextBlock from './AssistantTextBlock.vue'
import ToolCallBlock from './ToolCallBlock.vue'
import ErrorBlock from './ErrorBlock.vue'
import AbortedBlock from './AbortedBlock.vue'
import ResponseStatsBlock from './ResponseStatsBlock.vue'
import CompactionBlock from './CompactionBlock.vue'

defineOptions({ inheritAttrs: false })

const props = defineProps<{
  block: Block
  conversationId: string
  isThinkingStreaming: boolean
  thinkingEndedAt?: string | null
}>()

const emit = defineEmits<{
  continue: []
  retry: []
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
    />
    <div v-else-if="block.type === 'tool_call'" class="overflow-hidden px-8">
      <ToolCallBlock :block="block" :conversation-id="props.conversationId" :is-streaming="block.status === 'executing'" />
    </div>
    <ResponseStatsBlock
      v-else-if="block.type === 'response'"
      :usage="block.usage"
      :context-window="block.model.model.contextWindow"
    />
    <div v-else-if="block.type === 'error'" class="px-8">
      <ErrorBlock
        :message="block.message"
        :code="block.code"
        @continue="emit('continue')"
        @retry="emit('retry')"
      />
    </div>
    <div v-else-if="block.type === 'abort'" class="px-8">
      <AbortedBlock
        @continue="emit('continue')"
        @retry="emit('retry')"
      />
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

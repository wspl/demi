<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import AssistantMessageFooter from './AssistantMessageFooter.vue'
import type { MessageForkState } from '../message-fork'

withDefaults(defineProps<{
  content: string
  createdAt: string
  fork?: () => Promise<void>
  forkState?: MessageForkState
  isStreaming?: boolean
  showFooter?: boolean
}>(), {
  isStreaming: false,
})
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)] py-1.5">
    <StreamedMarkdown
      :content="content"
      :streaming="isStreaming"
      class="text-conversation text-fg-body"
    />
    <AssistantMessageFooter
      v-if="showFooter && !isStreaming"
      :content="content"
      :created-at="createdAt"
      :fork="fork"
      :fork-state="forkState"
    />
  </div>
</template>

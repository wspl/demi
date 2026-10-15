<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import AssistantMessageFooter from './AssistantMessageFooter.vue'
import type { MessageForkState } from '../message-fork'
import type { AnswerShare } from '../sharing'
import type { VersionChoice } from '../versions'
import { useTextArrivals } from '../whole-blocks'

withDefaults(defineProps<{
  content: string
  createdAt: string
  fork?: () => Promise<void>
  forkState?: MessageForkState
  share?: AnswerShare
  shareWait?: boolean
  hideFork?: boolean
  /** Regenerate in the footer; absent where it is not offered. */
  regenerate?: () => void
  /** The answer's versions in the footer; absent with none. */
  versions?: VersionChoice
  isStreaming?: boolean
  showFooter?: boolean
}>(), {
  isStreaming: false,
})

const arrivals = useTextArrivals()
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)] py-1.5">
    <StreamedMarkdown
      :content="content"
      :streaming="isStreaming"
      :arrivals="arrivals()"
      class="text-conversation text-fg-body"
    />
    <AssistantMessageFooter
      v-if="showFooter && !isStreaming"
      :content="content"
      :created-at="createdAt"
      :fork="fork"
      :fork-state="forkState"
      :share="share"
      :share-wait="shareWait"
      :hide-fork="hideFork"
      :regenerate="regenerate"
      :versions="versions"
    />
  </div>
</template>

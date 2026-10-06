<script setup lang="ts">
import AssistantTextBlock from '@demicodes/web-ui/agent/blocks/AssistantTextBlock.vue'
import { provideMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'
import { attachmentMarkdown } from '../fixtures/attachments'

/**
 * A reply that shows the attachments the agent uploaded: an image a click
 * shows large, a video that plays in place, links that open them, a file that
 * downloads, and a number the conversation does not have.
 */
const props = defineProps<{
  /** The gallery workspace, with its attachments. */
  files: ConversationFiles
  cwd: string
}>()

provideMessageFiles(() => ({ ...props.files, cwd: props.cwd }))

const createdAt = new Date(Date.now() - 60_000).toISOString()
</script>

<template>
  <div class="gallery-frame bg-surface py-3">
    <AssistantTextBlock :content="attachmentMarkdown" :created-at="createdAt" />
  </div>
</template>

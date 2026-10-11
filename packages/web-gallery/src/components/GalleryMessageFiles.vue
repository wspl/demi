<script setup lang="ts">
import AssistantTextBlock from '@demicodes/web-ui/agent/blocks/AssistantTextBlock.vue'
import { provideMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import type { ConversationFiles } from '@demicodes/web-ui/markdown/types'

/**
 * A reply that names files: the attachments the agent uploaded, or the
 * workspace's files, which open in the frame's panel.
 */
const props = defineProps<{
  /** The gallery workspace, with its attachments. */
  files: ConversationFiles
  cwd: string
  content: string
}>()

provideMessageFiles(() => ({ ...props.files, cwd: props.cwd }))

const createdAt = new Date(Date.now() - 60_000).toISOString()
</script>

<template>
  <div class="gallery-frame bg-surface py-3">
    <AssistantTextBlock :content="content" :created-at="createdAt" />
  </div>
</template>

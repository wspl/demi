<script lang="ts">
import { ref } from 'vue'

// Markdown and SVG: whether an attachment shows rendered or as source holds
// across attachments for the page's lifetime, as the File view's choice does.
const mode = ref<'preview' | 'source'>('preview')
</script>

<script setup lang="ts">
import { AttachmentView } from '@demicodes/plugin-sdk'
import type { AttachmentData } from './attachment'

/** An attachment's tab: its stored copy, previewed as the File view previews a file. */
defineProps<{
  conversation: string
  tabId: string
  data: AttachmentData
  shown: boolean
}>()
</script>

<template>
  <AttachmentView v-model:mode="mode" :name="data.name" :size="data.size" :url="data.url" />
</template>

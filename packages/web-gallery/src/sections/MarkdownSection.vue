<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { previewMarkdown } from '../fixtures/previews'
</script>

<template>
  <!-- One message at full width, rendered by the transcript's own component. -->
  <div class="min-h-0 flex-1 overflow-y-auto bg-surface-editor p-4 text-conversation text-fg-body">
    <StreamedMarkdown :content="previewMarkdown" />
  </div>
</template>

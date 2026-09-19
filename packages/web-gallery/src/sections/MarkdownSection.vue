<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { overlongMarkdown, previewMarkdown } from '../fixtures/previews'
</script>

<template>
  <div class="min-h-0 flex-1 overflow-y-auto bg-surface-editor p-4 text-conversation text-fg-body">
    <!-- One message at full width, rendered by the transcript's own component. -->
    <StreamedMarkdown :content="previewMarkdown" />
    <!-- A reply in a column as narrow as a squeezed transcript, which scrolls as the transcript
         does: the address and the path break, and the table and the equation scroll on their own,
         so the column never scrolls sideways. -->
    <div class="mt-8 max-h-[28rem] w-80 max-w-full overflow-y-auto rounded-md p-3 ring-1 ring-line">
      <StreamedMarkdown :content="overlongMarkdown" />
    </div>
  </div>
</template>

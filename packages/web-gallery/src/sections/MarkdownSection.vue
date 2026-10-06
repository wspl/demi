<script setup lang="ts">
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { overlongMarkdown, previewMarkdown } from '../fixtures/previews'
</script>

<template>
  <ScrollArea class="flex-1 bg-surface-editor text-conversation text-fg-body" viewport-class="p-4">
    <!-- One message at full width, rendered by the transcript's own component. -->
    <StreamedMarkdown :content="previewMarkdown" />
    <!-- A reply in a column as narrow as a squeezed transcript, which scrolls as the transcript
         does: the address and the path break, and the long line of code, the table and the
         equation scroll on their own, so the column never scrolls sideways. Their scrollbar
         takes no room: the long code block keeps the short one's padding, and its thumb shows
         inside that padding while the pointer is on it. -->
    <ScrollArea class="mt-8 max-h-[28rem] w-80 max-w-full rounded-md ring-1 ring-line" viewport-class="p-3">
      <StreamedMarkdown :content="overlongMarkdown" />
    </ScrollArea>
  </ScrollArea>
</template>

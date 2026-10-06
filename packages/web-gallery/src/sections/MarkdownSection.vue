<script setup lang="ts">
import ScrollArea from '@demicodes/web-ui/ui/ScrollArea.vue'
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { alignedMarkdown, dollarMarkdown, overlongMarkdown, previewMarkdown } from '../fixtures/previews'
</script>

<template>
  <ScrollArea class="flex-1 bg-surface-editor text-conversation text-fg-body" viewport-class="p-4">
    <!-- One message at full width, rendered by the transcript's own component. -->
    <StreamedMarkdown :content="previewMarkdown" />
    <!-- Prices and math in one reply: a dollar sign before an amount stays text, and math
         renders on its own and flush against Chinese text (`file-previews.md` § Markdown). -->
    <StreamedMarkdown class="mt-8" :content="dollarMarkdown" />
    <!-- Header cells start where their column's text starts; a column the delimiter row
         aligns (`--:` to the end, `:-:` to the center) aligns its header with its cells. -->
    <StreamedMarkdown class="mt-8" :content="alignedMarkdown" />
    <!-- A reply in a column as narrow as a squeezed transcript, which scrolls as the transcript
         does: the address and the path break, and the long line of code, the table and the
         equation scroll on their own, so the column never scrolls sideways. Each fades out at
         the side that hides more of it, the end at rest and the start once scrolled, so a reader
         sees there is more before any bar shows; the short code block, which fits, does not fade.
         Their scrollbar takes no room: the long code block keeps the short one's padding, and
         its thumb shows inside that padding while the pointer is on it. The wide table's
         Address header starts at its column's start, where the address does. -->
    <ScrollArea class="mt-8 max-h-[28rem] w-80 max-w-full rounded-md ring-1 ring-line" viewport-class="p-3">
      <StreamedMarkdown :content="overlongMarkdown" />
    </ScrollArea>
  </ScrollArea>
</template>

<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'

/**
 * A thinking's text, as an opened Thinking row shows it: at the transcript's
 * size, muted under the agent's prose (`product.md` § Following the agent's
 * work, One size for the reading).
 */
defineProps<{ content: string, streaming: boolean }>()
</script>

<template>
  <StreamedMarkdown
    :content="content"
    :streaming="streaming"
    class="px-3 py-1 text-conversation text-fg-subtle [--markdown-block-gap:--spacing(1)]"
  />
</template>

<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'

withDefaults(defineProps<{
  content: string
  isStreaming?: boolean
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
  </div>
</template>

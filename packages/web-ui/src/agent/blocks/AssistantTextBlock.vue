<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import AssistantMessageFooter from './AssistantMessageFooter.vue'

withDefaults(defineProps<{
  content: string
  createdAt: string
  forkable?: boolean
  isStreaming?: boolean
}>(), {
  isStreaming: false,
})

const emit = defineEmits<{ fork: [] }>()
</script>

<template>
  <div class="px-[var(--agent-pad-x,2rem)] py-1.5">
    <StreamedMarkdown
      :content="content"
      :streaming="isStreaming"
      class="text-conversation text-fg-body"
    />
    <AssistantMessageFooter
      v-if="!isStreaming"
      :content="content"
      :created-at="createdAt"
      :forkable="forkable"
      @fork="emit('fork')"
    />
  </div>
</template>

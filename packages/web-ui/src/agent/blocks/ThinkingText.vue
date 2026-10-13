<script setup lang="ts">
import StreamedMarkdown from '@demicodes/web-ui/ui/StreamedMarkdown.vue'
import { useTextArrivals } from '../whole-blocks'

/**
 * A thinking's text, as an opened Thinking row shows it: secondary reading,
 * one step under the transcript's size and looser (`--agent-secondary`),
 * muted under the agent's prose, with half an em between paragraphs. The one
 * place that styles it, for a Thinking row, a run's opened steps and a
 * subagent's transcript.
 */
defineProps<{ content: string, streaming: boolean }>()
const arrivals = useTextArrivals()
</script>

<template>
  <StreamedMarkdown
    :content="content"
    :streaming="streaming"
    :arrivals="arrivals()"
    class="px-3 py-1 text-conversation-secondary text-fg-subtle [--markdown-block-gap:0.5em]"
  />
</template>

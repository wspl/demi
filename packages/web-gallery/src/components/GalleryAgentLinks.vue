<script setup lang="ts">
import { reactive } from 'vue'
import AssistantTextBlock from '@demicodes/web-ui/agent/blocks/AssistantTextBlock.vue'
import GalleryReceipts from './GalleryReceipts.vue'
import { agentLinksMarkdown, gallerySubagents } from '../fixtures/subagents'

/** The reply to show, the four agents' by default; `narrow` sets it in a column as narrow as a phone's. */
const props = withDefaults(defineProps<{ markdown?: string; narrow?: boolean }>(), {
  markdown: agentLinksMarkdown,
  narrow: false,
})

/**
 * A reply that links the conversation's agents: running, finished, failed,
 * stopped, the main agent, and a number the conversation does not have. A
 * click opens the Agents window over the reply on that agent's tab, and
 * leaves it as it is when it shows that tab; Stop in the window stops the
 * agent here, and its link's mark follows.
 */
const agents = reactive(gallerySubagents())
const createdAt = new Date(Date.now() - 60_000).toISOString()

function stop(id: string): void {
  const agent = agents.find((candidate) => candidate.id === id)
  if (agent?.phase === 'running') {
    agent.phase = 'aborted'
    agent.endedAt = new Date().toISOString()
  }
}
</script>

<template>
  <GalleryReceipts :agents="agents" @abort="agents.forEach((agent) => stop(agent.id))" @abort-agent="stop">
    <!-- The reply ends the transcript below the window, as a session's latest reply does, so a link stays in reach while the window is open. -->
    <div class="gallery-frame flex h-[36rem] flex-col justify-end bg-surface py-3">
      <AssistantTextBlock :class="props.narrow ? 'max-w-[22rem]' : ''" :content="props.markdown" :created-at="createdAt" />
    </div>
  </GalleryReceipts>
</template>

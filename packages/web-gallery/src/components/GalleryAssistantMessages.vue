<script setup lang="ts">
import { ref } from 'vue'
import AssistantTextBlock from '@demicodes/web-ui/agent/blocks/AssistantTextBlock.vue'
import SessionNoticeBar from '@demicodes/web-ui/agent/SessionNoticeBar.vue'

const createdAt = new Date(Date.now() - 35_000).toISOString()
const forkRequested = ref(false)
const content = 'The message footer is ready. You can **copy this reply**, fork from this point, or check when it was sent.\n\nThe relative timestamp updates automatically while this page stays open.'
</script>

<template>
  <div class="gallery-frame bg-surface py-3">
    <AssistantTextBlock
      :content="content"
      :created-at="createdAt"
      forkable
      @fork="forkRequested = true"
    />
    <div
      v-if="forkRequested"
      class="mt-2 px-[var(--agent-pad-x,2rem)]"
    >
      <SessionNoticeBar label="Fork requested from this reply · preview only" />
    </div>
  </div>
</template>

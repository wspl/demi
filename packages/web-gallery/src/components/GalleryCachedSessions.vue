<script setup lang="ts">
import { computed, onBeforeUnmount, reactive, ref } from 'vue'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import { ConversationCache } from '@demicodes/web-ui/agent/conversation-cache'
import type { SessionLoad } from '@demicodes/web-ui/agent/session-status'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { transcriptDemoBlocks } from '../fixtures/blocks'

const cache = new ConversationCache()
const sessions = reactive(['A', 'B'].map((id) => ({
  id,
  blocks: transcriptDemoBlocks(),
  load: 'loading' as SessionLoad,
  reads: 0,
})))
const selected = ref('A')
const current = computed(() => sessions.find((session) => session.id === selected.value)!)
async function select(id: string): Promise<void> {
  selected.value = id
  const session = current.value
  try {
    await cache.open(id, async () => {
      session.reads += 1
      session.load = 'ready'
    })
  } catch (error) {
    // Unmounting cancels a pending fixture load; the view no longer needs it.
    if (!(error instanceof DOMException && error.name === 'AbortError')) {
      throw error
    }
  }
}
void select('A')
onBeforeUnmount(() => cache.clear())
</script>
<template>
  <div class="flex items-center gap-2 pb-3">
    <Button v-for="session in sessions" :key="session.id" size="sm" @click="select(session.id)">
      {{ session.id }} · {{ session.reads }} load
    </Button>
  </div>
  <div class="gallery-frame h-[16rem] bg-surface">
    <AgentMessageList
      :conversation-id="current.id"
      :blocks="current.blocks"
      :pending-steers="[]"
      :queue="[]"
      phase="idle"
      :load="current.load"
      :bottom-offset="0"
      :persisted-scroll-state="undefined"
    />
  </div>
</template>

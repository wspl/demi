<script setup lang="ts">
import { nextTick, ref } from 'vue'
import type { Block } from '@demicodes/core'
import AgentMessageList from '@demicodes/web-ui/agent/AgentMessageList.vue'
import type { SessionLoad } from '@demicodes/web-ui/agent/session-status'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { transcriptDemoBlocks } from '../fixtures/blocks'

const history = transcriptDemoBlocks().filter((block) => block.type === 'tool_call').slice(-2)
const blocks = ref<Block[]>(history)
const load = ref<SessionLoad>('ready')
let nextId = 1

async function restore(): Promise<void> {
  load.value = 'loading'
  blocks.value = []
  await nextTick()
  blocks.value = [...history]
  load.value = 'ready'
}

function append(): void {
  blocks.value.push({ ...history[0]!, id: `live-entrance-${nextId++}` })
}
</script>

<template>
  <div class="flex gap-2 pb-3">
    <Button size="sm" @click="restore">Restore history</Button>
    <Button size="sm" @click="append">Append live block</Button>
  </div>
  <div class="gallery-frame h-[16rem] bg-surface">
    <AgentMessageList
      conversation-id="entrance-demo"
      :blocks="blocks"
      :load="load"
      :pending-steers="[]"
      :queue="[]"
      phase="idle"
      :bottom-offset="0"
      :persisted-scroll-state="undefined"
      read-only
    />
  </div>
</template>

<script setup lang="ts">
import { onBeforeUnmount, reactive, ref, shallowRef } from 'vue'
import { deferred, type Deferred } from '@demicodes/utils'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import type { ChatSessionState } from '@demicodes/web-ui/agent/types'
import type { MessageForkRequest } from '@demicodes/web-ui/agent/message-fork'
import Button from '@demicodes/web-ui/ui/Button.vue'
import { demoModel } from '../fixtures/blocks'

const createdAt = new Date(Date.now() - 35_000).toISOString()
const source = reactive<ChatSessionState>({
  id: 'fork-source', title: 'Planning', phase: 'running', status: 'active',
  load: 'ready', lastError: null, pendingAction: null, archived: false, queue: [], pendingSteers: [],
  scroll: null, subagents: [], terminals: [],
  blocks: [
    { type: 'user', id: 'user-1', turnId: 'turn-1', model: demoModel, createdAt,
      content: [{ type: 'text', text: 'Outline the next step.' }], preamble: null },
    { type: 'text', id: 'answer-1', model: demoModel, createdAt, forkable: true,
      text: 'The outline is ready. You can **Fork from this message** while the next answer is still being written.' },
    { type: 'user', id: 'user-2', turnId: 'turn-2', model: demoModel, createdAt,
      content: [{ type: 'text', text: 'Now expand the outline.' }], preamble: null },
    { type: 'text', id: 'answer-2', model: demoModel, createdAt,
      text: 'I’m expanding the first step…' },
  ],
})
const current = ref<ChatSessionState>(source)
const outcome = ref<'success' | 'hold' | 'failure'>('success')
const completion = shallowRef<Deferred<void> | null>(null)
const failed = new Set<string>()

async function fork(request: MessageForkRequest): Promise<void> {
  const from = current.value
  if (outcome.value === 'hold') {
    completion.value = deferred<void>()
    try {
      await completion.value.promise
    } finally {
      completion.value = null
    }
  }
  if (outcome.value === 'failure' && !failed.has(request.id)) {
    failed.add(request.id)
    throw new Error('Could not confirm the new conversation. Try again.')
  }
  const cutoff = from.blocks.findIndex((block) => block.id === request.blockId)
  current.value = {
    ...from, id: request.id, title: from.title + ' (Fork)', phase: 'idle', status: 'idle',
    blocks: from.blocks.slice(0, cutoff + 1), scroll: null,
  }
}

onBeforeUnmount(() => completion.value?.reject(new Error('Preview closed')))
</script>

<template>
  <div class="space-y-3">
    <div class="flex flex-wrap items-center gap-2 text-chrome text-fg-muted">
      <label>Creation result
        <select v-model="outcome" class="ml-2 rounded bg-surface-raised px-2 py-1" :disabled="!!completion">
          <option value="success">Success</option>
          <option value="hold">Pending</option>
          <option value="failure">Fail, then retry</option>
        </select>
      </label>
      <Button v-if="completion" @click="completion.resolve()">Finish creation</Button>
      <Button v-if="current.id !== source.id" @click="current = source">Back to source</Button>
    </div>
    <div class="gallery-frame flex h-[440px] bg-surface">
      <ChatSession :conversation="current" :has-provider="true" :fork="fork" />
    </div>
  </div>
</template>

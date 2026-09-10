<script setup lang="ts">
import { onBeforeUnmount, reactive, ref, shallowRef } from 'vue'
import { deferred, type Deferred } from '@demicodes/utils'
import ChatSession, { type ChatSessionState } from '@demicodes/web-ui/agent/ChatSession.vue'
import Button from '@demicodes/web-ui/ui/Button.vue'
import GalleryComposer from './GalleryComposer.vue'
import {
  EditRejectedError,
  submitMessageEdit,
  type MessageEditState,
} from '@demicodes/web-ui/agent/message-editing'
import type { Block } from '@demicodes/core'
import { demoModel } from '../fixtures/blocks'

const session = reactive<ChatSessionState>({
  id: 'editing-example', title: 'Edit and resend', blocks: [], queue: [],
  pendingSteers: [], phase: 'idle', load: 'ready', lastError: null,
  archived: false, status: 'idle', scroll: null, subagents: [], terminals: [],
})
const revision = ref(0)
const messageEdit = ref<MessageEditState | null>(null)
const outcome = ref<'accept' | 'hold' | 'reject' | 'disconnect'>('accept')
const accepted = new Set<string>()
const completion = shallowRef<Deferred<void> | null>(null)

function reset(): void {
  if (messageEdit.value?.phase === 'sending') {
    return
  }
  messageEdit.value = null
  session.blocks = ['Describe the change.', 'Add a test.', 'Review the result.'].flatMap<Block>((text, index) => [
    {
      type: 'user', id: `user-${revision.value}-${index}`, turnId: `turn-${index}`,
      createdAt: new Date().toISOString(), model: demoModel,
      content: [{ type: 'text', text }], preamble: null,
    },
    {
      type: 'text', id: `reply-${revision.value}-${index}`,
      createdAt: new Date().toISOString(), model: demoModel,
      text: ['The change makes message editing explicit.', 'The test checks the request received by the model.', 'The result is ready for review.'][index]!,
    },
  ])
  revision.value += 1
}
reset()

async function submit(): Promise<void> {
  await submitMessageEdit({
    get: () => messageEdit.value,
    set: (state) => { messageEdit.value = state },
    send: async (request) => {
      if (accepted.has(request.operationId)) {
        return
      }
      if (outcome.value === 'hold') {
        completion.value = deferred<void>()
        try {
          await completion.value.promise
        } finally {
          completion.value = null
        }
      }
      if (outcome.value === 'reject') {
        throw new EditRejectedError('The conversation changed. Exit editing and reopen the message.')
      }
      const index = session.blocks.findIndex((block) => block.id === request.targetBlockId)
      if (index < 0 || request.version.revision !== revision.value) {
        throw new EditRejectedError('The conversation changed. Reopen the message to edit it.')
      }
      session.blocks = [
        ...session.blocks.slice(0, index),
        {
          type: 'user', id: request.operationId, turnId: request.operationId,
          createdAt: new Date().toISOString(), model: demoModel,
          content: request.content as Extract<Block, { type: 'user' }>['content'], preamble: null,
        },
        {
          type: 'text', id: `reply-${request.operationId}`,
          createdAt: new Date().toISOString(), model: demoModel,
          text: 'This reply follows the edited message. The later messages have been replaced.',
        },
      ]
      revision.value += 1
      accepted.add(request.operationId)
      if (outcome.value === 'disconnect') {
        throw new Error('Connection closed before edit confirmation')
      }
    },
  })
}

onBeforeUnmount(() => completion.value?.reject(new Error('Example closed')))
</script>

<template>
  <div class="flex flex-col gap-3">
    <div class="flex flex-wrap items-center gap-2">
      <Button v-for="mode in (['accept', 'hold', 'reject', 'disconnect'] as const)"
        :key="mode" size="sm" :variant="outcome === mode ? 'primary' : 'ghost'"
        @click="outcome = mode">
        {{ { accept: 'Accept', hold: 'Hold save', reject: 'Conflict', disconnect: 'Lose confirmation' }[mode] }}
      </Button>
      <Button v-if="completion" size="sm" @click="completion.resolve()">Complete save</Button>
      <Button size="sm" variant="ghost" @click="reset">Reset example</Button>
    </div>
    <div class="flex h-[32rem] overflow-hidden rounded-xl border border-line">
      <ChatSession
        :conversation="session" has-provider
        :edit-version="{ epoch: 'gallery-editing', revision }"
        :message-edit="messageEdit"
        @update:message-edit="messageEdit = $event"
        @save-scroll="(_id, state) => session.scroll = state"
      >
        <template #composer>
          <GalleryComposer
            placeholder="Ask Demi…"
            draft="An unrelated composer draft"
            v-model:message-edit="messageEdit"
            @submit-edit="submit"
          />
        </template>
      </ChatSession>
    </div>
  </div>
</template>

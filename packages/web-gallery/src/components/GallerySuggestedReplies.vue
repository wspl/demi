<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import type { UserContentBlock } from '@demicodes/protocol'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import type { SuggestedReply } from '@demicodes/web-ui/agent/suggested-replies'
import { PROJECT_ROOT } from '../fixtures/project'
import { SUGGESTION_ROUNDS, suggestedRepliesConversation } from '../fixtures/suggested-replies'
import { useTurnFlow, type SentMessage } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * Suggested replies, live: as the conversation rests after an answer, a row
 * stands under its footer, and the suggestions the small model wrote fill it
 * a moment later, as the product's arrive with the conversation's summary.
 *
 * - A click on a pill sends its words as a new message, alone; the composer's
 *   draft stays. The answer that follows offers the next round:
 *   `SUGGESTION_ROUNDS` in turn, one of them none, which keeps the row empty.
 * - A pill's Edit, on hover (a long press on a phone), puts its words into the
 *   composer, after the draft, with the focus.
 */
const flow = useTurnFlow({ id: 'suggested-replies', title: 'Login test', cwd: PROJECT_ROOT, blocks: suggestedRepliesConversation() })
const session = flow.state
const messageFiles = useMessageFiles()

/** How long the small model takes to write them, as the product's arrive after the answer. */
const WRITING_MS = 1200

const kept = ref<{ blockId: string; replies: readonly SuggestedReply[] } | null>(null)
let round = 0
let writing: ReturnType<typeof setTimeout> | undefined

function write(): void {
  clearTimeout(writing)
  const last = session.blocks.at(-1)
  if (session.phase !== 'idle' || last?.type !== 'text')
    return
  const replies = SUGGESTION_ROUNDS[round % SUGGESTION_ROUNDS.length] ?? []
  round += 1
  writing = setTimeout(() => {
    kept.value = { blockId: last.id, replies }
  }, WRITING_MS)
}

watch(() => session.phase, (phase) => {
  if (phase === 'idle')
    write()
}, { immediate: true })
onBeforeUnmount(() => clearTimeout(writing))

function sendSuggestion(text: string): void {
  flow.turn([{ type: 'text', text }], { text, attachments: [] })
}

const send = (content: UserContentBlock[], message: SentMessage) => flow.turn(content, message)
</script>

<template>
  <div class="h-full">
    <ChatSession @show-subagent="() => {}"
      :conversation="session"
      :files="messageFiles()"
      :suggested-replies="kept"
      has-provider
      @rename="session.title = $event"
      @save-scroll="(_id, state) => (session.scroll = state)"
      @retry="flow.resume()"
      @send-suggestion="sendSuggestion"
    >
      <template #composer>
        <GalleryComposer
          :running="session.phase === 'running' || flow.startingTurn.value"
          :flow="flow"
          @send="send"
          @steer="flow.steer"
          @queue="flow.queue"
          @stop="flow.stop"
          @compact="flow.compact"
        />
      </template>
    </ChatSession>
  </div>
</template>

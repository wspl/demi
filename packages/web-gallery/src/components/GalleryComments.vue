<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import type { MessageComment, UserContentBlock } from '@demicodes/protocol'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import WaitingComments from '@demicodes/web-ui/agent/comments/WaitingComments.vue'
import { anchorOfText, pictureAnchor, picturesOf } from '@demicodes/web-ui/agent/comments/anchors'
import { inReadingOrder, type ReplyCommenting } from '@demicodes/web-ui/agent/comments/comments'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { COMMENTED_REPLY_ID, REPLY_PICTURE, REPLY_PICTURE_SOURCE, TWO_COMMENTS, commentedConversation } from '../fixtures/comments'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { useTurnFlow, type SentMessage } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * Comments on a reply's words, live: the user selects text in the reply,
 * chooses Comment, writes one and presses Return. The comment marks the text
 * and waits in the composer's pill, which lists the waiting comments and whose
 * × removes them.
 *
 * - Send sends them with what is typed, the bubble showing each quoted text
 *   and its comment, and the reply keeps their marks, dimmer.
 * - Send Now in the box sends a message holding that comment alone, leaving
 *   the pill and the typed text as they are, steering while a turn runs.
 * - A picture of the reply takes a comment from its menu (right-click),
 *   outlined yellow with its mark at the picture's top, and its bubble shows a
 *   small copy of it as the quote.
 *
 * `seed` pins a state: two waiting comments, two sent with a message, or a
 * comment on the picture, waiting or sent. Comments stay in the specimen,
 * where the product keeps them in the conversation's draft.
 */
const props = defineProps<{
  seed: 'waiting' | 'sent' | 'picture-waiting' | 'picture-sent'
}>()

const flow = useTurnFlow({ id: `comments-${props.seed}`, title: 'Slow uploads', cwd: WORKSPACE_ROOT, blocks: commentedConversation() })
const session = flow.state
const messageFiles = useMessageFiles()
const waiting = ref<MessageComment[]>([])

const commenting: ReplyCommenting = {
  get waiting() {
    return waiting.value
  },
  add: (comment) => (waiting.value = [...waiting.value, comment]),
  edit: (id, text) => (waiting.value = waiting.value.map((each) => (each.id === id ? { ...each, text } : each))),
  remove: (id) => (waiting.value = waiting.value.filter((each) => each.id !== id)),
  // A message holding this comment alone, which steers a running turn, as the product's composer would send it.
  sendNow: (comment) => {
    const running = session.phase === 'running' || flow.startingTurn.value
    ;(running ? flow.steer : flow.turn)([], { text: '', attachments: [] }, [comment])
  },
}

const ordered = computed(() => inReadingOrder(waiting.value, session.blocks.map((block) => block.id)))

/** Send, Steer and Queue each carry the waiting comments, which leave the pill, as the product's composer sends them. */
function carrying(go: (content: UserContentBlock[], message: SentMessage, comments: MessageComment[]) => void) {
  return (content: UserContentBlock[], message: SentMessage): void => {
    go(content, message, ordered.value)
    waiting.value = []
  }
}
const send = carrying(flow.turn)
const steer = carrying(flow.steer)
const queue = carrying(flow.queue)

// The seed's comments are found by the words they quote once the reply has rendered.
onMounted(async () => {
  await nextTick()
  await new Promise((done) => requestAnimationFrame(done))
  const root = document.querySelector<HTMLElement>(`[data-comments-seed="${props.seed}"] [data-reply-text="${COMMENTED_REPLY_ID}"] .markdown-body`)
  if (!root)
    return
  if (props.seed === 'picture-waiting' || props.seed === 'picture-sent') {
    // Where the reply's first picture stands, as its menu's Comment reads it.
    const picture = picturesOf(root)[0]
    const anchor = picture ? pictureAnchor(root, picture) : null
    if (!anchor)
      return
    const comment: MessageComment = {
      id: `${props.seed}-picture`,
      blockId: COMMENTED_REPLY_ID,
      ...anchor,
      quote: REPLY_PICTURE.quote,
      picture: REPLY_PICTURE_SOURCE,
      text: 'Is this the 3G test or the Wi‑Fi one?',
    }
    if (props.seed === 'picture-waiting')
      waiting.value = [comment]
    else
      flow.turn([], { text: '', attachments: [] }, [comment])
    return
  }
  const comments = TWO_COMMENTS.flatMap((each, index): MessageComment[] => {
    const anchor = anchorOfText(root, each.quote)
    return anchor ? [{ id: `${props.seed}-${index}`, blockId: COMMENTED_REPLY_ID, ...anchor, at: anchor.start, quote: each.quote, text: each.text }] : []
  })
  if (props.seed === 'waiting') {
    waiting.value = comments
    return
  }
  flow.turn([{ type: 'text', text: 'Otherwise go ahead.' }], { text: 'Otherwise go ahead.', attachments: [] }, inReadingOrder(comments, [COMMENTED_REPLY_ID]))
})
</script>

<template>
  <div :data-comments-seed="seed" class="h-full">
    <ChatSession @show-subagents="() => {}"
      :conversation="session"
      :files="messageFiles()"
      :commenting="commenting"
      has-provider
      @rename="session.title = $event"
      @save-scroll="(_id, state) => (session.scroll = state)"
      @retry="flow.resume()"
    >
      <template #composer>
        <GalleryComposer
          :running="session.phase === 'running' || flow.startingTurn.value"
          :flow="flow"
          @send="send"
          @steer="steer"
          @queue="queue"
          @stop="flow.stop"
          @compact="flow.compact"
        >
          <template v-if="ordered.length > 0" #context>
            <WaitingComments
              :comments="ordered"
              @edit="commenting.edit"
              @remove="commenting.remove"
              @clear="waiting = []"
            />
          </template>
        </GalleryComposer>
      </template>
    </ChatSession>
  </div>
</template>

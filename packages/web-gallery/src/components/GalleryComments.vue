<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import type { MessageComment, UserContentBlock } from '@demicodes/protocol'
import { showToast } from '@demicodes/web-ui/infra/toast'
import { galleryMessageFiles } from '../fixtures/written-files'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import WaitingComments from '@demicodes/web-ui/agent/comments/WaitingComments.vue'
import { anchorOfText, pictureAnchor, picturesOf } from '@demicodes/web-ui/agent/comments/anchors'
import { inReadingOrder, type ReplyCommenting, type WaitingComment } from '@demicodes/web-ui/agent/comments/comments'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { COMMENTED_REPLY_ID, REPLY_PICTURE, REPLY_PICTURE_SOURCE, TWO_COMMENTS, commentedConversation } from '../fixtures/comments'
import { PROJECT_ROOT } from '../fixtures/project'
import { useTurnFlow, type SentMessage } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * Comments on a reply's words, live: the user selects text in the reply,
 * writes one in the comment field and presses Return. The comment marks the text,
 * its box stands under the line the text ends on, and it waits in the
 * composer's pill, whose click steps through the comments and whose × removes them.
 *
 * - Send sends them with what is typed, the bubble showing each quoted text
 *   and its comment, and the reply keeps their marks and boxes, dimmer.
 * - Command-Return in the comment field sends a message holding that comment
 *   alone, leaving the pill and the typed text as they are, steering while a
 *   turn runs.
 * - A picture of the reply takes a comment from its menu (right-click), which
 *   opens the same field; the picture is then outlined yellow with the box
 *   under it, and the bubble shows a small copy of it as the quote.
 *
 * `seed` pins a state: two waiting comments, two sent with a message, or a
 * comment on the picture, waiting or sent. Comments stay in the specimen,
 * where the product keeps them in the conversation's draft.
 */
const props = defineProps<{
  seed: 'waiting' | 'sent' | 'picture-waiting' | 'picture-sent'
}>()

const flow = useTurnFlow({ id: `comments-${props.seed}`, title: 'Slow uploads', cwd: PROJECT_ROOT, blocks: commentedConversation() })
const session = flow.state
const messageFiles = useMessageFiles()
const waiting = ref<WaitingComment[]>([])
/** The comments' files, as the conversation's are in the product. */
const page = galleryMessageFiles()

/** The comments as a sent message holds them, once their files are uploaded; a toast says why when one is not. */
async function sentComments(comments: readonly WaitingComment[]): Promise<MessageComment[] | null> {
  try {
    return await Promise.all(comments.map(async ({ content, ...comment }) => ({ ...comment, content: await page.content(content) })))
  } catch (error) {
    showToast({ title: error instanceof Error ? error.message : String(error), tone: 'danger' })
    return null
  }
}

const commenting: ReplyCommenting = {
  get waiting() {
    return waiting.value
  },
  files: page.files,
  add: (comment) => (waiting.value = [...waiting.value, comment]),
  edit: (id, content) => (waiting.value = waiting.value.map((each) => (each.id === id ? { ...each, content } : each))),
  remove: (id) => (waiting.value = waiting.value.filter((each) => each.id !== id)),
  // A message holding this comment alone, which steers a running turn, as the product's composer would send it.
  sendNow: async (comment) => {
    const sent = await sentComments([comment])
    if (!sent)
      return
    const running = session.phase === 'running' || flow.startingTurn.value
    ;(running ? flow.steer : flow.turn)([], { text: '', attachments: [] }, sent)
  },
}

const ordered = computed(() => inReadingOrder(waiting.value, session.blocks.map((block) => block.id)))

/**
 * Send, Steer and Queue each carry the waiting comments, which leave the
 * pill, as the product's composer sends them, once their files are uploaded.
 */
function carrying(go: (content: UserContentBlock[], message: SentMessage, comments: MessageComment[]) => void) {
  return (content: UserContentBlock[], message: SentMessage): void => {
    const carried = ordered.value
    waiting.value = []
    void sentComments(carried).then((sent) => {
      if (sent)
        go(content, message, sent)
      else
        waiting.value = [...carried, ...waiting.value]
    })
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
    const comment = {
      id: `${props.seed}-picture`,
      blockId: COMMENTED_REPLY_ID,
      ...anchor,
      quote: REPLY_PICTURE.quote,
      picture: REPLY_PICTURE_SOURCE,
    }
    const text = 'Is this the 3G test or the Wi‑Fi one?'
    if (props.seed === 'picture-waiting')
      waiting.value = [{ ...comment, content: { text, fileIds: [] } }]
    else
      flow.turn([], { text: '', attachments: [] }, [{ ...comment, content: [{ type: 'text', text }] }])
    return
  }
  const anchors = TWO_COMMENTS.flatMap((each, index) => {
    const anchor = anchorOfText(root, each.quote)
    return anchor ? [{ id: `${props.seed}-${index}`, blockId: COMMENTED_REPLY_ID, ...anchor, at: anchor.start, quote: each.quote, text: each.text }] : []
  })
  if (props.seed === 'waiting') {
    waiting.value = anchors.map(({ text, ...anchor }) => ({ ...anchor, content: { text, fileIds: [] } }))
    return
  }
  const comments = anchors.map(({ text, ...anchor }): MessageComment => ({ ...anchor, content: [{ type: 'text', text }] }))
  flow.turn([{ type: 'text', text: 'Otherwise go ahead.' }], { text: 'Otherwise go ahead.', attachments: [] }, inReadingOrder(comments, [COMMENTED_REPLY_ID]))
})
</script>

<template>
  <div :data-comments-seed="seed" class="h-full">
    <ChatSession @show-subagent="() => {}"
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
          <template v-if="ordered.length > 0" #carried>
            <WaitingComments :comments="ordered" @clear="waiting = []" />
          </template>
        </GalleryComposer>
      </template>
    </ChatSession>
  </div>
</template>

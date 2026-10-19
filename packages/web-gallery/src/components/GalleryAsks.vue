<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import type { Block, MessageComment } from '@demicodes/protocol'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { anchorOfText } from '@demicodes/web-ui/agent/comments/anchors'
import type { NewSideQuestion, ReplyAsking, ShownSideQuestion } from '@demicodes/web-ui/agent/comments/asks'
import type { ReplyCommenting } from '@demicodes/web-ui/agent/comments/comments'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { heldTranscript } from '@demicodes/web-ui/transport/protocol'
import { askAnswer, askFailure, askHistory, askMessage, askScript, askStep, type AskScript } from '../fixtures/asks'
import { COMMENTED_REPLY_ID, REPLY_PICTURE, REPLY_PICTURE_SOURCE, commentedConversation } from '../fixtures/comments'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * Side questions on a reply's words (`comments-and-ask.md` § Side
 * questions), live: select words of the reply and choose Ask, write a
 * question and press Return; the words turn purple and a card opens under
 * their block, titled Ask until the small model's title arrives, with one
 * status line while the answer is made and then the answer. Reply asks a
 * follow-up, × folds the card into the margin's mark, the mark or the words
 * open it again, ↗ opens the whole side conversation in the subagent
 * window, and a failure's Retry runs the turn again. The answers are
 * scripted and arrive step by step; nothing reaches a backend. A picture of
 * the reply takes a side question from its menu (right-click), outlined in
 * purple, and the side conversation's first message carries the picture
 * (`comments-and-ask.md` § Images). `seed` pins a state, and every control
 * still acts from it.
 */
const props = defineProps<{
  seed: 'asking' | 'working' | 'answered' | 'follow-up' | 'folded' | 'failed' | 'picture' | 'live'
}>()

const flow = useTurnFlow({ id: `asks-${props.seed}`, title: 'Slow uploads', cwd: WORKSPACE_ROOT, blocks: commentedConversation() })
const session = flow.state
const messageFiles = useMessageFiles()
const root = ref<HTMLElement>()

// The replies take comments too, as the product's do.
const comments = ref<MessageComment[]>([])
const commenting: ReplyCommenting = {
  get waiting() {
    return comments.value
  },
  add: (comment) => (comments.value = [...comments.value, comment]),
  edit: (id, text) => (comments.value = comments.value.map((each) => (each.id === id ? { ...each, text } : each))),
  remove: (id) => (comments.value = comments.value.filter((each) => each.id !== id)),
  // A message holding this comment alone, as the product's composer would send it.
  sendNow: (comment) => {
    const running = session.phase === 'running' || flow.startingTurn.value
    ;(running ? flow.steer : flow.turn)([], { text: '', attachments: [] }, [comment])
  },
}

/** A side question of the specimen: its record, its transcript, and the script its answers follow. */
interface GallerySide {
  question: Omit<ShownSideQuestion, 'history' | 'failures' | 'working'>
  blocks: Block[]
  working: boolean
  script: AskScript
  turns: number
}

const sides = ref<GallerySide[]>([])
let asked = 0

const timers = new Set<ReturnType<typeof setTimeout>>()
function later(ms: number, run: () => void): void {
  const timer = setTimeout(() => {
    timers.delete(timer)
    run()
  }, ms)
  timers.add(timer)
}
/** What opens the follow-up specimen's field once its card is drawn. */
let opened: MutationObserver | null = null

onBeforeUnmount(() => {
  for (const timer of timers)
    clearTimeout(timer)
  opened?.disconnect()
})

const STEP_MS = 1200

function sideOf(id: string): GallerySide | undefined {
  return sides.value.find((side) => side.question.id === id)
}

/** Its turn works through its script's steps, then answers, or fails when `fails`. */
function work(id: string, fails = false): void {
  const side = sideOf(id)
  if (!side)
    return
  side.working = true
  const turn = side.turns - 1
  // The small model titles the side conversation from its first question while the answer is made.
  if (side.question.title === null)
    later(STEP_MS / 2, () => (side.question.title = side.script.title))
  side.script.steps.forEach((step, index) => {
    later(STEP_MS * (index + 1), () => side.blocks.push(askStep(id, turn, index, step)))
  })
  later(STEP_MS * (side.script.steps.length + 1), () => {
    side.blocks.push(fails ? askFailure(id, turn) : askAnswer(id, turn, side.script.answer))
    side.working = false
  })
}

function start(question: NewSideQuestion): GallerySide {
  const side: GallerySide = {
    question: { ...question, title: null, folded: false, askedAt: new Date().toISOString() },
    blocks: [...askHistory()],
    working: false,
    script: askScript(question.question, asked++),
    turns: 1,
  }
  side.blocks.push(askMessage(question.id, 0, question.quote, question.question, question.picture ?? null))
  sides.value = [...sides.value, side]
  return sideOf(question.id)!
}

const asking: ReplyAsking = {
  get questions() {
    return sides.value.map((side) => ({
      ...side.question,
      history: heldTranscript(side.blocks.length, [{ start: 0, blocks: side.blocks }]),
      failures: {},
      working: side.working,
    }))
  },
  ask: (question) => {
    start(question)
    work(question.id)
  },
  followUp: (id, text) => {
    const side = sideOf(id)
    if (!side)
      return
    side.blocks.push(askMessage(id, side.turns, side.question.quote, text))
    side.turns += 1
    side.script = askScript(text, asked++)
    work(id)
  },
  retry: (id) => {
    const side = sideOf(id)
    if (!side)
      return
    side.blocks = side.blocks.filter((block) => block.type !== 'error')
    work(id)
  },
  fold: (id, folded) => {
    const side = sideOf(id)
    if (side)
      side.question.folded = folded
  },
  // The specimen holds every side conversation whole.
  read: () => {},
}

/** The pinned state: the question on the reply's words, at the point `seed` names. */
function applySeed(anchor: { start: number; end: number }): void {
  const quote = 'the same 30-second timeout'
  const side = props.seed === 'picture'
    ? start({ id: 'ask-picture', blockId: COMMENTED_REPLY_ID, ...REPLY_PICTURE, picture: REPLY_PICTURE_SOURCE, question: 'Which connection was this measured on?' })
    : start({ id: `ask-${props.seed}`, blockId: COMMENTED_REPLY_ID, ...anchor, quote, question: 'Why is the timeout the same for uploads?' })
  const id = side.question.id
  if (props.seed === 'asking') {
    side.working = true
    return
  }
  side.question.title = side.script.title
  const [first] = side.script.steps
  if (first)
    side.blocks.push(askStep(id, 0, 0, first))
  if (props.seed === 'working') {
    side.working = true
    return
  }
  if (props.seed === 'failed') {
    side.blocks.push(askFailure(id, 0))
    return
  }
  side.script.steps.slice(1).forEach((step, index) => side.blocks.push(askStep(id, 0, index + 1, step)))
  side.blocks.push(askAnswer(id, 0, side.script.answer))
  side.question.folded = props.seed === 'folded'
}

onMounted(async () => {
  if (props.seed === 'live')
    return
  await nextTick()
  await new Promise((done) => requestAnimationFrame(done))
  const reply = root.value?.querySelector<HTMLElement>(`[data-reply-text="${COMMENTED_REPLY_ID}"] .markdown-body`)
  const anchor = reply ? anchorOfText(reply, 'the same 30-second timeout') : null
  if (!anchor)
    return
  applySeed(anchor)
  if (props.seed !== 'follow-up')
    return
  // Reply's field open with a follow-up typed, as a press on Reply and typing
  // leave it, once the card is drawn: the list draws a row only in view.
  opened = new MutationObserver(() => {
    const card = root.value?.querySelector<HTMLElement>('[data-ask-card]')
    if (!card)
      return
    opened?.disconnect()
    ;[...card.querySelectorAll<HTMLElement>('[role="button"]')].at(-1)?.click()
    void nextTick(() => {
      const field = card.querySelector('textarea')
      if (field) {
        field.value = 'Would a longer timeout alone fix it?'
        field.dispatchEvent(new Event('input'))
      }
    })
  })
  opened.observe(root.value!, { childList: true, subtree: true })
})
</script>

<template>
  <div ref="root" :data-asks-seed="seed" class="h-full">
    <ChatSession @show-subagents="() => {}"
      :conversation="session"
      :files="messageFiles()"
      :commenting="commenting"
      :asking="asking"
      has-provider
      @rename="session.title = $event"
      @save-scroll="(_id, state) => (session.scroll = state)"
      @retry="flow.resume()"
    >
      <template #composer>
        <GalleryComposer
          :running="session.phase === 'running' || flow.startingTurn.value"
          :flow="flow"
          @send="flow.turn"
          @steer="flow.steer"
          @queue="flow.queue"
          @stop="flow.stop"
          @compact="flow.compact"
        />
      </template>
    </ChatSession>
  </div>
</template>

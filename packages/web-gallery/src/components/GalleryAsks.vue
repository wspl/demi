<script setup lang="ts">
import { nextTick, onBeforeUnmount, onMounted, ref } from 'vue'
import type { Block, MessageComment } from '@demicodes/protocol'
import ChatSession from '@demicodes/web-ui/agent/ChatSession.vue'
import { anchorOfText } from '@demicodes/web-ui/agent/comments/anchors'
import type { NewSideQuestion, ReplyAsking, ShownSideQuestion } from '@demicodes/web-ui/agent/comments/asks'
import type { ReplyCommenting } from '@demicodes/web-ui/agent/comments/comments'
import { useMessageFiles } from '@demicodes/web-ui/markdown/message-files'
import { heldTranscript } from '@demicodes/web-ui/transport/protocol'
import { askAnswer, askFailure, askHistory, askMessage, askScript, askStep, askStopped, type AskScript } from '../fixtures/asks'
import { COMMENTED_REPLY_ID, REPLY_PICTURE, REPLY_PICTURE_SOURCE, commentedConversation } from '../fixtures/comments'
import { WORKSPACE_ROOT } from '../fixtures/workspace'
import { useTurnFlow } from '../turn-flow'
import GalleryComposer from './GalleryComposer.vue'

/**
 * Side questions on a reply's words, live. The user selects words of the reply,
 * writes a question in the comment field and chooses Ask. The words turn purple and a card
 * opens under the line they end on, titled Ask until the small model's title arrives, with one
 * status line while the answer is made and then the answer.
 *
 * The card's controls:
 *
 * - Stop, while it works, ends the answer where it is: the card keeps what was
 *   answered and says Stopped.
 * - Reply asks a follow-up.
 * - The header or its chevron folds the card to its header and opens it again.
 * - Delete, in the card's menu (right-click) of a question still on its first
 *   answer, deletes the question with its card, as if it had not been asked.
 * - ↗ opens the whole side conversation in the subagent window.
 * - A failure's Retry runs the turn again.
 *
 * The answers are scripted and arrive step by step; nothing reaches a backend. A
 * picture of the reply takes a side question from its menu (right-click), outlined in
 * purple, and the side conversation's first message carries the picture. `seed` pins
 * a state, and every control still acts from it.
 */
const props = defineProps<{
  seed: 'asking' | 'working' | 'stopped' | 'stopped-empty' | 'answered' | 'follow-up' | 'folded' | 'failed' | 'picture' | 'live'
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
  question: Omit<ShownSideQuestion, 'history' | 'failures' | 'working' | 'activity'>
  blocks: Block[]
  working: boolean
  /** When its turn began to wait for the model, as the server would tell it; null when it waits for none. */
  waitingSince: number | null
  script: AskScript
  turns: number
}

const sides = ref<GallerySide[]>([])
let asked = 0

/** The pending timers, each with the owner whose script it runs: a side question's turn by its id, a title by `title:` and its id. */
const timers = new Map<ReturnType<typeof setTimeout>, string>()
function later(id: string, ms: number, run: () => void): void {
  const timer = setTimeout(() => {
    timers.delete(timer)
    run()
  }, ms)
  timers.set(timer, id)
}
/** What opens the follow-up specimen's field once its card is drawn. */
let opened: MutationObserver | null = null

onBeforeUnmount(() => {
  for (const timer of timers.keys())
    clearTimeout(timer)
  opened?.disconnect()
})

const STEP_MS = 1200
/** The answer streams a few words at a time. */
const WORDS_MS = 250
const WORDS_PER_TICK = 2

function sideOf(id: string): GallerySide | undefined {
  return sides.value.find((side) => side.question.id === id)
}

/** The server tells the side's wait, from `since`, or that it waits for none. */
function tellWait(side: GallerySide, since: number | null): void {
  side.waitingSince = since
}

/** Its turn works through its script's steps, then answers, or fails when `fails`. */
function work(id: string, fails = false): void {
  const side = sideOf(id)
  if (!side)
    return
  side.working = true
  tellWait(side, Date.now())
  const turn = side.turns - 1
  // The small model titles the side conversation from its first question while the answer is made.
  if (side.question.title === null)
    later(`title:${id}`, STEP_MS / 2, () => (side.question.title = side.script.title))
  side.script.steps.forEach((step, index) => {
    // Each step ended when its block comes, and the model is asked again.
    later(id, STEP_MS * (index + 1), () => {
      side.blocks.push(askStep(id, turn, index, step))
      tellWait(side, Date.now())
    })
  })
  later(id, STEP_MS * (side.script.steps.length + 1), () => {
    if (fails) {
      side.blocks.push(askFailure(id, turn))
      side.working = false
      tellWait(side, null)
      return
    }
    stream(side, turn)
  })
}

/** The answer's text grows block by block, as a model's streams; the turn ends with its last words. */
function stream(side: GallerySide, turn: number): void {
  const id = side.question.id
  const words = side.script.answer.split(/(?<=\s)/)
  const first = askAnswer(id, turn, '')
  const index = side.blocks.length
  let shown = 0
  const write = (): void => {
    shown = Math.min(words.length, shown + WORDS_PER_TICK)
    const block = { ...first, text: words.slice(0, shown).join('') }
    if (index < side.blocks.length)
      side.blocks[index] = block
    else
      side.blocks.push(block)
    tellWait(side, null)
    if (shown < words.length)
      later(id, WORDS_MS, write)
    else
      side.working = false
  }
  write()
}

/** Stop: the pending script is dropped and the turn ends with the stop record, whatever it wrote so far kept. */
function stop(id: string): void {
  const side = sideOf(id)
  if (!side || !side.working)
    return
  for (const [timer, owner] of timers) {
    if (owner === id) {
      clearTimeout(timer)
      timers.delete(timer)
    }
  }
  side.blocks.push(askStopped(id, side.turns - 1))
  side.working = false
  tellWait(side, null)
}

/** × on a question still on its first answer: it stops and goes with its card, as if it had not been asked. */
function remove(id: string): void {
  for (const [timer, owner] of timers) {
    if (owner === id || owner === `title:${id}`) {
      clearTimeout(timer)
      timers.delete(timer)
    }
  }
  sides.value = sides.value.filter((side) => side.question.id !== id)
}

function start(question: NewSideQuestion): GallerySide {
  const side: GallerySide = {
    question: { ...question, title: null, folded: false, askedAt: new Date().toISOString() },
    blocks: [...askHistory()],
    working: false,
    waitingSince: null,
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
      activity: {
        pendingCalls: [],
        waitingSince: side.waitingSince,
        actionSent: false,
      },
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
  stop,
  remove,
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
    side.waitingSince = Date.now()
    return
  }
  side.question.title = side.script.title
  const [first] = side.script.steps
  if (first)
    side.blocks.push(askStep(id, 0, 0, first))
  if (props.seed === 'working') {
    side.working = true
    side.waitingSince = Date.now()
    return
  }
  if (props.seed === 'stopped-empty') {
    side.blocks.push(askStopped(id, 0))
    return
  }
  if (props.seed === 'failed') {
    side.blocks.push(askFailure(id, 0))
    return
  }
  side.script.steps.slice(1).forEach((step, index) => side.blocks.push(askStep(id, 0, index + 1, step)))
  if (props.seed === 'stopped') {
    // Stopped part-way through its answer, in the middle of a sentence.
    side.blocks.push(askAnswer(id, 0, side.script.answer.split(' ').slice(0, 22).join(' ')), askStopped(id, 0))
    return
  }
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
    <ChatSession @show-subagent="() => {}"
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

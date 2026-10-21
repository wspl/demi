import { afterEach, beforeEach, expect, test } from 'bun:test'
import { until } from '@vueuse/core'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { reactive } from 'vue'
import type { AgentEntry } from '@demicodes/protocol'
import { heldTranscript } from '@demicodes/web-ui/transport/protocol'
import { NO_COST } from '@demicodes/web-ui/agent/context-cost'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import type { Conversation } from '../state/types'
import { takeAgents } from './activity'
import { useConversations } from './store'
import { useSideQuestions } from './side-questions'

// Cost: a played sync channel and a stubbed fetch; milliseconds.

const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const PATH = `/api/conversations/${CONVERSATION}/side-questions`
const SIDE = 'side-1'
const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The answers the backend gives each change, in order; each request takes the next. */
let changes: Array<() => Response | Promise<Response>>
/** The bodies of the changes the page sent, in order, read reactively. */
const sent: unknown[] = reactive([])

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  changes = []
  sent.length = 0
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith(PATH)) {
      sent.push(JSON.parse(String(init?.body ?? 'null')))
      const answer = changes.shift()
      if (!answer) {
        throw new Error(`The test gave no answer for ${init?.method} ${path}`)
      }
      return answer()
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
})

afterEach(() => {
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

function record(folded: boolean, title: string | null = null) {
  return {
    id: SIDE,
    blockId: 'reply-1',
    start: 4,
    end: 15,
    quote: 'stale cache',
    question: 'Why stale?',
    title,
    folded,
    askedAt: '2026-10-18T07:00:00.000Z',
  }
}

/** The side question's entry among the conversation's agents. */
function entry(folded: boolean, title: string | null = null): AgentEntry {
  const { id, ...role } = record(folded, title)
  return { id, role: { kind: 'side', ...role }, phase: 'idle', waitedMs: null, pendingCalls: [], context: null, cost: NO_COST, queue: [], pendingSteers: [], starting: null }
}

/** The backend's answer of a change: the side question as it is now. */
function answer(folded: boolean, title: string | null = null) {
  const { id, ...sideQuestion } = record(folded, title)
  return { id, sideQuestion }
}

const refused = () => Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Not changed' }, { status: 500 })

/** The conversation as the page holds it, its agents naming the side question at revision 1. */
function conversation(): Conversation {
  const conversations = useConversations()
  useProduct().start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Uploads')] }))
  const shown = conversations.items.find((item) => item.id === CONVERSATION)!
  takeAgents(shown, [entry(false)], { epoch: 'e', revision: 1 }, Date.now(), true)
  return shown
}

// Another page folds the card and the small model titles it: the side
// question's entry changes, and the card shows it with the transcript the
// page holds of it.
test('a side question\'s changed entry changes its card and keeps its transcript', () => {
  const shown = conversation()
  expect(shown.sides.map((side) => side.record.folded)).toEqual([false])
  const history = heldTranscript(0, [{ start: 0, blocks: [] }])
  shown.sides[0]!.history = history

  takeAgents(shown, [entry(true, 'Cache lifetime')], { epoch: 'e', revision: 2 }, Date.now(), false)
  expect(shown.sides[0]!.record).toEqual(record(true, 'Cache lifetime'))
  expect(shown.sides[0]!.history).toBe(history)
})

// × folds the card at once; a fold the backend refuses opens it again.
test('a fold shows at once and a refused one goes back', async () => {
  const sides = useSideQuestions()
  const shown = conversation()
  const asking = sides.asking(shown, () => {})

  // The backend's answer is the side question as it is now, which the page takes.
  changes.push(() => Response.json(answer(true, 'Cache lifetime')))
  asking.fold(SIDE, true)
  expect(asking.questions[0]!.folded).toBe(true)
  await until(() => asking.questions[0]!.title).toBe('Cache lifetime')

  changes.push(refused)
  asking.fold(SIDE, false)
  expect(asking.questions[0]!.folded).toBe(false)
  await until(() => asking.questions[0]!.folded).toBe(true)
})

// A question shows its card at once, working; one the backend refuses goes.
test('a new question shows at once, working, and a refused one goes', async () => {
  const sides = useSideQuestions()
  const shown = conversation()
  takeAgents(shown, [], { epoch: 'e', revision: 2 }, Date.now(), true)
  const asking = sides.asking(shown, () => {})

  changes.push(refused)
  asking.ask({ id: SIDE, blockId: 'reply-1', start: 4, end: 15, quote: 'stale cache', question: 'Why stale?' })
  expect(asking.questions.map((question) => [question.question, question.working])).toEqual([['Why stale?', true]])
  await until(() => asking.questions.length).toBe(0)
})

// The user folds the card and opens it again before the fold's answer
// comes: the writes go one after the other, the fold's answer, arriving
// last of what the page holds, and an entry's change meanwhile leave the card
// open, as the user last chose, and the backend ends open.
test('a fold and an opening go in order, and the older answer never undoes the opening', async () => {
  const sides = useSideQuestions()
  const shown = conversation()
  const asking = sides.asking(shown, () => {})

  let answerFold = (_: Response) => {}
  changes.push(() => new Promise<Response>((done) => (answerFold = done)))
  changes.push(() => Response.json(answer(false)))
  asking.fold(SIDE, true)
  await until(() => sent.length).toBe(1)
  asking.fold(SIDE, false)
  expect(asking.questions[0]!.folded).toBe(false)

  // Another page's change of the entry, which still holds the fold.
  takeAgents(shown, [entry(true, 'Cache lifetime')], { epoch: 'e', revision: 2 }, Date.now(), false)
  expect(asking.questions[0]!.title).toBe('Cache lifetime')
  expect(asking.questions[0]!.folded).toBe(false)
  expect(sent).toHaveLength(1)

  answerFold(Response.json(answer(true)))
  await until(() => sent.length).toBe(2)
  expect(asking.questions[0]!.folded).toBe(false)
  await until(() => shown.sides[0]!.folding).toBe(null)
  expect(sent).toEqual([{ folded: true }, { folded: false }])
  expect(asking.questions[0]!.folded).toBe(false)
})

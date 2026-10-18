import { afterEach, beforeEach, expect, test } from 'bun:test'
import { until } from '@vueuse/core'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { reactive, shallowRef } from 'vue'
import { heldTranscript } from '@demicodes/web-ui/transport/protocol'
import { conversationSummary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from '../state/product'
import type { SideRecord } from '../state/types'
import type { RunRevision } from '../state/revisions'
import { useSideQuestions } from './side-questions'
import { sideFor } from './side-records'

// Cost: a played sync channel and a stubbed fetch; milliseconds.

const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const PATH = `/api/conversations/${CONVERSATION}/side-questions`
const SIDE = 'side-1'
const realFetch = globalThis.fetch
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The answers the backend gives each route, in order; each request takes the next. */
let lists: Array<() => Response>
let changes: Array<() => Response | Promise<Response>>
/** The bodies of the changes the page sent, in order, read reactively. */
const sent: unknown[] = reactive([])
/** How many list reads reached the backend, read reactively. */
const reads = shallowRef(0)

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  lists = []
  changes = []
  sent.length = 0
  reads.value = 0
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path === PATH && (init?.method ?? 'GET') === 'GET') {
      reads.value += 1
      const answer = lists.shift()
      if (!answer) {
        throw new Error('The test gave no answer for this read')
      }
      return answer()
    }
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
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
})

function at(run: string, revision: number) {
  return productState({
    run,
    conversations: [conversationSummary(CONVERSATION, 'Uploads', { sideQuestionsRevision: revision })],
  })
}

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

const listed = (revision: number, folded: boolean, title: string | null = null) => () =>
  Response.json({ revision, sideQuestions: [record(folded, title)] })

const refused = () => Response.json({ requestId: 'req_test000000000001', code: 'internal_error', message: 'Not changed' }, { status: 500 })

function conversation() {
  return reactive({ id: CONVERSATION, sides: [] as SideRecord[], sidesHeld: null as RunRevision | null })
}

// Another page folds the card and the small model titles it: the summary's
// revision rises and the page reads the list again, keeping the side
// conversation's transcript it holds.
test('a newer revision in the summary reads the side questions again and keeps their transcripts', async () => {
  const product = useProduct()
  product.start()
  channels.last().connect(at('run-1', 2))
  const sides = useSideQuestions()
  const shown = conversation()
  lists.push(listed(2, false))
  sides.follow(shown)
  await until(() => shown.sides.length).toBe(1)
  const history = heldTranscript(0, [{ start: 0, blocks: [] }])
  shown.sides[0]!.history = history

  lists.push(listed(4, true, 'Cache lifetime'))
  channels.last().connect(at('run-1', 4))
  await until(() => shown.sides[0]?.record?.folded).toBe(true)
  expect(shown.sides[0]!.record?.title).toBe('Cache lifetime')
  expect(shown.sides[0]!.history).toBe(history)
  expect(reads.value).toBe(2)

  // The same revision again reads nothing.
  channels.last().connect(at('run-1', 4))
  await Promise.resolve()
  expect(reads.value).toBe(2)
})

// × folds the card at once; a fold the backend refuses opens it again.
test('a fold shows at once and a refused one goes back', async () => {
  const product = useProduct()
  product.start()
  channels.last().connect(at('run-1', 1))
  const sides = useSideQuestions()
  const shown = conversation()
  lists.push(listed(1, false))
  sides.follow(shown)
  await until(() => shown.sides.length).toBe(1)
  const asking = sides.asking(shown, () => {})

  // The backend's answer is the side question as it is now, which the page takes.
  changes.push(() => Response.json({ sideQuestion: record(true, 'Cache lifetime') }))
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
  const product = useProduct()
  product.start()
  channels.last().connect(at('run-1', 0))
  const sides = useSideQuestions()
  const shown = conversation()
  lists.push(() => Response.json({ revision: 0, sideQuestions: [] }))
  sides.follow(shown)
  await until(() => reads.value).toBe(1)
  const asking = sides.asking(shown, () => {})

  changes.push(refused)
  asking.ask({ id: SIDE, blockId: 'reply-1', start: 4, end: 15, quote: 'stale cache', question: 'Why stale?' })
  expect(asking.questions.map((question) => [question.question, question.working])).toEqual([['Why stale?', true]])
  await until(() => asking.questions.length).toBe(0)
})

// The user folds the card and opens it again before the fold's answer
// comes: the writes go one after the other, the fold's answer, arriving
// last of what the page holds, and a list read meanwhile leave the card
// open, as the user last chose, and the backend ends open.
test('a fold and an opening go in order, and the older answer never undoes the opening', async () => {
  const product = useProduct()
  product.start()
  channels.last().connect(at('run-1', 1))
  const sides = useSideQuestions()
  const shown = conversation()
  lists.push(listed(1, false))
  sides.follow(shown)
  await until(() => shown.sides.length).toBe(1)
  const asking = sides.asking(shown, () => {})

  let answerFold = (_: Response) => {}
  changes.push(() => new Promise<Response>((done) => (answerFold = done)))
  changes.push(() => Response.json({ sideQuestion: record(false) }))
  asking.fold(SIDE, true)
  await until(() => sent.length).toBe(1)
  asking.fold(SIDE, false)
  expect(asking.questions[0]!.folded).toBe(false)

  // Another page's change reads the list, which still holds the fold.
  lists.push(listed(2, true, 'Cache lifetime'))
  channels.last().connect(at('run-1', 2))
  await until(() => asking.questions[0]!.title).toBe('Cache lifetime')
  expect(asking.questions[0]!.folded).toBe(false)
  expect(sent).toHaveLength(1)

  answerFold(Response.json({ sideQuestion: record(true) }))
  await until(() => sent.length).toBe(2)
  expect(asking.questions[0]!.folded).toBe(false)
  await until(() => shown.sides[0]!.folding).toBe(null)
  expect(sent).toEqual([{ folded: true }, { folded: false }])
  expect(asking.questions[0]!.folded).toBe(false)
})

// Another page asks: the side question's frames come before the list read
// their announcement causes names it. The page keeps them, shows nothing
// for a side question the list has not named, and shows it with what came
// once the list names it.
test('frames of a side question the list has not named yet are kept until it does', async () => {
  const product = useProduct()
  product.start()
  channels.last().connect(at('run-1', 1))
  const sides = useSideQuestions()
  const shown = conversation()
  lists.push(() => Response.json({ revision: 1, sideQuestions: [] }))
  sides.follow(shown)
  await until(() => reads.value).toBe(1)
  const asking = sides.asking(shown, () => {})

  const history = heldTranscript(0, [{ start: 0, blocks: [] }])
  sideFor(shown, SIDE).history = history
  expect(asking.questions).toEqual([])

  lists.push(listed(2, false))
  channels.last().connect(at('run-1', 2))
  await until(() => asking.questions.length).toBe(1)
  expect(asking.questions[0]!.history).toBe(history)
})

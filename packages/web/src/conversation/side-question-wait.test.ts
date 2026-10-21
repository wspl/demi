import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test'

// A side question's window, as the session view opens it from the card's ↗:
// the window component on a happy-dom page over the side question the
// page's store holds, which the backend's answers and the socket's frames,
// played by the test, change. A fixed clock; a few ms besides the page's
// start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('@demicodes/web-ui/__tests__/vue-components')
const { h } = await import('vue')
const { waitFor } = await import('@demicodes/utils')
const { createPinia, disposePinia, setActivePinia } = await import('pinia')
const { default: SideQuestionWindow } = await import('@demicodes/web-ui/agent/comments/SideQuestionWindow.vue')
const { default: DockWindowArea } = await import('@demicodes/web-ui/agent/DockWindowArea.vue')
const { drawn, settled, tailRow } = await import('@demicodes/web-ui/agent/__tests__/drawn-list')
const { providerWaitLabel } = await import('@demicodes/web-ui/agent/thinking-label')
const { t } = await import('@demicodes/web-ui/agent/locales/text')
const { NO_COST } = await import('@demicodes/web-ui/agent/context-cost')
const { conversationSummary, productState } = await import('../__tests__/product-state')
const { playChannels } = await import('../__tests__/sync-channel')
const { useProduct } = await import('../state/product')
const { applyConversationEvent, takeAgents } = await import('./activity')
const { useConversations } = await import('./store')
const { useSideQuestions } = await import('./side-questions')
type AgentEntry = import('@demicodes/protocol').AgentEntry
type Block = import('@demicodes/protocol').Block
type Conversation = import('../state/types').Conversation

const CONVERSATION = '0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b'
const PATH = `/api/conversations/${CONVERSATION}/side-questions`
const SIDE = 'side-1'
const PRESS = Date.parse('2026-10-21T08:00:00.000Z')
const model = {
  providerId: 'stub', thinking: null, serviceTierId: null,
  model: { id: 'stub', name: 'Stub', contextWindow: 1000, outputLimit: null, thinking: [], acceptedExtensions: [] },
}
const realFetch = globalThis.fetch
const now = spyOn(Date, 'now')
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The backend's answers to the side question requests, in order; each request takes the next. */
let answers: Array<() => Promise<Response>>
const cleanups: (() => void)[] = []

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  answers = []
  globalThis.fetch = (async (input) => {
    const path = String(input)
    const answer = path.startsWith(PATH) ? answers.shift() : undefined
    if (!answer) {
      throw new Error(`Unexpected request: ${path}`)
    }
    return answer()
  }) as typeof fetch
})

afterEach(() => {
  for (const cleanup of cleanups.splice(0)) {
    cleanup()
  }
  useConversations().stopAll()
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
  now.mockReset()
})

/** What the user asks of the words. */
const words = { blockId: 'reply-1', start: 4, end: 15, quote: 'stale cache', question: 'Why stale?' }
/** The side question the user asks, under the id the page chose. */
const newQuestion = { id: SIDE, ...words }
/** The side question as the backend keeps it. */
const question = { ...words, title: null, folded: false, askedAt: '2026-10-21T08:00:00.000Z' }

/** The side question's entry among the conversation's agents. */
function entry(parts: Partial<AgentEntry> = {}): AgentEntry {
  return {
    id: SIDE, role: { kind: 'side', ...question }, phase: 'idle', waitedMs: null, pendingCalls: [],
    context: null, cost: NO_COST, queue: [], pendingSteers: [], starting: null, ...parts,
  }
}

/** The backend took the question: its answer is the side question as it is now. */
const taken = () => Promise.resolve(Response.json({ id: SIDE, sideQuestion: question }))

/** The user asks the side question, and the backend takes it. */
async function asked(shown: Conversation): Promise<void> {
  useSideQuestions().asking(shown, () => {}).ask(newQuestion)
  await waitFor(() => shown.sides[0]?.sent?.taken === true, () => 'the backend took the question')
}

/** The conversation as the page holds it, its agents at revision 1, without the side question. */
function conversation(): Conversation {
  const conversations = useConversations()
  useProduct().start()
  channels.last().connect(productState({ conversations: [conversationSummary(CONVERSATION, 'Uploads')] }))
  const shown = conversations.items.find((item) => item.id === CONVERSATION)!
  takeAgents(shown, [], { epoch: 'e', revision: 1 }, Date.now(), true)
  return shown
}

/** The side question's own blocks: its question, then `more`. */
function sideTranscript(...more: Block[]): Block[] {
  return [
    { type: 'user', id: 'q-1', turnId: SIDE, createdAt: question.askedAt, model, content: [{ type: 'text', text: 'Why stale?' }], preamble: null },
    ...more,
  ]
}

/** The side question's window, opened on what the store shows of it; answers what its tail row says. */
function openWindow(shown: Conversation): { row: () => Promise<string | null>, text: () => Promise<string> } {
  const asking = useSideQuestions().asking(shown, () => {})
  const view = drawn(() => h(DockWindowArea, { area: 600 }, () => [
    h(SideQuestionWindow, { question: asking.questions.find((question) => question.id === SIDE) ?? null, onClose: () => {}, onRead: () => {} }),
  ]))
  cleanups.push(view.unmount)
  return {
    row: async () => {
      await settled()
      return tailRow(view.host)
    },
    text: async () => {
      await settled()
      return view.host.textContent ?? ''
    },
  }
}

// The window opened right after the press, before the backend answered and
// before the page holds any of the side question's transcript. Before, the
// window drew no list until its first question was held, so the Requesting
// row had nowhere to show. It shows no time until the server tells the wait,
// then the server's.
test('a side question\'s window opened right after the press says Requesting, then the server\'s count', async () => {
  const shown = conversation()
  answers.push(() => new Promise(() => {}))
  now.mockReturnValue(PRESS)
  useSideQuestions().asking(shown, () => {}).ask(newQuestion)
  now.mockReturnValue(PRESS + 2_000)
  const window = openWindow(shown)
  expect(await window.row()).toBe(t('transcript.thinking.requesting'))

  now.mockReturnValue(PRESS + 5_000)
  takeAgents(shown, [entry({ phase: 'running', waitedMs: 3_000 })], { epoch: 'e', revision: 2 }, Date.now(), true)
  expect(await window.row()).toBe(providerWaitLabel(3_000))
})

// The question goes while the socket is away, the backend takes it, and its
// whole turn runs and ends before the socket is back. The page saw the side
// question idle before and after; what the socket brings back, the question
// in its transcript, shows the server had it. Before, the press waited to see
// its turn run and end, and the window said Requesting for good.
test('a side question whose whole turn ran while the socket was away shows no Requesting once the socket is back', async () => {
  const shown = conversation()
  answers.push(taken)
  now.mockReturnValue(PRESS)
  await asked(shown)
  const window = openWindow(shown)
  // Taken, though the socket has not shown it yet: Requesting, without a time.
  now.mockReturnValue(PRESS + 5_000)
  expect(await window.row()).toBe(t('transcript.thinking.requesting'))

  now.mockReturnValue(PRESS + 40_000)
  takeAgents(shown, [entry()], { epoch: 'e', revision: 4 }, Date.now(), true)
  const blocks = sideTranscript({ type: 'text', id: 'a-1', createdAt: question.askedAt, model, text: 'The cache keeps it a day.' })
  applyConversationEvent(shown, {
    type: 'transcript_reset', node: SIDE, start: 0, length: blocks.length, blocks, version: { epoch: 'e', revision: 3 },
    versions: [], failures: {}, asked: undefined, receivedAt: Date.now(),
  } as Parameters<typeof applyConversationEvent>[1])
  expect(await window.row()).toBeNull()
})

// A side question's model writes a long call: its window shows the call by
// its title, as the conversation's does, not Requesting. Before, the window
// was given the side question's wait but not its calls being written.
test('a side question\'s call being written shows in its window', async () => {
  const shown = conversation()
  answers.push(taken)
  now.mockReturnValue(PRESS)
  await asked(shown)
  const blocks = sideTranscript()
  applyConversationEvent(shown, {
    type: 'transcript_reset', node: SIDE, start: 0, length: blocks.length, blocks, version: { epoch: 'e', revision: 1 },
    versions: [], failures: {}, asked: undefined, receivedAt: Date.now(),
  } as Parameters<typeof applyConversationEvent>[1])
  takeAgents(shown, [entry({
    phase: 'running',
    pendingCalls: [{ toolUseId: 'call-1', toolName: 'read', description: 'Read the cache settings' }],
  })], { epoch: 'e', revision: 2 }, Date.now(), true)

  const window = openWindow(shown)
  expect(await window.text()).toContain('Read the cache settings')
  expect(await window.row()).toBeNull()
})

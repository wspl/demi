import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, expect, spyOn, test } from 'bun:test'

// The transcript's Requesting row, as the session view draws the root's list
// from the conversation's state (`ChatSession.vue`, `rootActivity`): the
// list component on a happy-dom page over a runtime whose server the test
// plays, at a clock the test sets. A few ms besides the page's start.

GlobalRegistrator.register({ url: 'https://demi.test/' })
await import('../../__tests__/vue-components')
const { h, reactive } = await import('vue')
const { default: AgentMessageList } = await import('../AgentMessageList.vue')
const { ConversationRuntime } = await import('../conversation-runtime')
const { EMPTY_TRANSCRIPT } = await import('@demicodes/conversation-client')
const { pressed, waitClock } = await import('../wait-clock')
const { rootActivity } = await import('../agent-activity')
const { providerWaitLabel } = await import('../thinking-label')
const { t } = await import('../locales/text')
const { drawn, settled, tailRow } = await import('./drawn-list')
const { agentsFrame, clientHarness, rootEntry, userBlock } = await import('./agent-harness')
type RuntimeState = import('../conversation-runtime').RuntimeState

const PRESS = Date.parse('2026-10-21T08:00:00.000Z')
const now = spyOn(Date, 'now')
const cleanups: (() => void)[] = []
afterEach(() => {
  for (const cleanup of cleanups.splice(0)) {
    cleanup()
  }
  now.mockReset()
})

function state(): RuntimeState {
  return reactive<RuntimeState>({
    id: 'conversation',
    history: EMPTY_TRANSCRIPT,
    shownAt: null,
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    starting: null,
    pendingCalls: [],
    waitingSince: null,
    waitAction: null,
    requestOutput: null,
    lastError: null,
    load: 'loading',
    pendingAction: null,
    waitClock: waitClock(),
    failures: {},
    contextUsage: null,
  })
}

/** The root's list over `conversation`, with the bindings the session view gives it. */
function drawnRoot(conversation: RuntimeState): () => Promise<string | null> {
  const view = drawn(() => h(AgentMessageList, {
    conversationId: conversation.id,
    // The message the turn answers; an empty transcript says it has none.
    blocks: [userBlock('u-1', 'm-1', 'Sort the downloads into folders.')],
    activity: rootActivity(conversation),
    pendingSteers: conversation.pendingSteers,
    starting: conversation.starting,
    queue: conversation.queue,
    phase: conversation.phase,
    load: conversation.load,
    pendingAction: conversation.pendingAction,
    pendingSubmissions: [],
    bottomOffset: 0,
    persistedScrollState: undefined,
  }))
  cleanups.push(view.unmount)
  return async () => {
    await settled()
    return tailRow(view.host)
  }
}

// The socket drops during the wait a message began and comes back: the
// handshake tells the same wait again, its start reckoned from a later
// delivery. The page knows the wait by the message that began it, never by
// its start, which each delivery's latency moves; before, the row counted
// from the server's start after the reconnect, not from the press.
test('a wait the page\'s message began counts from the press when the server tells it again after a reconnect', async () => {
  const conversation = state()
  const server = clientHarness()
  const runtime = new ConversationRuntime({ state: conversation, connect: async () => server.client })
  cleanups.push(() => runtime.dispose())
  pressed(conversation.waitClock, 'm-1', PRESS)
  now.mockReturnValue(PRESS + 400)
  await runtime.connect()
  server.receive(agentsFrame([rootEntry('conversation', 'running', { waitedMs: 0, waitAction: 'm-1' })]))
  now.mockReturnValue(PRESS + 30_500)
  server.receive(agentsFrame([rootEntry('conversation', 'running', { waitedMs: 30_000, waitAction: 'm-1' })], 2))

  now.mockReturnValue(PRESS + 31_000)
  const row = drawnRoot(conversation)
  expect(await row()).toBe(providerWaitLabel(31_000))
})

// A wait no press of this page began, as another page's message's, counts
// from the server's start.
test('a wait another page began counts from the server\'s start', async () => {
  const conversation = state()
  const server = clientHarness()
  const runtime = new ConversationRuntime({ state: conversation, connect: async () => server.client })
  cleanups.push(() => runtime.dispose())
  pressed(conversation.waitClock, 'm-1', PRESS)
  now.mockReturnValue(PRESS + 10_000)
  await runtime.connect()
  server.receive(agentsFrame([rootEntry('conversation', 'running', { waitedMs: 2_000, waitAction: 'elsewhere' })]))

  now.mockReturnValue(PRESS + 12_000)
  const row = drawnRoot(conversation)
  expect(await row()).toBe(providerWaitLabel(4_000))
})

// A conversation opened from the copy the page kept shows it while its
// socket opens: the tail row says it connects, as a lost socket's does.
test('a conversation shown from its kept copy before its socket opens says Connecting', async () => {
  const conversation = state()
  conversation.load = 'opening'
  conversation.phase = 'running'
  const row = drawnRoot(conversation)
  expect(await row()).toBe(t('transcript.connecting'))
})

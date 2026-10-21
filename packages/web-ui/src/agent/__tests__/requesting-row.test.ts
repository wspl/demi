import { GlobalRegistrator } from '@happy-dom/global-registrator'
import { afterEach, beforeEach, expect, spyOn, test } from 'bun:test'

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
const { rootActivity } = await import('../agent-activity')
const { providerWaitLabel } = await import('../thinking-label')
const { t } = await import('../locales/text')
const { drawn, settled, tailRow } = await import('./drawn-list')
const { agentsFrame, clientHarness, rootEntry, userBlock } = await import('./agent-harness')
type RuntimeState = import('../conversation-runtime').RuntimeState
type PendingSubmissionState = import('../types').PendingSubmissionState

const SENT = Date.parse('2026-10-21T08:00:00.000Z')
const now = spyOn(Date, 'now')
const cleanups: (() => void)[] = []
beforeEach(() => {
  now.mockReturnValue(SENT)
})
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
    requestOutput: null,
    lastError: null,
    load: 'loading',
    pendingAction: null,
    failures: {},
    contextUsage: null,
  })
}

/** The conversation `conversation` over a socket whose server the test plays, opened. */
async function connected(conversation: RuntimeState) {
  const server = clientHarness()
  const runtime = new ConversationRuntime({ state: conversation, connect: async () => server.client })
  cleanups.push(() => runtime.dispose())
  await runtime.connect()
  return server
}

/**
 * The root's list over `conversation`, with the bindings the session view
 * gives it, and the messages on their way `sending`; each call draws it
 * anew, as a return to the conversation does.
 */
function drawnRoot(conversation: RuntimeState, sending: { messages: PendingSubmissionState[] } = { messages: [] }): () => Promise<string | null> {
  let unmount: (() => void) | null = null
  return async () => {
    unmount?.()
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
      pendingSubmissions: sending.messages,
      bottomOffset: 0,
      persistedScrollState: undefined,
    }))
    unmount = view.unmount
    cleanups.push(view.unmount)
    await settled()
    return tailRow(view.host)
  }
}

const message: PendingSubmissionState = { id: 'm-2', way: 'send', text: 'Then the photos.', attachments: [], error: null }

// The page cannot know how long a wait it has no answer about has lasted:
// before, it counted from the press, and the count moved once the server
// told the wait from its own start.
test('after Send and before the server answers, the row says Requesting without a time', async () => {
  const conversation = state()
  const server = await connected(conversation)
  server.receive(agentsFrame([rootEntry('conversation', 'idle')]))
  const sending = reactive({ messages: [message] })
  now.mockReturnValue(SENT)
  const row = drawnRoot(conversation, sending)
  expect(await row()).toBe(t('transcript.thinking.requesting'))
  now.mockReturnValue(SENT + 30_000)
  expect(await row()).toBe(t('transcript.thinking.requesting'))
})

test('once the server tells its wait, the row shows the server\'s count, not one from the press', async () => {
  const conversation = state()
  const server = await connected(conversation)
  server.receive(agentsFrame([rootEntry('conversation', 'idle')]))
  const sending = reactive({ messages: [message] })
  now.mockReturnValue(SENT + 30_000)
  // The message reached the server late: the wait it began is two seconds old.
  server.receive(agentsFrame([rootEntry('conversation', 'running', { waitedMs: 2_000 })], 2))
  const row = drawnRoot(conversation, sending)
  expect(await row()).toBe(providerWaitLabel(2_000))
  sending.messages = []
  now.mockReturnValue(SENT + 35_000)
  expect(await row()).toBe(providerWaitLabel(7_000))
})

// The server counts each wait from its start and tells it on every
// delivery; the page only shows it. Before, a list drawn again counted from
// when it was drawn, so every return restarted the clock.
test('a switch, a reconnect and a reload show the server\'s count unchanged', async () => {
  const STARTED = SENT
  const conversation = state()
  const server = await connected(conversation)
  now.mockReturnValue(STARTED + 4_000)
  server.receive(agentsFrame([rootEntry('conversation', 'running', { waitedMs: 4_000 })]))
  const row = drawnRoot(conversation)
  now.mockReturnValue(STARTED + 5_000)
  expect(await row()).toBe(providerWaitLabel(5_000))

  // A switch to another conversation and back draws the list again.
  now.mockReturnValue(STARTED + 12_000)
  expect(await row()).toBe(providerWaitLabel(12_000))

  // The socket comes back, and its handshake tells the same wait again.
  now.mockReturnValue(STARTED + 20_300)
  server.receive(agentsFrame([rootEntry('conversation', 'running', { waitedMs: 20_300 })], 2))
  now.mockReturnValue(STARTED + 21_000)
  expect(await row()).toBe(providerWaitLabel(21_000))

  // A reload holds nothing of the page before it: the handshake tells the wait.
  const reloaded = state()
  const again = await connected(reloaded)
  now.mockReturnValue(STARTED + 40_000)
  again.receive(agentsFrame([rootEntry('conversation', 'running', { waitedMs: 40_000 })]))
  now.mockReturnValue(STARTED + 41_000)
  expect(await drawnRoot(reloaded)()).toBe(providerWaitLabel(41_000))
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

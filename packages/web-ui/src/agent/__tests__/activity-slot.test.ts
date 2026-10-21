import { expect, spyOn, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { SessionPhase } from '@demicodes/protocol'
import { activitySlotKind } from '../activity-slot'
import type { SessionLoad } from '../session-status'
import { turnStartingMessage } from '../send-way'
import type { PendingSubmissionState } from '../types'
import { useActivitySlot, type ActivitySlotSources } from '../useActivitySlot'

function kind(options: {
  load?: SessionLoad
  backendAway?: boolean
  actionSent?: boolean
  startingTurn?: boolean
  waiting?: boolean
} = {}) {
  return activitySlotKind({
    load: options.load ?? 'ready',
    backendAway: options.backendAway ?? false,
    actionSent: options.actionSent ?? false,
    startingTurn: options.startingTurn ?? false,
    waiting: options.waiting ?? false,
  })
}

test('a first opening whose socket is not made yet says it connects, as a lost socket does', () => {
  expect(kind({ load: 'connecting' })).toBe('connecting')
  expect(kind({ load: 'connecting', backendAway: true })).toBeNull()
  // The copy the page kept shows while the socket opens.
  expect(kind({ load: 'opening' })).toBe('connecting')
})

test('a message sent while the first opening runs says Requesting, never Connecting; a socket that is not made yet still says Connecting', () => {
  expect(kind({ load: 'opening', startingTurn: true })).toBe('requesting')
  expect(kind({ load: 'opening', startingTurn: true, backendAway: true })).toBeNull()
  expect(kind({ load: 'connecting', startingTurn: true })).toBe('connecting')
})

test('the row says Requesting while the server says the agent waits, and nothing otherwise', () => {
  expect(kind({ waiting: true })).toBe('requesting')
  expect(kind()).toBeNull()
})

test('a message that starts a turn, a recovery or a side question this page sent says Requesting before the server has it', () => {
  expect(kind({ startingTurn: true })).toBe('requesting')
  expect(kind({ actionSent: true })).toBe('requesting')
})

test('a conversation that failed to open waits for nothing: the failure told at the tail says so', () => {
  expect(kind({ load: 'failed', waiting: true })).toBeNull()
})

test('connecting wins over a pending recovery and a wait', () => {
  expect(kind({ load: 'reconnecting' })).toBe('connecting')
  expect(kind({ load: 'reconnecting', actionSent: true })).toBe('connecting')
  expect(kind({ load: 'reconnecting', waiting: true })).toBe('connecting')
})

test('while the connection banner says the backend is away, a reconnecting conversation shows no row of its own', () => {
  const away = { load: 'reconnecting', backendAway: true } as const
  expect(kind(away)).toBeNull()
  expect(kind({ ...away, waiting: true })).toBeNull()
  expect(kind({ ...away, actionSent: true })).toBeNull()
  // Its socket lost while the page still reaches the backend: the row says so.
  expect(kind({ load: 'reconnecting', waiting: true })).toBe('connecting')
})

/**
 * The tail row as the message list drives it, over a conversation the test
 * changes the way the page's state changes, at a clock the test sets.
 */
interface Conversation {
  phase: SessionPhase
  load: SessionLoad
  pendingSubmissions: PendingSubmissionState[]
  waitingSince: number | null
  actionSent: boolean
}

function slotOver(phase: SessionPhase, opened = 0) {
  const now = spyOn(Date, 'now').mockReturnValue(opened)
  const conversation = reactive<Conversation>({
    phase,
    load: 'ready',
    pendingSubmissions: [],
    waitingSince: null,
    actionSent: false,
  })
  const scope = effectScope()
  const { slot } = scope.run(() => useActivitySlot(sourcesOf(conversation)))!
  return {
    conversation,
    slot,
    at: (ms: number) => now.mockReturnValue(ms),
    [Symbol.dispose]() {
      scope.stop()
      now.mockRestore()
    },
  }
}

/** What the message list gives the row of `conversation`. */
function sourcesOf(conversation: Conversation): ActivitySlotSources {
  return {
    input: () => ({ load: conversation.load, backendAway: false }),
    phase: () => conversation.phase,
    pendingSubmissions: () => conversation.pendingSubmissions,
    activity: () => ({
      pendingCalls: [],
      waitingSince: conversation.waitingSince,
      actionSent: conversation.actionSent,
    }),
  }
}

function sent(id = 'turn-1', error: string | null = null): PendingSubmissionState {
  return { id, way: 'send', text: 'hello', attachments: [], error }
}

function sending(conversation: Conversation, messages: PendingSubmissionState[]): void {
  conversation.pendingSubmissions = messages
}

test('a message sent to an idle conversation says Requesting without a time until the server tells its wait, then the server\'s', async () => {
  using session = slotOver('idle')
  const { conversation, slot, at } = session
  expect(slot.value).toBeNull()

  at(5_000)
  sending(conversation, [sent()])
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: null })
  at(7_000)
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: null })

  // The backend starts the turn, and its wait, before it writes the message.
  conversation.phase = 'running'
  conversation.waitingSince = 5_400
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_400 })

  // The message's block confirms it; the page drops the pending message.
  at(8_000)
  conversation.pendingSubmissions = []
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_400 })
})

test('a view shown during a wait counts from the start the server tells, not from when it opened', async () => {
  // A subagent's request had waited eight minutes when its view opened.
  using session = slotOver('running', 600_000)
  const { conversation, slot, at } = session
  conversation.waitingSince = 120_000
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 120_000 })

  at(660_000)
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 120_000 })
})

// The server says when a wait ends: the answer streams, a call's title is
// written, the turn ends. A running turn that it tells no wait for waits for
// nothing the row could count, whatever its transcript ends with.
test('a running turn the server tells no wait for shows no Requesting', async () => {
  using session = slotOver('running')
  const { conversation, slot } = session
  await nextTick()
  expect(slot.value).toBeNull()
  conversation.waitingSince = 0
  await nextTick()
  expect(slot.value?.kind).toBe('requesting')
  conversation.waitingSince = null
  await nextTick()
  expect(slot.value).toBeNull()
})

test('a recovery this page asked for says Requesting without a time until the server tells its wait', async () => {
  using session = slotOver('idle')
  const { conversation, slot } = session
  conversation.actionSent = true
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: null })
  // The root's entry acknowledges it: the runtime takes its phase and its
  // wait, then settles the action.
  conversation.phase = 'running'
  conversation.waitingSince = 3_200
  conversation.actionSent = false
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 3_200 })
})

test('a failed delivery ends the wait, and Retry says Requesting again', async () => {
  using session = slotOver('idle')
  const { conversation, slot } = session
  sending(conversation, [sent()])
  await nextTick()
  sending(conversation, [sent('turn-1', 'Connection closed before confirmation')])
  await nextTick()
  expect(slot.value).toBeNull()

  sending(conversation, [sent()])
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: null })
})

test('a message sent while the socket reconnects says Connecting, and Requesting once the socket is back', async () => {
  using session = slotOver('idle')
  const { conversation, slot } = session
  conversation.load = 'reconnecting'
  sending(conversation, [sent()])
  await nextTick()
  expect(slot.value).toEqual({ kind: 'connecting' })

  conversation.load = 'ready'
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: null })
})

// Back-to-back sends 400 ms apart, the first one failing. The composer's Stop
// and the transcript's Requesting read one rule, so the one shows exactly while
// the other does. Before, the composer kept Stop for the second message, a
// steer that goes as the next message, while the row showed nothing. About 1
// ms.
test('the composer shows Stop exactly while the row says Requesting for a message that starts a turn', async () => {
  using session = slotOver('idle')
  const { conversation, slot } = session
  // As ConversationComposer passes it: running, or a message on its way
  // that starts a turn; SessionComposer shows Stop while running or compacting.
  const stopShown = () =>
    conversation.phase !== 'idle'
    || turnStartingMessage(conversation.phase, conversation.pendingSubmissions) !== undefined
  const shown = async () => {
    await nextTick()
    return { stop: stopShown(), requesting: slot.value?.kind === 'requesting' }
  }
  const message = (id: string, way: PendingSubmissionState['way'], error: string | null = null): PendingSubmissionState =>
    ({ id, way, text: id, attachments: [], error })
  const both = (shownNow: boolean) => ({ stop: shownNow, requesting: shownNow })

  expect(await shown()).toEqual(both(false))
  sending(conversation, [message('a', 'send')])
  expect(await shown()).toEqual(both(true))
  // Sent while the first is on its way: it steers the turn the first starts.
  sending(conversation, [message('a', 'send'), message('b', 'steer')])
  expect(await shown()).toEqual(both(true))
  // The first is not delivered: the session answers the steer as the next
  // message.
  sending(conversation, [message('a', 'send', 'Upload failed'), message('b', 'steer')])
  expect(await shown()).toEqual(both(true))
  sending(conversation, [message('a', 'send', 'Upload failed'), message('b', 'steer', 'Upload failed')])
  expect(await shown()).toEqual(both(false))
  // Retry sends the first again as the next message.
  sending(conversation, [message('a', 'send'), message('b', 'steer', 'Upload failed')])
  expect(await shown()).toEqual(both(true))
  // The backend starts its turn before it writes the message.
  conversation.phase = 'running'
  expect(await shown()).toEqual(both(true))
})

test('a message sent during a turn joins the queue and adds no Requesting under the streaming answer', async () => {
  using session = slotOver('running')
  const { conversation, slot } = session
  sending(conversation, [{ ...sent(), way: 'queue' }])
  await nextTick()
  expect(slot.value).toBeNull()
})

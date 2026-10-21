import { expect, spyOn, test } from 'bun:test'
import { effectScope, nextTick, reactive } from 'vue'
import type { SessionPhase } from '@demicodes/protocol'
import { activitySlotKind } from '../activity-slot'
import type { SessionLoad } from '../session-status'
import { turnStartingMessage } from '../send-way'
import type { PendingSubmissionState } from '../types'
import { useActivitySlot, type ActivitySlotSources } from '../useActivitySlot'
import { pressed, refused, serverWaits, waitClock, type WaitClock } from '../wait-clock'

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
  waitAction: string | null
  sentAction: string | null
  clock: WaitClock
}

function slotOver(phase: SessionPhase, opened = 0) {
  const now = spyOn(Date, 'now').mockReturnValue(opened)
  const conversation = reactive<Conversation>({
    phase,
    load: 'ready',
    pendingSubmissions: [],
    waitingSince: null,
    waitAction: null,
    sentAction: null,
    clock: waitClock(),
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
      waitAction: conversation.waitAction,
      waitClock: conversation.clock,
      sentAction: conversation.sentAction,
    }),
  }
}

function sent(id = 'turn-1', error: string | null = null): PendingSubmissionState {
  return { id, way: 'send', text: 'hello', attachments: [], error }
}

// What the page's actors do, which the row reads: the store presses each
// message as it is sent, and again at its Retry, and the runtime takes the
// wait the server tells, with the action that began it.
function sending(conversation: Conversation, messages: PendingSubmissionState[]): void {
  for (const message of messages) {
    const before = conversation.pendingSubmissions.find((each) => each.id === message.id)
    if (message.error === null && (before === undefined || before.error !== null)) {
      pressed(conversation.clock, message.id)
    }
  }
  conversation.pendingSubmissions = messages
}
function serverTells(conversation: Conversation, since: number | null, action: string | null = null): void {
  conversation.waitingSince = since
  conversation.waitAction = since === null ? null : action
  serverWaits(conversation.clock, conversation.waitAction)
}

test('a message sent to an idle conversation says Requesting from the press, through its confirmation, on one clock', async () => {
  using session = slotOver('idle')
  const { conversation, slot, at } = session
  expect(slot.value).toBeNull()

  at(5_000)
  sending(conversation, [sent()])
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })

  // The backend starts the turn, and its wait, which names the message,
  // before it writes the message.
  at(6_000)
  conversation.phase = 'running'
  serverTells(conversation, 5_400, 'turn-1')
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })

  // The message's block confirms it; the page drops the pending message.
  at(8_000)
  conversation.pendingSubmissions = []
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })
})

test('a view shown during a wait counts from the start the server tells, not from when it opened', async () => {
  // A subagent's request had waited eight minutes when its view opened.
  using session = slotOver('running', 600_000)
  const { conversation, slot, at } = session
  serverTells(conversation, 120_000)
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
  serverTells(conversation, 0)
  await nextTick()
  expect(slot.value?.kind).toBe('requesting')
  serverTells(conversation, null)
  await nextTick()
  expect(slot.value).toBeNull()
})

// The request after a call's result, and the one a steer sent now starts
// again, are waits no press began: the server names no action for them, and
// the row counts from its start, as every other page does.
test('a wait within the turn counts from the server\'s start, the message\'s from its press', async () => {
  using session = slotOver('idle')
  const { conversation, slot, at } = session
  at(5_000)
  sending(conversation, [sent()])
  at(6_000)
  conversation.phase = 'running'
  serverTells(conversation, 5_400, 'turn-1')
  conversation.pendingSubmissions = []
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })

  at(9_000)
  serverTells(conversation, null)
  await nextTick()
  expect(slot.value).toBeNull()
  at(20_100)
  serverTells(conversation, 20_000)
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 20_000 })
  at(79_400)
  serverTells(conversation, 79_400)
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 79_400 })
})

// Switching to another conversation and back draws the list again. Before,
// each list counted a wait it was not told the start of from when it was
// drawn, so the clock went back to 0s at every return, while the same
// request went on.
test('a list drawn again during a wait counts on from the wait\'s start', async () => {
  using session = slotOver('idle')
  const { conversation, at } = session
  const drawn = async (ms: number) => {
    at(ms)
    const scope = effectScope()
    const { slot } = scope.run(() => useActivitySlot(sourcesOf(conversation)))!
    await nextTick()
    const shown = slot.value
    scope.stop()
    return shown
  }
  at(5_000)
  sending(conversation, [sent()])
  expect(await drawn(5_000)).toEqual({ kind: 'requesting', since: 5_000 })
  expect(await drawn(7_000)).toEqual({ kind: 'requesting', since: 5_000 })
  conversation.phase = 'running'
  serverTells(conversation, 5_400, 'turn-1')
  expect(await drawn(8_000)).toEqual({ kind: 'requesting', since: 5_000 })
  conversation.pendingSubmissions = []
  expect(await drawn(30_000)).toEqual({ kind: 'requesting', since: 5_000 })
  serverTells(conversation, 40_000)
  expect(await drawn(52_000)).toEqual({ kind: 'requesting', since: 40_000 })
})

test('a recovery this page asked for counts from the press, also once the server tells its wait', async () => {
  using session = slotOver('idle')
  const { conversation, slot, at } = session
  at(3_000)
  pressed(conversation.clock, 'resume-1')
  conversation.sentAction = 'resume-1'
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 3_000 })
  // The root's entry acknowledges it: the runtime takes its phase and its
  // wait, then settles the action.
  at(3_500)
  conversation.phase = 'running'
  serverTells(conversation, 3_200, 'resume-1')
  conversation.sentAction = null
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 3_000 })
})

test('a failed delivery ends the wait, and Retry starts it again from the retry', async () => {
  using session = slotOver('idle')
  const { conversation, slot, at } = session
  sending(conversation, [sent()])
  await nextTick()
  conversation.pendingSubmissions = [sent('turn-1', 'Connection closed before confirmation')]
  refused(conversation.clock, 'turn-1')
  await nextTick()
  expect(slot.value).toBeNull()

  at(9_000)
  sending(conversation, [sent()])
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 9_000 })
})

test('a message sent while the socket reconnects says Connecting, and Requesting from the press once the socket is back', async () => {
  using session = slotOver('idle')
  const { conversation, slot, at } = session
  conversation.load = 'reconnecting'
  at(5_000)
  sending(conversation, [sent()])
  await nextTick()
  expect(slot.value).toEqual({ kind: 'connecting' })

  at(12_000)
  conversation.load = 'ready'
  await nextTick()
  expect(slot.value).toEqual({ kind: 'requesting', since: 5_000 })
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

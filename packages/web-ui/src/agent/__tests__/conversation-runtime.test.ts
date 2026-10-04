import { expect, jest, spyOn, test } from 'bun:test'
import type { PendingSteer } from '@demicodes/protocol'
import { deferred, delay, waitFor } from '@demicodes/utils'
import { computed } from 'vue'
import { ConversationRuntime, type RuntimeState } from '../conversation-runtime'
import { ConversationSocketError, connectConversationClient } from '../../transport/conversation-socket'
import { pageReturned } from '../../transport/liveness'
import { playSockets } from '../../transport/__tests__/test-socket'
import { clientHarness, model, userBlock } from './agent-harness'

function state(): RuntimeState {
  return {
    blocks: [],
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    lastError: null,
    load: 'loading',
    pendingAction: null,
    failures: {},
    contextUsage: null,
  }
}

/**
 * One turn of the event loop, which lets the promise callbacks that are due
 * run. Bun's fake clock replaces the timers but not `setImmediate`, so this
 * works on it too.
 */
function turn(): Promise<void> {
  return new Promise((resolve) => setImmediate(resolve))
}

test('the current edit version reacts to connection, snapshots, patches and disconnect', async () => {
  const h = clientHarness()
  const runtime = new ConversationRuntime({ state: state(), connect: async () => h.client })
  const version = computed(() => runtime.transcriptVersion())
  expect(version.value).toBeNull()
  await runtime.connect()
  h.receive({ type: 'transcript_reset', version: { epoch: 'epoch', revision: 1 }, blocks: [] })
  expect(version.value).toEqual({ epoch: 'epoch', revision: 1 })
  h.receive({ type: 'transcript_patch', revision: 2, patches: [{ op: 'replace', value: [] }] })
  expect(version.value).toEqual({ epoch: 'epoch', revision: 2 })
  runtime.dispose()
  expect(version.value).toBeNull()
})

test('the usage the composer shows is the one the session reports, not one read from the transcript', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
  await runtime.connect()
  try {
    // A response before the compaction still measures the history the summary replaced.
    h.receive({ type: 'transcript_reset', version: { epoch: 'epoch', revision: 1 }, blocks: [] })
    h.receive({ type: 'context_usage', usage: { tokens: 6_000, window: 100_000, compactFrom: 50_000 } })
    expect(current.contextUsage).toEqual({ tokens: 6_000, window: 100_000, compactFrom: 50_000 })
  } finally {
    runtime.dispose()
  }
})

for (const failure of ['none', 'before-confirmation', 'before-reconciliation'] as const) {
  test(`edit confirmation preserves generation recovery: ${failure}`, async () => {
    const h = clientHarness()
    const current = state()
    const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
    await runtime.connect()
    h.receive({
      type: 'transcript_reset',
      version: { epoch: 'epoch', revision: 1 },
      blocks: failure === 'before-reconciliation' ? [] : [userBlock('target', 'old-turn', 'old')],
    })
    current.lastError = failure === 'before-reconciliation' ? 'generation failed' : 'old turn failed'
    const pending = runtime.editAndSend({
      operationId: 'edit', targetBlockId: 'target', version: { epoch: 'epoch', revision: 1 },
      content: [{ type: 'text', text: 'new' }],
    })
    try {
      await waitFor(() => h.sent.some((frame) => frame.type === 'edit_and_send'))
      if (failure === 'before-confirmation') {
        h.receive({ type: 'error', code: 'generation_failed', message: 'generation failed' })
      }
      h.receive({ type: 'edit_result', operationId: 'edit', outcome: { status: 'accepted', turnId: 'replacement' } })
      await pending
      if (failure === 'none') {
        expect(current.lastError).toBeNull()
      } else {
        expect(current.lastError).toBe('generation failed')
      }
    } finally {
      runtime.dispose()
      await pending.catch(() => {})
    }
  })
}

test('disposing a view while its socket connects never opens the late connection', async () => {
  const late = clientHarness()
  const connected = deferred<typeof late.client>()
  const runtime = new ConversationRuntime({
    state: state(),
    connect: () => connected.promise,
  })
  const opening = runtime.connect()
  const result = opening.catch((error) => error)
  runtime.dispose()
  connected.resolve(late.client)
  expect(await result).toBeInstanceOf(Error)
  expect(late.sent).toEqual([])
})

test('disposing an open view detaches without sending task-close or abort commands', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    connect: async () => h.client,
  })
  await runtime.connect()
  expect(current.load).toBe('ready')
  runtime.dispose()
  expect(h.sent.map((frame) => frame.type)).toEqual(['open'])
  await expect(runtime.connect()).rejects.toThrow('disposed')
})

test('retry reconciles an already accepted message before submitting again', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    connect: async () => h.client,
  })
  try {
    await runtime.connect()
    h.receive({
      type: 'transcript_reset',
      version: { epoch: 'runtime-test', revision: 1 },
      blocks: [userBlock('user-block', 'persisted-message', 'Already accepted')],
    })
    await runtime.submit([{ type: 'text', text: 'Already accepted' }], 'persisted-message')
    expect(h.sent.map((frame) => frame.type)).toEqual(['open'])
  } finally {
    runtime.dispose()
  }
})

test('a pending steer delivered now stops the turn, which writes it, and continues the turn', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
  try {
    await runtime.connect()
    const steer: PendingSteer = { id: 'steer', turnId: 'turn', model, content: [{ type: 'text', text: 'now' }] }
    h.receive({ type: 'phase', phase: 'running' })
    h.receive({ type: 'pending_steers', pendingSteers: [steer] })
    const delivered = runtime.interruptPendingSteer('steer')
    await waitFor(() => h.sent.some((frame) => frame.type === 'abort'))
    // The backend answers the abort before the stopped turn has saved, and
    // takes Continue only once the session is idle.
    h.receive({ type: 'abort_result', result: { target: 'active_turn', canAbortAgain: false } })
    await delay(0)
    expect(h.sent.some((frame) => frame.type === 'resume')).toBe(false)
    h.receive({ type: 'phase', phase: 'idle' })
    await waitFor(() => h.sent.some((frame) => frame.type === 'resume'))
    h.receive({ type: 'phase', phase: 'running' })
    h.receive({ type: 'phase', phase: 'idle' })
    await delivered
    expect(h.sent.map((frame) => frame.type)).toEqual(['open', 'abort', 'resume'])
  } finally {
    runtime.dispose()
  }
})

test('a queued message sent now steers the running turn, and runs next without an error when the ending turn refuses the steer', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
  try {
    await runtime.connect()
    h.receive({ type: 'queue', queue: [{ id: 'queued', content: [{ type: 'text', text: 'next' }] }] })
    // Idle: the message moves to the front and runs next.
    await runtime.sendQueuedNow('queued')
    expect(h.sent.at(-1)).toEqual({ type: 'send_queued_message', messageId: 'queued' })
    // Running, but ending: the turn refuses the steer and keeps the message queued.
    h.receive({ type: 'phase', phase: 'running' })
    const sending = runtime.sendQueuedNow('queued')
    await waitFor(() => h.sent.at(-1)?.type === 'steer_queued_message')
    const steer = h.sent.at(-1)
    if (steer?.type !== 'steer_queued_message') {
      throw new Error('a steer was expected')
    }
    h.receive({ type: 'steer_result', steerId: steer.steerId, outcome: { status: 'rejected', reason: 'The running turn is finishing' } })
    await sending
    expect(h.sent.at(-1)).toEqual({ type: 'send_queued_message', messageId: 'queued' })
  } finally {
    runtime.dispose()
  }
})

test('a view whose tree another client disposed opens it again after the first wait, as after a lost connection', async () => {
  const harnesses = [clientHarness(), clientHarness()]
  const current = state()
  let connects = 0
  const runtime = new ConversationRuntime({
    state: current,
    connect: async () => harnesses[connects++]!.client,
  })
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    await runtime.connect()
    harnesses[0]!.receive({ type: 'closed' })
    expect(current.load).toBe('reconnecting')
    jest.advanceTimersByTime(999)
    await turn()
    expect(connects).toBe(1)
    jest.advanceTimersByTime(1)
    await turn()
    expect(current.load).toBe('ready')
    expect(connects).toBe(2)
    expect(harnesses[1]!.sent.map((frame) => frame.type)).toEqual(['open'])
    expect(runtime.connected).toBe(true)
  } finally {
    random.mockRestore()
    jest.useRealTimers()
    runtime.dispose()
  }
})

test('a failure the session reported is over once another tab starts the next action', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
  try {
    await runtime.connect()
    h.receive({ type: 'phase', phase: 'running' })
    h.receive({ type: 'error', message: 'The provider failed', code: 'provider_error' })
    h.receive({ type: 'phase', phase: 'idle' })
    expect(current.lastError).toBe('The provider failed')
    // Another tab sends: this tab hears only the session start the next action.
    h.receive({ type: 'phase', phase: 'running' })
    expect(current.lastError).toBeNull()
    expect(h.sent.map((frame) => frame.type)).toEqual(['open'])
  } finally {
    runtime.dispose()
  }
})

test('a connection that cannot be made is tried again after the page\'s waits, never told as a failure', async () => {
  const h = clientHarness()
  const current = state()
  let connects = 0
  const runtime = new ConversationRuntime({
    state: current,
    connect: async () => {
      connects += 1
      if (connects < 3) {
        throw new ConversationSocketError('Agent socket failed to connect')
      }
      return h.client
    },
  })
  jest.useFakeTimers()
  // The random part at its most halves each wait: half a second, then a second.
  const random = spyOn(Math, 'random').mockReturnValue(1)
  try {
    const opening = runtime.connect()
    await turn()
    expect(connects).toBe(1)
    expect(current.load).toBe('reconnecting')
    expect(current.lastError).toBeNull()
    jest.advanceTimersByTime(499)
    await turn()
    expect(connects).toBe(1)
    jest.advanceTimersByTime(1)
    await turn()
    expect(connects).toBe(2)
    jest.advanceTimersByTime(999)
    await turn()
    expect(connects).toBe(2)
    jest.advanceTimersByTime(1)
    await opening
    expect(connects).toBe(3)
    expect(current.load).toBe('ready')
    expect(current.lastError).toBeNull()
  } finally {
    random.mockRestore()
    jest.useRealTimers()
    runtime.dispose()
  }
})

test('a connection lost before the session answered opens the conversation again after the first wait, never told as a failure', async () => {
  const sockets = playSockets()
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    connect: (signal) => connectConversationClient('ws://fixture', signal),
  })
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    const opening = runtime.connect()
    // The backend restarts while the conversation opens: the socket closes
    // after the page sent `open` and before the session answered it.
    const lost = sockets.last()
    lost.open()
    await turn()
    expect(lost.sent).toEqual([{ type: 'open' }])
    lost.end()
    await turn()
    expect(current.load).toBe('reconnecting')
    expect(current.lastError).toBeNull()
    jest.advanceTimersByTime(1_000)
    await turn()
    const next = sockets.last()
    expect(next).not.toBe(lost)
    next.open()
    await turn()
    next.receive({ type: 'opened' })
    await opening
    expect(current.load).toBe('ready')
    expect(current.lastError).toBeNull()
  } finally {
    random.mockRestore()
    jest.useRealTimers()
    runtime.dispose()
    sockets.restore()
  }
})

test('a page back from sleep breaks a conversation socket silent past the watch and opens the conversation again at once', async () => {
  const sockets = playSockets()
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    connect: (signal) => connectConversationClient('ws://fixture', signal),
  })
  jest.useFakeTimers()
  try {
    const opening = runtime.connect()
    const slept = sockets.last()
    slept.open()
    await turn()
    slept.receive({ type: 'opened' })
    await opening
    // The laptop sleeps for 80 seconds: the clock goes on, the watch's timer
    // does not, and no close reaches the page.
    jest.setSystemTime(Date.now() + 80_000)
    pageReturned()
    expect(slept.closed).toBe(true)
    // The conversation connects again without waiting.
    const next = sockets.last()
    expect(next).not.toBe(slept)
    expect(current.load).toBe('reconnecting')
    next.open()
    await turn()
    next.receive({ type: 'opened' })
    await turn()
    expect(current.load).toBe('ready')
  } finally {
    jest.useRealTimers()
    runtime.dispose()
    sockets.restore()
  }
})

test('a page back from sleep breaks a conversation socket that still waits for its session to open, and opens the conversation again at once', async () => {
  const sockets = playSockets()
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    connect: (signal) => connectConversationClient('ws://fixture', signal),
  })
  jest.useFakeTimers()
  try {
    const opening = runtime.connect()
    const slept = sockets.last()
    slept.open()
    await turn()
    expect(slept.sent).toEqual([{ type: 'open' }])
    // The laptop sleeps for 80 seconds before the session answers `open`.
    jest.setSystemTime(Date.now() + 80_000)
    pageReturned()
    expect(slept.closed).toBe(true)
    // The opening learns of the break through its promises, then waits no
    // reconnect wait: the zero-delay timer the return left is due now.
    await turn()
    expect(current.load).toBe('reconnecting')
    jest.advanceTimersByTime(0)
    await turn()
    const next = sockets.last()
    expect(next).not.toBe(slept)
    next.open()
    await turn()
    next.receive({ type: 'opened' })
    await opening
    expect(current.load).toBe('ready')
  } finally {
    jest.useRealTimers()
    runtime.dispose()
    sockets.restore()
  }
})

test('a session that refuses to open is a failure told once', async () => {
  const current = state()
  const refusal = 'Choose a model for the conversation before opening it'
  let connects = 0
  const runtime = new ConversationRuntime({
    state: current,
    connect: async () => {
      connects += 1
      return clientHarness({ type: 'error', message: refusal, code: 'model_not_selected' }).client
    },
  })
  await expect(runtime.connect()).rejects.toThrow(refusal)
  expect(current.load).toBe('failed')
  expect(current.lastError).toBe(refusal)
  expect(connects).toBe(1)
  runtime.dispose()
})

test('disposing during the backoff wait ends the retries', async () => {
  let connects = 0
  const runtime = new ConversationRuntime({
    state: state(),
    connect: async () => {
      connects += 1
      throw new ConversationSocketError('Agent socket failed to connect')
    },
  })
  jest.useFakeTimers()
  try {
    const result = runtime.connect().catch((error: unknown) => error)
    // The first attempt fails, and the backoff wait begins.
    await turn()
    expect(connects).toBe(1)
    runtime.dispose()
    // Every timer still set fires now: a backoff wait that dispose left running would start another attempt.
    jest.runAllTimers()
    expect(await result).toBeInstanceOf(Error)
    await turn()
    expect(connects).toBe(1)
  } finally {
    jest.useRealTimers()
  }
})

test('a resume is pending from the request until the next phase event', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  const resumed = runtime.resume()
  expect(s.pendingAction).toBe('resume')
  await waitFor(() => h.sent.at(-1)?.type === 'resume')
  h.receive({ type: 'phase', phase: 'running' })
  expect(s.pendingAction).toBeNull()
  h.receive({ type: 'phase', phase: 'idle' })
  await resumed
  expect(s.pendingAction).toBeNull()
})

test('the agent\'s own retries change nothing the page shows: the row keeps saying Requesting', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  h.receive({ type: 'phase', phase: 'running' })
  const before = structuredClone(s)
  h.receive({ type: 'retry_scheduled', attempt: 1, delayMs: 1000, code: 'overloaded' })
  expect(s).toEqual(before)
  runtime.dispose()
})

test('failure facts arrive beside the transcript, accumulate across patches, and start over on a reset', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  const lifts = { retryAt: '2026-09-22T07:37:39.000Z' }
  h.receive({ type: 'transcript_reset', version: { epoch: 'epoch', revision: 1 }, blocks: [], failures: { first: lifts } })
  expect(s.failures).toEqual({ first: lifts })
  h.receive({ type: 'transcript_patch', revision: 2, patches: [], failures: { second: { retryAt: null } } })
  expect(s.failures).toEqual({ first: lifts, second: { retryAt: null } })
  h.receive({ type: 'transcript_patch', revision: 3, patches: [] })
  expect(s.failures).toEqual({ first: lifts, second: { retryAt: null } })
  h.receive({ type: 'transcript_reset', version: { epoch: 'epoch', revision: 4 }, blocks: [] })
  expect(s.failures).toEqual({})
  runtime.dispose()
})

test('a closed connection ends a pending resume', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  void runtime.resume().catch(() => {})
  expect(s.pendingAction).toBe('resume')
  h.receive({ type: 'closed' })
  expect(s.pendingAction).toBeNull()
  runtime.dispose()
})

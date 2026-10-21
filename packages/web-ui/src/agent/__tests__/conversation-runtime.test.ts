import { beforeEach, expect, jest, spyOn, test } from 'bun:test'
import type { AgentEntry, PendingSteer } from '@demicodes/protocol'
import { deferred, delay, waitFor } from '@demicodes/utils'
import { computed } from 'vue'
import { ConversationRuntime, RemovalRefusedError, type RuntimeState } from '../conversation-runtime'
import { ConversationSocketError, connectConversationClient } from '../../transport/conversation-socket'
import { pageReturned } from '../../transport/liveness'
import { playSockets } from '../../transport/__tests__/test-socket'
import { ConversationClient, EMPTY_TRANSCRIPT } from '@demicodes/conversation-client'
import { agentsFrame, clientHarness, createdAt, model, rootEntry, subagentEntry, userBlock } from './agent-harness'

/** The root's entry with its session in `phase`, as the whole agents. */
/** The root's entry as the test's server last told it. */
let root: AgentEntry = rootEntry('conversation')

beforeEach(() => {
  root = rootEntry('conversation')
})

/** The agents as the server tells them once the root's session is in `phase`. */
function phase(phase: RuntimeState['phase']) {
  root = { ...root, phase }
  return agentsFrame([root])
}

/** The agents as the server tells them once the root's queue or pending steers are `waiting`. */
function waiting(waiting: Partial<Pick<AgentEntry, 'queue' | 'pendingSteers'>>) {
  root = { ...root, ...waiting }
  return agentsFrame([root])
}

function state(): RuntimeState {
  return {
    id: 'conversation',
    history: EMPTY_TRANSCRIPT,
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    starting: null,
    pendingCalls: [],
    waitingSince: null, requestOutput: null,
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
  h.receive({ type: 'transcript_reset', node: 'conversation', start: 0, length: 0, version: { epoch: 'epoch', revision: 1 }, blocks: [] })
  expect(version.value).toEqual({ epoch: 'epoch', revision: 1 })
  h.receive({ type: 'transcript_patch', node: 'conversation', revision: 2, patches: [{ op: 'truncate', length: 0 }] })
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
    // A response before the compaction still measures the history the summary
    // replaced.
    h.receive({ type: 'transcript_reset', node: 'conversation', start: 0, length: 0, version: { epoch: 'epoch', revision: 1 }, blocks: [] })
    const usage = { tokens: 6_000, window: 100_000, compactFrom: 50_000, rows: [], account: null }
    h.receive(agentsFrame([rootEntry('conversation', 'idle', { context: usage })]))
    expect(current.contextUsage).toEqual(usage)
    // A subagent's usage is its record's, never the composer's.
    h.receive({ type: 'agent', revision: 2, agent: subagentEntry('child', 'conversation', 'running', { context: { ...usage, tokens: 70_000 } }) })
    expect(current.contextUsage?.tokens).toBe(6_000)
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
      node: 'conversation',
      start: 0,
      length: 1,
      version: { epoch: 'epoch', revision: 1 },
      blocks: failure === 'before-reconciliation' ? [] : [userBlock('target', 'old-turn', 'old')],
    })
    current.lastError = { message: failure === 'before-reconciliation' ? 'generation failed' : 'old turn failed', requestId: null }
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
        // The failure kept: the page's headline for the one the socket
        // reported, whose code names no headline of its own.
        const message = failure === 'before-confirmation' ? 'The turn failed.' : 'generation failed'
        expect({ ...current.lastError }).toEqual({ message, requestId: null })
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
  expect(h.sent.map((frame) => frame.type)).toEqual(['subscribe', 'open'])
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
      node: 'conversation',
      start: 0,
      length: 1,
      version: { epoch: 'runtime-test', revision: 1 },
      blocks: [userBlock('user-block', 'persisted-message', 'Already accepted')],
    })
    await runtime.submit([{ type: 'text', text: 'Already accepted' }], 'persisted-message')
    expect(h.sent.map((frame) => frame.type)).toEqual(['subscribe', 'open'])
  } finally {
    runtime.dispose()
  }
})

test('send now on a pending steer or a queued message stops nothing and never turns the message into a steer', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
  try {
    await runtime.connect()
    const steer: PendingSteer = { id: 'steer', turnId: 'turn', model, content: [{ type: 'text', text: 'now' }] }
    h.receive(phase('running'))
    h.receive(waiting({ pendingSteers: [steer] }))
    h.receive(waiting({ queue: [{ id: 'queued', content: [{ type: 'text', text: 'next' }] }] }))
    await runtime.interruptPendingSteer('steer')
    await runtime.sendQueuedNow('queued')
    // The session cuts the running round short itself: no Stop, no Continue.
    expect(h.sent).toEqual([
      { type: 'subscribe', node: 'conversation' },
      { type: 'open' },
      { type: 'steer_now', steerId: 'steer' },
      { type: 'send_queued_message', messageId: 'queued' },
    ])
  } finally {
    runtime.dispose()
  }
})

// Remove on a steer or a queued message. The session answers whether it removed
// the message or the message had gone on. The removal settles on that answer
// alone, whatever frames come before it.
test.each([
  { kind: 'steer', outcome: 'removed' },
  { kind: 'steer', outcome: 'gone_on' },
  { kind: 'queued', outcome: 'removed' },
  { kind: 'queued', outcome: 'gone_on' },
] as const)('removing a $kind message settles on the session\'s answer: $outcome', async ({ kind, outcome }) => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
  const content = [{ type: 'text' as const, text: 'skip the e2e tests' }]
  try {
    await runtime.connect()
    h.receive(phase('running'))
    h.receive(waiting({ pendingSteers: kind === 'steer' ? [{ id: 'm', turnId: 'turn-1', model, content }] : [] }))
    h.receive(waiting({ queue: kind === 'queued' ? [{ id: 'm', content }] : [] }))
    let settled = false
    const removal = runtime.removeWaiting('m').then(() => 'removed', (error: unknown) => error).finally(() => {
      settled = true
    })
    await waitFor(() => h.sent.length > 2)
    expect(h.sent.at(-1)).toEqual(kind === 'steer' ? { type: 'cancel_pending_steer', steerId: 'm' } : { type: 'dequeue_message', messageId: 'm' })
    // The list's frame comes first, and the agent going idle tells nothing.
    h.receive(waiting(kind === 'steer' ? { pendingSteers: [] } : { queue: [] }))
    h.receive(phase('idle'))
    await turn()
    expect(settled).toBe(false)
    h.receive({ type: 'remove_result', id: 'm', outcome, at: null })
    const result = await removal
    if (outcome === 'removed') {
      expect(result).toBe('removed')
    } else {
      expect(result).toBeInstanceOf(RemovalRefusedError)
    }
  } finally {
    runtime.dispose()
  }
})

for (const fate of ['removed', 'gone on', 'still listed'] as const) {
  test(`a removal whose socket is lost before the answer settles from the reopened session: ${fate}`, async () => {
    const sockets = playSockets()
    const current = state()
    const runtime = new ConversationRuntime({
      state: current,
      connect: (signal) => connectConversationClient('ws://fixture', signal),
    })
    jest.useFakeTimers()
    const random = spyOn(Math, 'random').mockReturnValue(0)
    const content = [{ type: 'text' as const, text: 'skip the e2e tests' }]
    const steer: PendingSteer = { id: 's', turnId: 'turn', model, content }
    try {
      const opening = runtime.connect()
      const lost = sockets.last()
      lost.open()
      await turn()
      lost.receive({ type: 'opened' })
      await opening
      lost.receive(phase('running'))
      lost.receive(waiting({ pendingSteers: [steer] }))
      const removal = runtime.removeWaiting('s').then(() => 'removed', (error: unknown) => error)
      await turn()
      expect(lost.sent.at(-1)).toEqual({ type: 'cancel_pending_steer', steerId: 's' })
      lost.end()
      await turn()
      jest.advanceTimersByTime(1000)
      await turn()
      const next = sockets.last()
      next.open()
      await turn()
      next.receive({ type: 'opened' })
      await turn()
      const written = { type: 'steer' as const, id: 's', turnId: 'turn', createdAt, model, content }
      next.receive({ type: 'transcript_reset', node: 'conversation', start: 0, length: fate === 'gone on' ? 1 : 0, version: { epoch: 'epoch', revision: 1 }, blocks: fate === 'gone on' ? [written] : [] })
      next.receive(phase('running'))
      next.receive(waiting({ queue: [] }))
      next.receive(waiting({ pendingSteers: fate === 'still listed' ? [steer] : [] }))
      await turn()
      if (fate === 'still listed') {
        // Asked again on the new connection, and settled on its answer.
        expect(next.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }, { type: 'cancel_pending_steer', steerId: 's' }])
        next.receive(waiting({ pendingSteers: [] }))
        next.receive({ type: 'remove_result', id: 's', outcome: 'removed', at: null })
      } else {
        expect(next.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }])
      }
      const result = await removal
      if (fate === 'gone on') {
        expect(result).toBeInstanceOf(RemovalRefusedError)
      } else {
        expect(result).toBe('removed')
      }
    } finally {
      random.mockRestore()
      jest.useRealTimers()
      runtime.dispose()
      sockets.restore()
    }
  })
}

test('a steer the session accepts waits as a pending steer, and one it refuses, as an ending turn does, goes as a message with its id and no error', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({ state: current, connect: async () => h.client })
  const content = [{ type: 'text' as const, text: 'skip the e2e tests' }]
  try {
    await runtime.connect()
    h.receive(phase('running'))
    const accepted = runtime.steer(content, 'first')
    await waitFor(() => h.sent.at(-1)?.type === 'steer')
    expect(h.sent.at(-1)).toEqual({ type: 'steer', steerId: 'first', content })
    h.receive({ type: 'steer_result', steerId: 'first', outcome: { status: 'accepted', at: null } })
    await accepted
    // The turn is ending: the session refuses the next steer.
    const refused = runtime.steer(content, 'second')
    await waitFor(() => h.sent.filter((frame) => frame.type === 'steer').length === 2)
    h.receive({ type: 'steer_result', steerId: 'second', outcome: { status: 'rejected', code: 'turn_finishing', reason: 'The running turn is finishing' } })
    await waitFor(() => h.sent.at(-1)?.type === 'send')
    expect(h.sent.at(-1)).toEqual({ type: 'send', messageId: 'second', content })
    h.receive(waiting({ queue: [{ id: 'second', content }] }))
    h.receive({ type: 'send_result', messageId: 'second', at: null })
    await refused
    expect(h.sent.map((frame) => frame.type)).toEqual(['subscribe', 'open', 'steer', 'steer', 'send'])
    expect(current.lastError).toBeNull()
  } finally {
    runtime.dispose()
  }
})

for (const held of [true, false]) {
  test(`a steer whose socket is lost before the session answered is ${held ? 'not sent again when the reopened session lists it' : 'sent as a message when the reopened session holds it nowhere'}`, async () => {
    const sockets = playSockets()
    const current = state()
    const runtime = new ConversationRuntime({
      state: current,
      connect: (signal) => connectConversationClient('ws://fixture', signal),
    })
    jest.useFakeTimers()
    const random = spyOn(Math, 'random').mockReturnValue(0)
    const content = [{ type: 'text' as const, text: 'skip the e2e tests' }]
    const steer: PendingSteer = { id: 'steer', turnId: 'turn', model, content }
    try {
      const opening = runtime.connect()
      const lost = sockets.last()
      lost.open()
      await turn()
      lost.receive({ type: 'opened' })
      await opening
      lost.receive(phase('running'))
      lost.receive(waiting({ pendingSteers: [] }))
      let settled = false
      const steering = runtime.steer(content, 'steer').finally(() => {
        settled = true
      })
      await turn()
      expect(lost.sent.at(-1)).toEqual({ type: 'steer', steerId: 'steer', content })
      // The network drops the socket before the session's answer arrives.
      lost.end()
      await turn()
      expect(settled).toBe(false)
      jest.advanceTimersByTime(1000)
      await turn()
      const next = sockets.last()
      expect(next).not.toBe(lost)
      next.open()
      await turn()
      next.receive({ type: 'opened' })
      await turn()
      // Nothing goes before the session has shown what it holds.
      expect(next.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }])
      next.receive({ type: 'transcript_reset', node: 'conversation', start: 0, length: 0, version: { epoch: 'epoch', revision: 1 }, blocks: [] })
      next.receive(phase('running'))
      next.receive(waiting({ queue: [] }))
      next.receive(waiting({ pendingSteers: held ? [steer] : [] }))
      await turn()
      if (held) {
        await steering
        expect(next.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }])
      } else {
        expect(next.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }, { type: 'send', messageId: 'steer', content }])
        next.receive(waiting({ queue: [{ id: 'steer', content }] }))
        next.receive({ type: 'send_result', messageId: 'steer', at: null })
        await steering
      }
      expect(current.lastError).toBeNull()
    } finally {
      random.mockRestore()
      jest.useRealTimers()
      runtime.dispose()
      sockets.restore()
    }
  })
}

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
    expect(harnesses[1]!.sent.map((frame) => frame.type)).toEqual(['subscribe', 'open'])
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
    h.receive(phase('running'))
    h.receive({ type: 'error', message: 'The provider failed', code: 'provider_error' })
    h.receive(phase('idle'))
    expect({ ...current.lastError }).toEqual({ message: 'The turn failed.', requestId: null })
    // Another tab sends: this tab hears only the session start the next action.
    h.receive(phase('running'))
    expect(current.lastError).toBeNull()
    expect(h.sent.map((frame) => frame.type)).toEqual(['subscribe', 'open'])
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
    expect(current.load).toBe('connecting')
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
    expect(lost.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }])
    lost.end()
    await turn()
    expect(current.load).toBe('connecting')
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

// The product's stuck page: a message sent just before the socket dropped
// showed as not delivered, and each Retry failed the same way. Fake sockets
// and a fake clock: a few milliseconds.
test('a message whose socket is lost before the session confirmed it waits for the connection and is sent again with its id', async () => {
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
    const lost = sockets.last()
    lost.open()
    await turn()
    lost.receive({ type: 'opened' })
    await opening
    let settled = false
    const sending = runtime.submit([{ type: 'text', text: 'Hello' }], 'message').finally(() => {
      settled = true
    })
    await turn()
    expect(lost.sent.at(-1)).toEqual({ type: 'send', messageId: 'message', content: [{ type: 'text', text: 'Hello' }] })
    // The network drops the socket before the session's confirmation arrives.
    lost.end()
    await turn()
    expect(settled).toBe(false)
    expect(current.load).toBe('reconnecting')
    expect(current.lastError).toBeNull()
    // The send waits for the page's first wait like the connection does.
    jest.advanceTimersByTime(999)
    await turn()
    expect(sockets.last()).toBe(lost)
    jest.advanceTimersByTime(1)
    await turn()
    const next = sockets.last()
    expect(next).not.toBe(lost)
    next.open()
    await turn()
    next.receive({ type: 'opened' })
    await turn()
    expect(next.sent).toEqual([
      { type: 'subscribe', node: 'conversation' },
      { type: 'open' },
      { type: 'send', messageId: 'message', content: [{ type: 'text', text: 'Hello' }] },
    ])
    next.receive({
      type: 'transcript_reset',
      node: 'conversation',
      start: 0,
      length: 1,
      version: { epoch: 'epoch', revision: 1 },
      blocks: [userBlock('block', 'message', 'Hello')],
    })
    next.receive({ type: 'send_result', messageId: 'message', at: null })
    await sending
    expect(current.load).toBe('ready')
    expect(current.lastError).toBeNull()
  } finally {
    random.mockRestore()
    jest.useRealTimers()
    runtime.dispose()
    sockets.restore()
  }
})

test('a saved message whose socket is lost before the session took it goes again as the same turn once the conversation is open, unless the session holds it', async () => {
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
    const lost = sockets.last()
    lost.open()
    await turn()
    lost.receive({ type: 'opened' })
    await opening
    const sending = runtime.sendSaved('saved', 'turn', 'message')
    await turn()
    expect(lost.sent.at(-1)).toEqual({ type: 'send_saved', id: 'saved', turnId: 'turn', delivery: 'message' })
    lost.end()
    await turn()
    expect(current.lastError).toBeNull()
    jest.advanceTimersByTime(1000)
    await turn()
    const next = sockets.last()
    next.open()
    await turn()
    next.receive({ type: 'opened' })
    await turn()
    expect(next.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }, { type: 'send_saved', id: 'saved', turnId: 'turn', delivery: 'message' }])
    next.receive({
      type: 'transcript_reset',
      node: 'conversation',
      start: 0,
      length: 1,
      version: { epoch: 'epoch', revision: 1 },
      blocks: [userBlock('block', 'turn', 'Check the DNS')],
    })
    next.receive({ type: 'send_result', messageId: 'turn', at: null })
    expect(await sending).toBe(true)

    // The session holds it already, as when the first send reached it: nothing
    // goes again.
    const held = sockets.last()
    const sent = held.sent.length
    expect(await runtime.sendSaved('saved', 'turn', 'message')).toBe(true)
    expect(held.sent).toHaveLength(sent)
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
    jest.advanceTimersByTime(0)
    expect(slept.closed).toBe(true)
    // The conversation connects again without waiting: no timer runs before it.
    await turn()
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
    expect(slept.sent).toEqual([{ type: 'subscribe', node: 'conversation' }, { type: 'open' }])
    // The laptop sleeps for 80 seconds before the session answers `open`.
    jest.setSystemTime(Date.now() + 80_000)
    pageReturned()
    jest.advanceTimersByTime(0)
    expect(slept.closed).toBe(true)
    // The opening learns of the break through its promises, then waits no
    // reconnect wait: the zero-delay timer the return left is due now.
    await turn()
    expect(current.load).toBe('connecting')
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

// The frames of the handshake wait until the session has opened, then go. The
// subscription that held them lives as long as the connection, and once held a
// transcript and every command's output for that long. Cost: one opening and
// two collections.
test('the handshake\'s frames are applied once the session opens and are not kept after', async () => {
  const current = state()
  let deliver: (frame: unknown) => void = () => {}
  const client = new ConversationClient({
    send(frame) {
      if (frame.type === 'open') {
        deliver({ type: 'opened' })
        deliver(agentsFrame([rootEntry('conversation')]))
      }
    },
    onFrame(handler) {
      deliver = handler
      return () => {}
    },
    onClose: () => () => {},
    close() {},
  })
  let frame: WeakRef<object> | undefined
  client.subscribe((event) => {
    if (event.type === 'queue') {
      frame = new WeakRef(event)
    }
  })
  const runtime = new ConversationRuntime({ state: current, connect: async () => client })
  try {
    await runtime.connect()
    expect(current.load).toBe('ready')
    expect(frame).toBeDefined()
    await turn()
    Bun.gc(true)
    await turn()
    Bun.gc(true)
    expect(frame!.deref()).toBeUndefined()
  } finally {
    runtime.dispose()
  }
})

// A page that hides a child while its open's handshake is on its way lets go of
// the child's transcript (`crates/agent-server/src/server/tree.rs`). The reset
// the handshake held for it revived it, stale from then on. Cost: one opening.
test('a child the page hides while the handshake arrives takes nothing of it', async () => {
  let shown = ['child']
  let deliver: (frame: unknown) => void = () => {}
  const sent: string[] = []
  const reset = (node: string) => ({ type: 'transcript_reset', node, start: 0, length: 0, blocks: [], version: { epoch: 'e', revision: 1 } })
  const client = new ConversationClient({
    send(frame) {
      sent.push(frame.type === 'subscribe' || frame.type === 'unsubscribe' ? `${frame.type} ${frame.node}` : frame.type)
      if (frame.type === 'open') {
        deliver({ type: 'opened' })
        deliver(reset('conversation'))
        deliver(reset('child'))
        // The user closes the Agents window before the session has opened.
        shown = []
      }
    },
    onFrame(handler) {
      deliver = handler
      return () => {}
    },
    onClose: () => () => {},
    close() {},
  })
  const applied: string[] = []
  const runtime = new ConversationRuntime({
    state: state(),
    connect: async () => client,
    nodes: () => shown,
    onEvent: (event) => {
      if (event.type === 'transcript_reset') {
        applied.push(event.node)
      }
    },
  })
  try {
    await runtime.connect()

    expect(sent).toEqual(['subscribe conversation', 'subscribe child', 'open', 'unsubscribe child'])
    expect(applied).toEqual(['conversation'])
  } finally {
    runtime.dispose()
  }
})

test('a session that refuses to open is a failure told once', async () => {
  const current = state()
  // A record that holds no model opens as ready, not failed: the refusal
  // here is another one.
  const refusal = 'The provider is no longer available'
  let connects = 0
  const runtime = new ConversationRuntime({
    state: current,
    connect: async () => {
      connects += 1
      return clientHarness({ type: 'error', message: refusal, code: 'provider_not_found', requestId: 'req_socket0000000001' }).client
    },
  })
  await expect(runtime.connect()).rejects.toThrow(refusal)
  expect(current.load).toBe('failed')
  // In the page's words for its code, with the socket's request.
  expect({ ...current.lastError }).toEqual({ message: 'The provider wasn’t found.', requestId: 'req_socket0000000001' })
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
    // Every timer still set fires now: a backoff wait that dispose left running
    // would start another attempt.
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
  h.receive(phase('running'))
  expect(s.pendingAction).toBeNull()
  h.receive(phase('idle'))
  await resumed
  expect(s.pendingAction).toBeNull()
})

test('the agent\'s own retries change nothing the page shows: the row keeps saying Requesting', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  h.receive(phase('running'))
  const before = structuredClone(s)
  h.receive({ type: 'retry_scheduled', attempt: 1, delayMs: 1000, code: 'overloaded' })
  expect(s).toEqual(before)
  runtime.dispose()
})

test('failure facts arrive beside the transcript and accumulate across patches and resets', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  const lifts = { retryAt: '2026-09-22T07:37:39.000Z' }
  h.receive({ type: 'transcript_reset', node: 'conversation', start: 0, length: 0, version: { epoch: 'epoch', revision: 1 }, blocks: [], failures: { first: lifts } })
  expect(s.failures).toEqual({ first: lifts })
  h.receive({ type: 'transcript_patch', node: 'conversation', revision: 2, patches: [], failures: { second: { retryAt: null } } })
  expect(s.failures).toEqual({ first: lifts, second: { retryAt: null } })
  h.receive({ type: 'transcript_patch', node: 'conversation', revision: 3, patches: [] })
  expect(s.failures).toEqual({ first: lifts, second: { retryAt: null } })
  // A reset brings the blocks from its start; the page may hold the blocks
  // before it, with their facts.
  h.receive({ type: 'transcript_reset', node: 'conversation', start: 0, length: 0, version: { epoch: 'epoch', revision: 4 }, blocks: [] })
  expect(s.failures).toEqual({ first: lifts, second: { retryAt: null } })
  runtime.dispose()
})

// A Resume another page started first is no failure: the session refuses
// this one as busy running the turn, which runs on. Busy with another page's
// Compact, which may end without running the turn, the refusal is a failure, as
// is a refusal of another kind.
test.each([
  { busy: 'running', failed: false },
  { busy: 'compacting', failed: true },
] as const)('a Resume the session refuses as busy $busy fails: $failed', async ({ busy, failed }) => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  const resumed = runtime.resume().then(() => null, (error: unknown) => error)
  await waitFor(() => h.sent.at(-1)?.type === 'resume')
  h.receive(phase(busy))
  h.receive({ type: 'rejected', command: 'resume', code: 'session_busy', params: {}, reason: `Session is busy (${busy})` })
  expect(await resumed).toEqual(failed ? expect.objectContaining({ code: 'session_busy' }) : null)
  expect(s.pendingAction).toBeNull()
  runtime.dispose()
})

test('a Resume the session refuses as closed fails', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, connect: async () => h.client })
  await runtime.connect()
  const refused = runtime.resume().then(() => null, (error: unknown) => error)
  await waitFor(() => h.sent.at(-1)?.type === 'resume')
  h.receive({ type: 'rejected', command: 'resume', code: 'session_closed', params: {}, reason: 'The agent session is closed' })
  expect(await refused).toMatchObject({ code: 'session_closed' })
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

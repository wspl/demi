import { expect, test } from 'bun:test'
import type { ModelSelection, PendingSteer } from '@demicodes/protocol'
import { deferred, delay, waitFor } from '@demicodes/utils'
import { computed } from 'vue'
import { ConversationRuntime, type RuntimeState } from '../conversation-runtime'
import { AgentSocketError } from '../../transport/agent-socket'
import { clientHarness, model, userBlock } from './agent-harness'

function state(): RuntimeState {
  return {
    blocks: [],
    phase: 'idle',
    queue: [],
    pendingSteers: [],
    model: {
      providerId: 'stub',
      modelId: 'stub',
      thinkingEffort: null,
      serviceTierId: null,
    },
    lastError: null,
    load: 'loading',
    pendingAction: null,
    failures: {},
  }
}

test('the current edit version reacts to connection, snapshots, patches and disconnect', async () => {
  const h = clientHarness()
  const runtime = new ConversationRuntime({ state: state(), prepareModel: async () => model, connect: async () => h.client })
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

for (const failure of ['none', 'before-confirmation', 'before-reconciliation'] as const) {
  test(`edit confirmation preserves generation recovery: ${failure}`, async () => {
    const h = clientHarness()
    const current = state()
    const runtime = new ConversationRuntime({ state: current, prepareModel: async () => model, connect: async () => h.client })
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

test('disposing a view during model preparation prevents a late connection', async () => {
  const prepared = deferred<ModelSelection>()
  let connects = 0
  const runtime = new ConversationRuntime({
    state: state(),
    prepareModel: () => prepared.promise,
    connect: async () => {
      connects += 1
      return clientHarness().client
    },
  })
  const opening = runtime.connect()
  const result = opening.catch((error) => error)
  runtime.dispose()
  prepared.resolve(model)
  expect(await result).toBeInstanceOf(Error)
  expect(connects).toBe(0)
})

test('disposing an open view detaches without sending task-close or abort commands', async () => {
  const h = clientHarness()
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    prepareModel: async () => model,
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
    prepareModel: async () => model,
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
  const runtime = new ConversationRuntime({ state: current, prepareModel: async () => model, connect: async () => h.client })
  try {
    await runtime.connect()
    const steer: PendingSteer = { id: 'steer', turnId: 'turn', model, content: [{ type: 'text', text: 'now' }] }
    h.receive({ type: 'phase', phase: 'running' })
    h.receive({ type: 'pending_steers', pendingSteers: [steer] })
    const delivered = runtime.interruptPendingSteer('steer')
    await waitFor(() => h.sent.some((frame) => frame.type === 'abort'))
    expect(h.sent.some((frame) => frame.type === 'resume')).toBe(false)
    h.receive({ type: 'phase', phase: 'idle' })
    h.receive({ type: 'abort_result', result: { target: 'active_turn', canAbortAgain: false } })
    await waitFor(() => h.sent.some((frame) => frame.type === 'resume'))
    h.receive({ type: 'phase', phase: 'running' })
    h.receive({ type: 'phase', phase: 'idle' })
    await delivered
    expect(h.sent.map((frame) => frame.type)).toEqual(['open', 'abort', 'resume'])
  } finally {
    runtime.dispose()
  }
})

test('server takeover does not start a reconnect fight between views', async () => {
  const h = clientHarness()
  let connects = 0
  const runtime = new ConversationRuntime({
    state: state(),
    prepareModel: async () => model,
    connect: async () => {
      connects += 1
      return h.client
    },
  })
  await runtime.connect()
  h.receive({ type: 'closed' })
  await delay(1100)
  expect(connects).toBe(1)
  expect(runtime.connected).toBe(false)
  runtime.dispose()
})

test('a connection that cannot be made is retried with backoff, never told as a failure', async () => {
  const h = clientHarness()
  const current = state()
  let connects = 0
  const runtime = new ConversationRuntime({
    state: current,
    prepareModel: async () => model,
    connect: async () => {
      connects += 1
      if (connects < 3) {
        throw new AgentSocketError('Agent socket failed to connect')
      }
      return h.client
    },
    reconnect: { baseMs: 1, maxMs: 4 },
  })
  try {
    const opening = runtime.connect()
    await waitFor(() => connects === 2)
    expect(current.load).toBe('reconnecting')
    expect(current.lastError).toBeNull()
    await opening
    expect(connects).toBe(3)
    expect(current.load).toBe('ready')
    expect(current.lastError).toBeNull()
  } finally {
    runtime.dispose()
  }
})

test('a session that refuses to open is a failure told once', async () => {
  const current = state()
  const runtime = new ConversationRuntime({
    state: current,
    prepareModel: async () => {
      throw new Error('No provider is configured')
    },
    connect: async () => clientHarness().client,
    reconnect: { baseMs: 1, maxMs: 4 },
  })
  await expect(runtime.connect()).rejects.toThrow('No provider is configured')
  expect(current.load).toBe('failed')
  expect(current.lastError).toBe('No provider is configured')
  runtime.dispose()
})

test('disposing during the backoff wait ends the retries', async () => {
  let connects = 0
  const runtime = new ConversationRuntime({
    state: state(),
    prepareModel: async () => model,
    connect: async () => {
      connects += 1
      throw new AgentSocketError('Agent socket failed to connect')
    },
    reconnect: { baseMs: 50, maxMs: 50 },
  })
  const result = runtime.connect().catch((error) => error)
  await waitFor(() => connects === 1)
  runtime.dispose()
  expect(await result).toBeInstanceOf(Error)
  await delay(120)
  expect(connects).toBe(1)
})

test('a resume is pending from the request until the next phase event', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, prepareModel: async () => model, connect: async () => h.client })
  await runtime.connect()
  const resumed = runtime.resume()
  expect(s.pendingAction).toBe('resume')
  await delay(0)
  expect(h.sent.at(-1)?.type).toBe('resume')
  h.receive({ type: 'phase', phase: 'running' })
  expect(s.pendingAction).toBeNull()
  h.receive({ type: 'phase', phase: 'idle' })
  await resumed
  expect(s.pendingAction).toBeNull()
})

test('the agent\'s own retries change nothing the page shows: the row keeps saying Requesting', async () => {
  const h = clientHarness()
  const s = state()
  const runtime = new ConversationRuntime({ state: s, prepareModel: async () => model, connect: async () => h.client })
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
  const runtime = new ConversationRuntime({ state: s, prepareModel: async () => model, connect: async () => h.client })
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
  const runtime = new ConversationRuntime({ state: s, prepareModel: async () => model, connect: async () => h.client })
  await runtime.connect()
  void runtime.resume().catch(() => {})
  expect(s.pendingAction).toBe('resume')
  h.receive({ type: 'closed' })
  expect(s.pendingAction).toBeNull()
  runtime.dispose()
})

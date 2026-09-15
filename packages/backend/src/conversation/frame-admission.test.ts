import { expect, test, vi } from 'bun:test'
import { createInProcessTransportPair, serverFrameSchema, type ServerFrame } from '@demicodes/agent'
import { ActivityGate, deferred, waitFor } from '@demicodes/utils'
import { admitConversationFrame } from './frame-admission'
import { conversationScopedTransport } from './scoped-transport'
import { LocalControlService } from '../storage/control'
import { openSqliteDatabase } from '../storage/database'
import { CONTROL_MIGRATIONS, migrate } from '../storage/migrations'

test('frame admission waits outside an idle reservation, then holds a demand lease', async () => {
  const gate = new ActivityGate()
  const retire = gate.tryReserve('idle')!
  let admitted = false
  const entering = admitConversationFrame(gate, new AbortController().signal).then(release => {
    admitted = true
    return release
  })
  await Promise.resolve()
  expect(admitted).toBe(false)
  expect(gate.active).toBe(false)
  retire()
  const release = await entering
  expect(release).not.toBeNull()
  expect(gate.demandActive).toBe(true)
  release?.()
  expect(gate.active).toBe(false)
})

test('frame admission is refused during reset without waiting for its reservation', async () => {
  const gate = new ActivityGate()
  const reset = await gate.reserve('forced')
  try {
    expect(await admitConversationFrame(gate, new AbortController().signal)).toBeNull()
    expect(gate.holdsReservation(reset)).toBe(true)
    expect(gate.active).toBe(false)
  } finally {
    reset()
  }
})

test('a forced transition replacing idle retirement refuses the waiting frame', async () => {
  const gate = new ActivityGate()
  const retire = gate.tryReserve('idle')!
  const entering = admitConversationFrame(gate, new AbortController().signal)
  retire()
  const reset = gate.tryReserve('forced')!
  try {
    expect(await entering).toBeNull()
    expect(gate.active).toBe(false)
  } finally {
    reset()
  }
})

test('canceling an idle waiter leaves retirement owned and never admits the old frame', async () => {
  const gate = new ActivityGate()
  const retire = gate.tryReserve('idle')!
  const canceled = new AbortController()
  const entering = admitConversationFrame(gate, canceled.signal)
  canceled.abort()
  await expect(entering).rejects.toThrow('Aborted')
  expect(gate.holdsReservation(retire)).toBe(true)
  retire()
  await Promise.resolve()
  expect(gate.active).toBe(false)
  await expect(admitConversationFrame(gate, canceled.signal)).rejects.toThrow('Aborted')
  expect(gate.active).toBe(false)
})

test('idle admission is bounded and removes its timer on timeout and success', async () => {
  vi.useFakeTimers()
  const gate = new ActivityGate()
  const retire = gate.tryReserve('idle')!
  try {
    const entering = admitConversationFrame(gate, new AbortController().signal)
    expect(vi.getTimerCount()).toBe(1)
    vi.advanceTimersByTime(30_000)
    expect(await entering).toBeNull()
    expect(gate.holdsReservation(retire)).toBe(true)
    expect(gate.active).toBe(false)
    expect(vi.getTimerCount()).toBe(0)

    const next = admitConversationFrame(gate, new AbortController().signal)
    expect(vi.getTimerCount()).toBe(1)
    retire()
    const release = await next
    expect(release).not.toBeNull()
    release?.()
    expect(vi.getTimerCount()).toBe(0)
  } finally {
    retire()
    vi.useRealTimers()
  }
})

/** Exercises frame admission through the conversation transport and its real record lookup. */
async function frameFixture() {
  const db = openSqliteDatabase(':memory:')
  migrate(db, CONTROL_MIGRATIONS)
  const control = new LocalControlService(db)
  const user = await control.createUser({ email: 'idle@example.test', passwordHash: 'unused', role: 'user' })
  const conversation = await control.createConversation(user!.id)
  const gate = new ActivityGate()
  const waiting = deferred<void>()
  const settled = deferred<void>()
  const pair = createInProcessTransportPair()
  const transport = conversationScopedTransport(pair.server, conversation, {
    control,
    providerAllowed: async () => true,
    admitFrame: async signal => {
      waiting.resolve()
      try {
        return await admitConversationFrame(gate, signal)
      } finally {
        settled.resolve()
      }
    },
  })
  const received: unknown[] = []
  const replies: ServerFrame[] = []
  const unsubscribe = transport.onFrame(frame => { received.push(frame) })
  pair.client.onFrame(frame => { replies.push(serverFrameSchema.parse(frame)) })
  return {
    gate, waiting, settled, received, replies, transport, unsubscribe,
    send: () => pair.client.send({ type: 'abort' }),
    close: () => {
      transport.close()
      pair.client.close()
      db.close()
    },
  }
}

test('transport delivers a frame after idle retirement and reports conversation_busy during reset', async () => {
  const f = await frameFixture()
  try {
    const retire = f.gate.tryReserve('idle')!
    f.send()
    await f.waiting.promise
    expect(f.received).toEqual([])
    expect(f.replies).toEqual([])
    expect(f.gate.active).toBe(false)
    retire()
    await waitFor(() => f.received.length === 1 && !f.gate.active)
    expect(f.received).toEqual([{ type: 'abort' }])
    const reset = f.gate.tryReserve('forced')!
    try {
      f.send()
      await waitFor(() => f.replies.length === 1)
      expect(f.replies[0]).toMatchObject({ type: 'error', code: 'conversation_busy' })
      expect(f.received).toHaveLength(1)
    } finally {
      reset()
    }
  } finally {
    f.close()
  }
})

test.each(['close', 'unsubscribe'] as const)('transport %s cancels the idle waiter without delivering or releasing retirement', async action => {
  const f = await frameFixture()
  const retire = f.gate.tryReserve('idle')!
  try {
    f.send()
    await f.waiting.promise
    if (action === 'close')
      f.transport.close()
    else
      f.unsubscribe()
    await f.settled.promise
    expect(f.gate.holdsReservation(retire)).toBe(true)
    retire()
    await Promise.resolve()
    expect(f.received).toEqual([])
    expect(f.replies).toEqual([])
    expect(f.gate.active).toBe(false)
  } finally {
    retire()
    f.close()
  }
})

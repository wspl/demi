import { afterEach, expect, test } from 'bun:test'
import { connectControlClient, type ControlConnection } from '../control-client'

const realSocket = globalThis.WebSocket
const connections: ControlConnection[] = []
class FakeSocket extends EventTarget {
  static latest: FakeSocket
  closed = false
  sent: string[] = []
  failSend = false
  constructor(_url: string) {
    super()
    FakeSocket.latest = this
  }
  send(value: string): void {
    if (this.failSend) {
      throw new Error('send failed')
    }
    this.sent.push(value)
  }
  close(): void {
    this.closed = true
    this.dispatchEvent(new Event('close'))
  }
  receive(value: unknown): void {
    this.dispatchEvent(new MessageEvent('message', { data: value }))
  }
}

afterEach(() => {
  for (const connection of connections.splice(0)) {
    connection.close()
  }
  globalThis.WebSocket = realSocket
})

async function connect(timeoutMs = 1000, signal?: AbortSignal) {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const opening = connectControlClient('ws://fixture', { timeoutMs, signal })
  const socket = FakeSocket.latest
  socket.dispatchEvent(new Event('open'))
  const api = await opening
  connections.push(api)
  return { api, socket }
}

test('control responses validate method results and correlate failures without losing other calls', async () => {
  const { api, socket } = await connect()
  const providers = api.listProviders()
  const models = api.listModels({ providerId: 'test' }).catch((error) => error)
  socket.receive(
    JSON.stringify({ id: 2, ok: false, error: 'Model catalog unavailable' }),
  )
  socket.receive(
    JSON.stringify({
      id: 1,
      ok: true,
      result: [{ id: 'test', label: 'Test', isAvailable: true }],
    }),
  )
  expect(await providers).toEqual([
    { id: 'test', label: 'Test', isAvailable: true },
  ])
  expect((await models).message).toBe('Model catalog unavailable')
  expect(socket.closed).toBe(false)
})

test('malformed JSON, envelopes and typed results reject every pending call and close', async () => {
  for (const response of [
    '{broken',
    '[]',
    JSON.stringify({ id: '1', ok: true, result: [] }),
    JSON.stringify({
      id: 1,
      ok: true,
      result: [{ id: 'test', isAvailable: 'yes' }],
    }),
    new Uint8Array([1]),
  ]) {
    const { api, socket } = await connect()
    const pending = [api.listProviders(), api.defaultWorkspace()].map((call) =>
      call.catch((error) => error),
    )
    socket.receive(response)
    for (const result of await Promise.all(pending)) {
      expect(result.message).toBe('Invalid control response')
    }
    expect(socket.closed).toBe(true)
    await expect(api.listProviders()).rejects.toThrow('closed')
  }
})

test('a timed out call releases its waiter and a late reply does not settle the next call', async () => {
  const { api, socket } = await connect(5)
  await expect(api.defaultWorkspace()).rejects.toThrow('timed out')
  const next = api.defaultWorkspace()
  socket.receive(JSON.stringify({ id: 1, ok: true, result: { cwd: '/old' } }))
  socket.receive(
    JSON.stringify({ id: 2, ok: true, result: { cwd: '/current' } }),
  )
  expect(await next).toEqual({ cwd: '/current' })
})

test('close, abort, send errors and socket errors reject active calls', async () => {
  for (const action of ['close', 'abort', 'send', 'error']) {
    const controller = new AbortController()
    const { api, socket } = await connect(1000, controller.signal)
    socket.failSend = action === 'send'
    const pending = api.defaultWorkspace().catch((error) => error)
    if (action === 'close') {
      api.close()
    }
    if (action === 'abort') {
      controller.abort()
    }
    if (action === 'error') {
      socket.dispatchEvent(new Event('error'))
    }
    expect(await pending).toBeInstanceOf(Error)
    expect(socket.closed).toBe(true)
  }
})

test('startup closure and timeout reject opening rather than leaving it pending', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const closed = connectControlClient('ws://fixture').catch((error) => error)
  FakeSocket.latest.close()
  expect(await closed).toBeInstanceOf(Error)
  await expect(
    connectControlClient('ws://fixture', { timeoutMs: 5 }),
  ).rejects.toThrow('timed out')
  expect(FakeSocket.latest.closed).toBe(true)
})

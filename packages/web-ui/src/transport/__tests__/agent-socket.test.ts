import { afterEach, expect, test } from 'bun:test'
import { connectAgentClient } from '../agent-socket'

const realSocket = globalThis.WebSocket
class FakeSocket extends EventTarget {
  static latest: FakeSocket
  closed = false
  sent: string[] = []
  constructor(_url: string) {
    super()
    FakeSocket.latest = this
  }
  send(data: string): void {
    this.sent.push(data)
  }
  close(): void {
    if (this.closed) {
      return
    }
    this.closed = true
    this.dispatchEvent(new Event('close'))
  }
}
afterEach(() => {
  globalThis.WebSocket = realSocket
})

test('malformed JSON closes the connection and rejects unconfirmed sends', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const opening = connectAgentClient('ws://fixture')
  const socket = FakeSocket.latest
  socket.dispatchEvent(new Event('open'))
  const client = await opening
  const pending = client
    .submit([
      {
        type: 'text',
        text: 'retain this draft',
      },
    ])
    .catch((error) => error)
  expect(() =>
    socket.dispatchEvent(new MessageEvent('message', { data: '{broken' })),
  ).not.toThrow()
  expect(await pending).toBeInstanceOf(Error)
  expect(socket.closed).toBe(true)
})

test('an invalid product frame is handled like a failed connection', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const opening = connectAgentClient('ws://fixture', {
    decodeFrame() {
      throw new Error('Invalid frame')
    },
  })
  const socket = FakeSocket.latest
  socket.dispatchEvent(new Event('open'))
  const client = await opening
  let disconnected = false
  client.subscribe((event) => {
    disconnected = event.type === 'disconnected'
  })
  socket.dispatchEvent(new MessageEvent('message', { data: '{}' }))
  expect(disconnected).toBe(true)
  expect(socket.closed).toBe(true)
})

test('canceling socket startup closes the transport without waiting for its timeout', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const controller = new AbortController()
  const result = connectAgentClient('ws://fixture', {
    signal: controller.signal,
  }).catch((error) => error)
  controller.abort()
  expect(await result).toBeInstanceOf(Error)
  expect(FakeSocket.latest.closed).toBe(true)
})

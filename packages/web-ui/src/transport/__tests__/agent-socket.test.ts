import { afterEach, expect, test } from 'bun:test'
import { AgentSocketError, connectAgentClient } from '../agent-socket'

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

test('a socket that opens carries the client from then on', async () => {
  // The fake takes the socket's place for the code under test; it has no type of the DOM's.
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const opening = connectAgentClient('ws://fixture')
  const socket = FakeSocket.latest
  socket.dispatchEvent(new Event('open'))
  const client = await opening
  client.cancelPendingSteer('steer')
  expect(socket.sent.map((data) => JSON.parse(data))).toEqual([{ type: 'cancel_pending_steer', steerId: 'steer' }])
  let disconnected = false
  client.subscribe((event) => {
    disconnected ||= event.type === 'disconnected'
  })
  socket.close()
  expect(disconnected).toBe(true)
})

test('a socket that closes before it opens is a connection failure, which the runtime retries', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const result = connectAgentClient('ws://fixture').catch((error) => error)
  FakeSocket.latest.close()
  expect(await result).toBeInstanceOf(AgentSocketError)
})

test('canceling socket startup closes the transport without waiting for its timeout', async () => {
  globalThis.WebSocket = FakeSocket as unknown as typeof WebSocket
  const controller = new AbortController()
  const result = connectAgentClient('ws://fixture', controller.signal).catch((error) => error)
  controller.abort()
  expect(await result).toBeInstanceOf(Error)
  expect(FakeSocket.latest.closed).toBe(true)
})

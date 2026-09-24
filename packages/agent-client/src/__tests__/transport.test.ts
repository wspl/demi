import { expect, test } from 'bun:test'
import { AgentClient } from '../client'
import { createWebSocketTransport, type SocketClose, type SocketMessage, type WebSocketLike } from '../transport'
import { model, text, user } from './harness'

/** A socket whose other end the test plays: what the client wrote, and what the server says. */
class FakeSocket implements WebSocketLike {
  readonly written: string[] = []
  closed = false
  private readonly messages = new Set<(event: SocketMessage) => void>()
  private readonly closes = new Set<(event: SocketClose) => void>()
  private readonly errors = new Set<(event: unknown) => void>()

  send(data: string): void {
    this.written.push(data)
  }

  close(): void {
    this.closed = true
  }

  addEventListener(type: string, listener: (event: never) => void): void {
    this.listeners(type).add(listener)
  }

  removeEventListener(type: string, listener: (event: never) => void): void {
    this.listeners(type).delete(listener)
  }

  message(data: unknown): void {
    for (const listener of this.messages) {
      listener({ data })
    }
  }

  end(code: number, reason: string): void {
    for (const listener of this.closes) {
      listener({ code, reason })
    }
  }

  private listeners(type: string): Set<(event: never) => void> {
    switch (type) {
      case 'message':
        return this.messages
      case 'close':
        return this.closes
      case 'error':
        return this.errors
      default:
        throw new Error(`no ${type} event`)
    }
  }
}

// A browser `WebSocket` is what the transport takes.
const acceptsBrowserSockets: WebSocket extends WebSocketLike ? true : false = true

test('frames travel as JSON text messages both ways', async () => {
  expect(acceptsBrowserSockets).toBe(true)
  const socket = new FakeSocket()
  const client = new AgentClient(createWebSocketTransport(socket))
  const opening = client.open(model)
  expect(socket.written.map((data) => JSON.parse(data))).toEqual([{ type: 'open', model }])
  socket.message(JSON.stringify({ type: 'opened' }))
  await opening

  const submitting = client.submit([{ type: 'text', text: 'hello' }], 'm1')
  expect(JSON.parse(socket.written.at(-1) ?? '')).toEqual({ type: 'send', messageId: 'm1', content: [{ type: 'text', text: 'hello' }] })
  socket.message(JSON.stringify({ type: 'transcript_reset', blocks: [user('u1', 'm1', 'hello')], version: { epoch: 'e', revision: 1 } }))
  await submitting
  socket.message(JSON.stringify({ type: 'transcript_patch', patches: [{ op: 'add', index: 1, value: text('t1', 'hi') }], revision: 2 }))
  expect(client.transcript().blocks.map((block) => block.id)).toEqual(['u1', 't1'])
})

test('a message that is not JSON text closes the socket and disconnects the client', () => {
  for (const data of ['not json', new Uint8Array([123, 125])]) {
    const socket = new FakeSocket()
    const client = new AgentClient(createWebSocketTransport(socket))
    const endings: string[] = []
    client.subscribe((event) => {
      if (event.type === 'disconnected') {
        endings.push(event.error.message)
      }
    })
    socket.message(data)
    expect(socket.closed).toBe(true)
    expect(endings).toHaveLength(1)
    socket.message(JSON.stringify({ type: 'opened' }))
    expect(endings).toHaveLength(1)
  }
})

test('a socket the server closes disconnects the client with the close code', () => {
  const socket = new FakeSocket()
  const client = new AgentClient(createWebSocketTransport(socket))
  const endings: string[] = []
  client.subscribe((event) => {
    if (event.type === 'disconnected') {
      endings.push(event.error.message)
    }
  })
  socket.end(4001, 'lagged')
  expect(endings).toEqual(['The agent socket closed (4001 lagged)'])
})

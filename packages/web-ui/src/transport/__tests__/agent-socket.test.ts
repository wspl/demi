import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { AgentSocketError, connectAgentClient } from '../agent-socket'
import { playSockets } from './test-socket'

let sockets: ReturnType<typeof playSockets>
beforeEach(() => {
  sockets = playSockets()
})
afterEach(() => {
  sockets.restore()
})

test('a socket that opens carries the client from then on', async () => {
  const opening = connectAgentClient('ws://fixture')
  const socket = sockets.last()
  socket.open()
  const client = await opening
  client.cancelPendingSteer('steer')
  expect(socket.sent).toEqual([{ type: 'cancel_pending_steer', steerId: 'steer' }])
  let disconnected = false
  client.subscribe((event) => {
    disconnected ||= event.type === 'disconnected'
  })
  socket.end()
  expect(disconnected).toBe(true)
})

test('a socket that closes before it opens is a connection failure, which the runtime retries', async () => {
  const result = connectAgentClient('ws://fixture').catch((error) => error)
  sockets.last().end()
  expect(await result).toBeInstanceOf(AgentSocketError)
})

test('canceling socket startup closes the transport without waiting for its timeout', async () => {
  const controller = new AbortController()
  const result = connectAgentClient('ws://fixture', controller.signal).catch((error) => error)
  controller.abort()
  expect(await result).toBeInstanceOf(Error)
  expect(sockets.last().closed).toBe(true)
})

test('a socket that brings nothing for 75 seconds is let go as broken, and each message, a heartbeat included, starts the silence again', async () => {
  jest.useFakeTimers()
  try {
    const opening = connectAgentClient('ws://fixture')
    const socket = sockets.last()
    socket.open()
    const client = await opening
    const endings: unknown[] = []
    client.subscribe((event) => {
      if (event.type === 'disconnected') {
        endings.push(event.error)
      }
    })
    jest.advanceTimersByTime(30_000)
    socket.receive({ type: 'heartbeat' })
    jest.advanceTimersByTime(74_999)
    expect(endings).toEqual([])
    expect(socket.closed).toBe(false)
    jest.advanceTimersByTime(1)
    expect(endings).toHaveLength(1)
    // A transport failure, which the runtime answers by connecting again.
    expect(endings[0]).toBeInstanceOf(AgentSocketError)
    expect(socket.closed).toBe(true)
  } finally {
    jest.useRealTimers()
  }
})

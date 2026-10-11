import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { ConversationSocketError, connectConversationClient } from '../conversation-socket'
import { pageReturned } from '../liveness'
import { playSockets } from './test-socket'

let sockets: ReturnType<typeof playSockets>
beforeEach(() => {
  sockets = playSockets()
})
afterEach(() => {
  sockets.restore()
})

test('a socket that opens carries the client from then on', async () => {
  const opening = connectConversationClient('ws://fixture')
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
  const result = connectConversationClient('ws://fixture').catch((error) => error)
  sockets.last().end()
  expect(await result).toBeInstanceOf(ConversationSocketError)
})

test('canceling socket startup closes the transport without waiting for its timeout', async () => {
  const controller = new AbortController()
  const result = connectConversationClient('ws://fixture', controller.signal).catch((error) => error)
  controller.abort()
  expect(await result).toBeInstanceOf(Error)
  expect(sockets.last().closed).toBe(true)
})

test('a socket that brings nothing for 75 seconds is let go as broken, and each message, a heartbeat included, starts the silence again', async () => {
  jest.useFakeTimers()
  try {
    const opening = connectConversationClient('ws://fixture')
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
    // The judgement waits for the messages already queued.
    jest.advanceTimersByTime(0)
    expect(endings).toHaveLength(1)
    // A transport failure, which the runtime answers by connecting again.
    expect(endings[0]).toBeInstanceOf(ConversationSocketError)
    expect(socket.closed).toBe(true)
  } finally {
    jest.useRealTimers()
  }
})

// `web-application.md` § Liveness and reconnection: the return reads what
// came in while the page was away before it judges a socket. Cost: fake
// timers, a few milliseconds.
test('a page back from a long freeze reads the heartbeat its socket holds before it judges the socket, and keeps it', async () => {
  jest.useFakeTimers()
  try {
    const opening = connectConversationClient('ws://fixture')
    const socket = sockets.last()
    socket.open()
    const client = await opening
    let disconnected = false
    client.subscribe((event) => {
      disconnected ||= event.type === 'disconnected'
    })
    // The browser froze the hidden tab for 80 seconds: the clock went on,
    // the watch's timer did not, and the heartbeat the backend sent meanwhile
    // waits in the tab's queue, delivered once the tab runs again, after the
    // event that says the page is visible.
    jest.setSystemTime(Date.now() + 80_000)
    setTimeout(() => socket.receive({ type: 'heartbeat' }), 0)
    const returned = pageReturned()
    jest.advanceTimersByTime(0)
    await returned
    expect(disconnected).toBe(false)
    expect(socket.closed).toBe(false)
    // The heartbeat started its silence again.
    jest.advanceTimersByTime(75_000)
    // The judgement waits for the messages already queued.
    jest.advanceTimersByTime(0)
    expect(socket.closed).toBe(true)
  } finally {
    jest.useRealTimers()
  }
})

// The same rule for the watch's own timer: a tab frozen while it stays
// visible runs its timers that came due as it runs again, before the
// messages it held. Cost: fake timers, a few milliseconds.
test('a silence watch whose timer comes due in a frozen tab waits for the heartbeat queued behind it, and keeps the socket', async () => {
  jest.useFakeTimers()
  try {
    const opening = connectConversationClient('ws://fixture')
    const socket = sockets.last()
    socket.open()
    await opening
    // The heartbeat the backend sent while the tab was frozen is delivered
    // just after the watch's timer, which came due meanwhile.
    setTimeout(() => socket.receive({ type: 'heartbeat' }), 75_000)
    jest.advanceTimersByTime(75_000)
    jest.advanceTimersByTime(0)
    expect(socket.closed).toBe(false)
  } finally {
    jest.useRealTimers()
  }
})

import { afterEach, beforeEach, expect, jest, test } from 'bun:test'
import { ConversationSocketError, connectConversationClient } from '../conversation-socket'
import { reportPageErrorsWith } from '../../infra/errors'
import { followPage, pageReturned, pageWentAway, pendingReturnCheck } from '../liveness'
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
  client.steerNow('steer')
  expect(socket.sent).toEqual([{ type: 'steer_now', steerId: 'steer' }])
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

/** Collects the page's error reports until the answer is called. */
function collectReports(): { messages: string[]; stop: () => void } {
  const messages: string[] = []
  const stop = reportPageErrorsWith(async (report) => {
    messages.push(report.message)
    return null
  })
  return { messages, stop }
}

// `web-application.md` § Liveness and reconnection: what the return reads
// may have waited in the browser since before the page went away, so only
// a pong to the return's own ping shows the socket alive. Cost: fake
// timers, a few milliseconds.
test('a page back with a heartbeat that waited in its queue asks its socket for a pong, and replaces a socket that brings none within 5 seconds', async () => {
  jest.useFakeTimers()
  const reports = collectReports()
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
    // The phone's system cut the connection without a close while the page
    // was away; the heartbeat that came just before waits in the tab's
    // queue, delivered once the tab runs again.
    jest.setSystemTime(Date.now() + 120_000)
    setTimeout(() => socket.receive({ type: 'heartbeat' }), 0)
    const returned = pageReturned()
    jest.advanceTimersByTime(0)
    expect(socket.pings).toHaveLength(1)
    jest.advanceTimersByTime(4_999)
    jest.advanceTimersByTime(0)
    expect(socket.closed).toBe(false)
    jest.advanceTimersByTime(1)
    // The judgement waits for the messages already queued.
    jest.advanceTimersByTime(0)
    expect(socket.closed).toBe(true)
    expect(endings).toHaveLength(1)
    expect(endings[0]).toBeInstanceOf(ConversationSocketError)
    expect(reports.messages).toEqual(['The conversation socket brought no pong at return within 5 seconds'])
    await returned
  } finally {
    reports.stop()
    jest.useRealTimers()
  }
})

test('a page back whose socket answers the return\'s ping keeps it, and the return ends with the pong', async () => {
  jest.useFakeTimers()
  const reports = collectReports()
  try {
    const opening = connectConversationClient('ws://fixture')
    const socket = sockets.last()
    socket.open()
    const client = await opening
    let disconnected = false
    client.subscribe((event) => {
      disconnected ||= event.type === 'disconnected'
    })
    // Away for less than the silence the watch allows by the clock.
    jest.setSystemTime(Date.now() + 60_000)
    let ended = false
    const returned = pageReturned().then(() => {
      ended = true
    })
    jest.advanceTimersByTime(0)
    const [id] = socket.pings
    await Promise.resolve()
    expect(ended).toBe(false)
    socket.receive({ type: 'pong', id: id! })
    await returned
    jest.advanceTimersByTime(5_000)
    jest.advanceTimersByTime(0)
    expect(socket.closed).toBe(false)
    expect(disconnected).toBe(false)
    expect(reports.messages).toEqual([])
    // Its watch would otherwise be asked at the next test's return.
    client.disconnect()
  } finally {
    reports.stop()
    jest.useRealTimers()
  }
})

// A pong that answers an earlier return's ping says nothing of the socket
// after the page went away again. Cost: fake timers, a few milliseconds.
test('a page that goes away again before its ping is answered asks again at the next return, and an old pong does not keep a dead socket', async () => {
  jest.useFakeTimers()
  try {
    const opening = connectConversationClient('ws://fixture')
    const socket = sockets.last()
    socket.open()
    const client = await opening
    jest.setSystemTime(Date.now() + 60_000)
    pageReturned()
    jest.advanceTimersByTime(0)
    expect(socket.pings).toHaveLength(1)
    // Hidden again before the pong; the connection dies meanwhile, after
    // the pong to the first ping reached the browser, which delivers it
    // only once the page runs again.
    pageWentAway()
    jest.advanceTimersByTime(1_000)
    setTimeout(() => socket.receive({ type: 'pong', id: socket.pings[0]! }), 0)
    const returned = pageReturned()
    jest.advanceTimersByTime(0)
    expect(socket.pings).toHaveLength(2)
    jest.advanceTimersByTime(4_999)
    jest.advanceTimersByTime(0)
    expect(socket.closed).toBe(false)
    jest.advanceTimersByTime(1)
    jest.advanceTimersByTime(0)
    expect(socket.closed).toBe(true)
    await returned
    client.disconnect()
  } finally {
    jest.useRealTimers()
  }
})

test('a socket still opening when the page went away is abandoned at its return, and one opened since is left to open', async () => {
  jest.useFakeTimers()
  try {
    const before = connectConversationClient('ws://fixture').catch((error: unknown) => error)
    const opening = sockets.last()
    pageWentAway()
    const since = connectConversationClient('ws://fixture')
    const later = sockets.last()
    pageReturned()
    jest.advanceTimersByTime(0)
    expect(opening.closed).toBe(true)
    // A transport failure, which the runtime answers by connecting again.
    expect(await before).toBeInstanceOf(ConversationSocketError)
    expect(later.closed).toBe(false)
    later.open()
    ;(await since).disconnect()
  } finally {
    jest.useRealTimers()
  }
})

test('the browser showing the page from its back-forward cache, or resuming a frozen page, is a return; a first show is not', async () => {
  jest.useFakeTimers()
  const window = new EventTarget()
  const document = Object.assign(new EventTarget(), { visibilityState: 'visible' as DocumentVisibilityState })
  const stop = followPage(window, document)
  try {
    const opening = connectConversationClient('ws://fixture')
    const socket = sockets.last()
    socket.open()
    const client = await opening
    window.dispatchEvent(Object.assign(new Event('pageshow'), { persisted: false }))
    jest.advanceTimersByTime(0)
    expect(socket.pings).toHaveLength(0)
    window.dispatchEvent(new Event('pagehide'))
    window.dispatchEvent(Object.assign(new Event('pageshow'), { persisted: true }))
    jest.advanceTimersByTime(0)
    expect(socket.pings).toHaveLength(1)
    socket.receive({ type: 'pong', id: socket.pings[0]! })
    await pendingReturnCheck()
    document.dispatchEvent(new Event('freeze'))
    document.dispatchEvent(new Event('resume'))
    jest.advanceTimersByTime(0)
    expect(socket.pings).toHaveLength(2)
    socket.receive({ type: 'pong', id: socket.pings[1]! })
    client.disconnect()
  } finally {
    stop()
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

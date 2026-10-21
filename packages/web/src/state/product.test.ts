import { afterEach, beforeEach, expect, jest, spyOn, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { browserCameOnline, browserWentOffline, pageReturned, pageWentAway, waitToReconnect } from '@demicodes/web-ui/transport/liveness'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { onSessionExpired } from '../api/client'
import { conversationSummary as summary, productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from './product'
import type { ProductState } from '../api/generated/web-api'

const realFetch = globalThis.fetch
const FIRST = '00000000-0000-4000-8000-000000000001'
const SECOND = '00000000-0000-4000-8000-000000000002'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** The requests to the models' routes, as `METHOD path`. */
let modelRequests: string[]
let vendorResponse: () => Promise<Response>
let vendorReads: number
let sessionReads: number

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  modelRequests = []
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  vendorReads = 0
  sessionReads = 0
  globalThis.fetch = (async (input, init) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      modelRequests.push(`${init?.method ?? 'GET'} ${path}`)
      return new Response(null, { status: 204 })
    }
    if (path === '/api/providers/catalog') {
      vendorReads++
      return vendorResponse()
    }
    if (path === '/api/auth/me') {
      sessionReads++
      return Response.json({ requestId: 'req_test000000000001', code: 'unauthenticated', message: 'Sign in' }, { status: 401 })
    }
    throw new Error(`Unexpected request: ${path}`)
  }) as typeof fetch
})

afterEach(() => {
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
  globalThis.fetch = realFetch
  jest.useRealTimers()
})

/** Starts following the state, and the channel brings `state` first. */
function started(state = productState()) {
  const product = useProduct()
  product.start()
  channels.last().connect(state)
  return product
}

const titles = () => useProduct().snapshot?.conversations.map((conversation) => conversation.title)

test('the channel\'s snapshot is the page\'s copy, and each later message replaces its part', () => {
  const product = useProduct()
  product.start()
  expect(channels.last().url).toBe('ws://127.0.0.1:3271/api/sync')
  expect(product.load).toBe('loading')
  channels.last().connect(productState({ conversations: [summary(FIRST, 'First')] }))
  expect(product.load).toBe('ready')
  const channel = channels.last()

  channel.send({ type: 'conversation', conversation: summary(FIRST, 'Renamed') })
  expect(titles()).toEqual(['Renamed'])
  // A new conversation comes before the order that places it.
  channel.send({ type: 'conversation', conversation: summary(SECOND, 'Second') })
  expect(titles()).toEqual(['Second', 'Renamed'])
  channel.send({ type: 'conversation_order', ids: [FIRST, SECOND] })
  expect(titles()).toEqual(['Renamed', 'Second'])
  channel.send({ type: 'preferences', preferences: { appearance: { theme: 'dark' }, shortcuts: {} } })
  expect(product.snapshot?.preferences.appearance.theme).toBe('dark')
  channel.send({ type: 'user', user: { ...productState().user, nickname: 'Ana' } })
  expect(product.snapshot?.user.nickname).toBe('Ana')
  const passkey = { id: 'cHVibGlj', name: 'iCloud Keychain', synced: true, createdAt: '2026-10-13T08:00:00.000Z', lastUsedAt: null }
  channel.send({ type: 'passkeys', passkeys: [passkey], userHandle: 'dXNlci0x' })
  expect(product.snapshot?.passkeys).toEqual({ passkeys: [passkey], userHandle: 'dXNlci0x' })
  const phone = {
    id: 'session-2',
    name: 'Safari on iPhone',
    createdAt: '2026-10-13T08:00:00.000Z',
    lastActiveAt: '2026-10-13T08:00:00.000Z',
    current: false,
    handoffId: 'handoff-1',
    notifications: false,
  }
  channel.send({ type: 'sessions', sessions: [phone] })
  expect(product.snapshot?.sessions).toEqual([phone])
  channel.send({ type: 'heartbeat' })
  expect(titles()).toEqual(['Renamed', 'Second'])
  // A later snapshot, as after a reconnect, replaces the whole copy.
  channel.send({ type: 'snapshot', state: productState({ conversations: [summary(SECOND, 'Only')] }) })
  expect(titles()).toEqual(['Only'])
})

test('the channel tells the backend which conversation the page shows as it changes, and each new connection tells it again', () => {
  jest.useFakeTimers()
  const product = started()
  const channel = channels.last()
  product.show(FIRST)
  product.show(FIRST)
  product.show(null)
  expect(channel.fromPage).toEqual([{ showing: FIRST }, { showing: null }])
  product.show(SECOND)
  // The backend forgets it with the connection; the next one says it again.
  channel.end()
  jest.advanceTimersByTime(30_000)
  const next = channels.last()
  expect(next).not.toBe(channel)
  next.connect(productState())
  expect(next.fromPage).toEqual([{ showing: SECOND }])
})

test('a plugin turned off leaves the page with its state, and one turned on again brings its state afresh', () => {
  const product = started()
  const channel = channels.last()
  const skills = productState().plugins[0]!

  channel.send({ type: 'plugins', plugins: [{ ...skills, enabled: false }] })
  expect(product.snapshot?.plugins).toEqual([{ ...skills, enabled: false }])
  expect(product.snapshot?.pluginStates).toEqual({})

  channel.send({ type: 'plugins', plugins: [skills] })
  channel.send({ type: 'plugin', plugin: 'skills', state: { sources: [] } })
  expect(product.snapshot?.pluginStates).toEqual({ skills: { sources: [] } })
})

test('a page of another build than the backend serves is out of date, as soon as a snapshot says so', () => {
  process.env.DEMI_WEB_BUILD = 'b1'
  try {
    const product = started(productState({ webBuild: 'b1' }))
    expect(product.newBuild).toBeNull()
    // The backend restarted with a new release; the channel connects again.
    channels.last().send({ type: 'snapshot', state: productState({ webBuild: 'b2' }) })
    expect(product.newBuild).toBe('b2')
    // A backend that serves no web app, as in development, says nothing.
    channels.last().send({ type: 'snapshot', state: productState({ webBuild: null }) })
    expect(product.newBuild).toBeNull()
  } finally {
    delete process.env.DEMI_WEB_BUILD
  }
})

test('a snapshot of another build that this page cannot read still names the build to load, and the banner goes', () => {
  process.env.DEMI_WEB_BUILD = 'b1'
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(1)
  try {
    const product = started(productState({ webBuild: 'b1' }))
    channels.last().end(1001, 'backend_closing')
    expect(product.connection).toBe('restarting')
    jest.advanceTimersByTime(2_000)
    // The new release's Cloud has a state this build does not know.
    const state = productState({ webBuild: 'b2' })
    channels.last().open()
    channels.last().deliver({ type: 'snapshot', state: { ...state, cloud: { ...state.cloud, state: 'migrating' } } })
    expect(product.newBuild).toBe('b2')
    expect(product.connection).toBeNull()
    // The page loads the new build; the channel is not replaced meanwhile.
    const count = channels.opened.length
    jest.advanceTimersByTime(60_000)
    expect(channels.opened.length).toBe(count)
  } finally {
    random.mockRestore()
    delete process.env.DEMI_WEB_BUILD
  }
})

test('a connection whose handshake does not complete within 10 seconds is abandoned and tried again', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(1)
  try {
    const product = started()
    channels.last().end(1001, 'backend_closing')
    jest.advanceTimersByTime(2_000)
    // A proxy in front of the restarting backend holds the upgrade.
    const held = channels.last()
    const count = channels.opened.length
    jest.advanceTimersByTime(9_999)
    expect(held.closed).toBe(false)
    jest.advanceTimersByTime(1)
    expect(held.closed).toBe(true)
    expect(product.connection).toBe('restarting')
    jest.advanceTimersByTime(2_000)
    expect(channels.opened.length).toBe(count + 1)
    channels.last().connect(productState())
    expect(product.connection).toBeNull()
  } finally {
    random.mockRestore()
  }
})

// When the channel's snapshot arrives after the backend was away, every
// other socket of the page that waits to connect connects at once, so no
// Connecting row outlives the banner. Cost: fake timers, a few milliseconds.
test('a socket waiting its backoff, as a conversation\'s does, connects as soon as the channel\'s snapshot is back after a restart', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(1)
  try {
    const product = started()
    channels.last().end(1001, 'backend_closing')
    // A conversation's socket the restart closed too, waiting 8 seconds after its fifth failed try.
    const connected: number[] = []
    const conversation = waitToReconnect(5, () => connected.push(Date.now()))
    jest.advanceTimersByTime(2_000)
    channels.last().connect(productState())
    expect(product.connection).toBeNull()
    expect(connected).toHaveLength(1)
    conversation.cancel()
  } finally {
    random.mockRestore()
  }
})

test('a write\'s answer shows at once, unless the channel brought its part since the write was sent', () => {
  const product = started(productState({ conversations: [summary(FIRST, 'First'), summary(SECOND, 'Second')] }))
  const channel = channels.last()

  // Nothing came on the channel meanwhile: the answer shows.
  const renamed = product.sent()
  expect(product.answered(renamed, { type: 'conversation', conversation: summary(FIRST, 'Renamed') })).toBe(true)
  expect(titles()).toEqual(['Renamed', 'Second'])

  // Tab A renames, and tab B pins just after the rename commits: A's
  // channel brings both before A's answer, which lacks the pin.
  const again = product.sent()
  channel.send({ type: 'conversation', conversation: summary(FIRST, 'Again', { pinned: true }) })
  expect(product.answered(again, { type: 'conversation', conversation: summary(FIRST, 'Again') })).toBe(false)
  expect(product.snapshot?.conversations[0]).toMatchObject({ title: 'Again', pinned: true })

  // Another part that came meanwhile does not hold the answer back.
  const other = product.sent()
  channel.send({ type: 'conversation', conversation: summary(SECOND, 'Second', { unread: true }) })
  expect(product.answered(other, { type: 'conversation', conversation: summary(FIRST, 'Final', { pinned: true }) })).toBe(true)
  expect(titles()).toEqual(['Final', 'Second'])
})

test('a change shows over the state read until its write lands, and one the channel overtook leaves with the next value', () => {
  const product = started(productState({ conversations: [summary(FIRST, 'First'), summary(SECOND, 'Second')] }))
  const channel = channels.last()
  const rename = (id: string, title: string) => product.change(`conversation:${id}`, (state) => ({
    ...state,
    conversations: state.conversations.map((item) => item.id === id ? { ...item, title } : item),
  }))

  // Shown at once; the channel's value of the part, read before the write
  // went out, does not bring the old title back.
  const first = rename(FIRST, 'Renamed')
  expect(titles()).toEqual(['Renamed', 'Second'])
  first.send()
  channel.send({ type: 'conversation', conversation: summary(FIRST, 'First', { unread: true }) })
  expect(product.snapshot?.conversations[0]).toMatchObject({ title: 'Renamed', unread: true })
  // The answer came after the channel's value: the change stays until the
  // part's next value, which is the write's own.
  first.land({ type: 'conversation', conversation: summary(FIRST, 'Renamed') })
  expect(titles()).toEqual(['Renamed', 'Second'])
  channel.send({ type: 'conversation', conversation: summary(FIRST, 'Renamed') })
  // Nothing of this page's holds the part any more: another page's title shows.
  channel.send({ type: 'conversation', conversation: summary(FIRST, 'Elsewhere') })
  expect(titles()).toEqual(['Elsewhere', 'Second'])

  // A refused change leaves at once, and the backend's value shows.
  const refused = rename(SECOND, 'Refused')
  refused.send()
  expect(titles()).toEqual(['Elsewhere', 'Refused'])
  refused.drop()
  expect(titles()).toEqual(['Elsewhere', 'Second'])

  // An answer that came first lands the change; signing out releases those on their way.
  const landed = rename(SECOND, 'Landed')
  landed.send()
  landed.land()
  expect(product.read?.conversations[1]?.title).toBe('Landed')
  rename(FIRST, 'Unsent')
  product.stop()
  expect(product.snapshot).toBeNull()
})

test('a closed channel connects again after a second, then twice as long each time, up to 30 seconds', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    started()
    const waits: number[] = []
    for (let closes = 0; closes < 7; closes += 1) {
      const count = channels.opened.length
      channels.last().end(1006)
      let waited = 0
      while (channels.opened.length === count) {
        jest.advanceTimersByTime(100)
        waited += 100
      }
      waits.push(waited)
      channels.last().open()
    }
    expect(waits).toEqual([1_000, 2_000, 4_000, 8_000, 16_000, 30_000, 30_000])
    // A channel that brought its snapshot is healthy: the next close waits
    // a second again.
    channels.last().send({ type: 'snapshot', state: productState() })
    const count = channels.opened.length
    channels.last().end(1006)
    jest.advanceTimersByTime(999)
    expect(channels.opened.length).toBe(count)
    jest.advanceTimersByTime(1)
    expect(channels.opened.length).toBe(count + 1)
  } finally {
    random.mockRestore()
  }
})

test('a backend that shuts down shows the banner Demi is restarting, tried every one to two seconds until a snapshot comes', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(1)
  try {
    const product = started()
    expect(product.connection).toBeNull()
    channels.last().end(1001, 'backend_closing')
    expect(product.connection).toBe('restarting')
    // Connections that end while the backend is away keep the banner and
    // the short wait: none doubles.
    for (let refused = 0; refused < 4; refused += 1) {
      const count = channels.opened.length
      jest.advanceTimersByTime(1_999)
      expect(channels.opened.length).toBe(count)
      jest.advanceTimersByTime(1)
      expect(channels.opened.length).toBe(count + 1)
      channels.last().open()
      channels.last().end(1006)
      expect(product.connection).toBe('restarting')
    }
    jest.advanceTimersByTime(2_000)
    channels.last().connect(productState())
    expect(product.connection).toBeNull()
  } finally {
    random.mockRestore()
  }
})

test('a channel lost without the backend saying so shows no banner until its first new attempt fails too', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    const product = started()
    channels.last().end(1006)
    // A blip the next connection mends shows nothing.
    expect(product.connection).toBeNull()
    jest.advanceTimersByTime(1_000)
    channels.last().end(1006)
    expect(product.connection).toBe('reconnecting')
    jest.advanceTimersByTime(2_000)
    channels.last().connect(productState())
    expect(product.connection).toBeNull()
  } finally {
    random.mockRestore()
  }
})

test('a first connection that cannot reach the backend says the page connects; one the backend fails shows its failure in the regions, not the banner', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    const product = useProduct()
    product.start()
    expect(product.connecting).toBe(false)
    channels.last().end(1006)
    jest.advanceTimersByTime(1_000)
    channels.last().end(1006)
    expect(product.load).toBe('loading')
    expect(product.connecting).toBe(true)
    expect(product.connection).toBeNull()
    jest.advanceTimersByTime(2_000)
    channels.last().end(1011, 'internal_error')
    expect(product.load).toBe('failed')
    expect(product.connection).toBeNull()
  } finally {
    random.mockRestore()
  }
})

test('each wait is shortened by a random part, so the pages of all users do not return at once', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(1)
  try {
    started()
    const count = channels.opened.length
    channels.last().end(1006)
    jest.advanceTimersByTime(499)
    expect(channels.opened.length).toBe(count)
    jest.advanceTimersByTime(1)
    expect(channels.opened.length).toBe(count + 1)
  } finally {
    random.mockRestore()
  }
})

test('a channel that brings nothing for 75 seconds is taken as broken and replaced', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    started()
    const broken = channels.last()
    jest.advanceTimersByTime(30_000)
    broken.send({ type: 'heartbeat' })
    jest.advanceTimersByTime(74_999)
    expect(broken.closed).toBe(false)
    jest.advanceTimersByTime(1)
    // The judgement waits for the messages already queued.
    jest.advanceTimersByTime(0)
    expect(broken.closed).toBe(true)
    jest.advanceTimersByTime(1_000)
    expect(channels.last()).not.toBe(broken)
  } finally {
    random.mockRestore()
  }
})

test('a first connection that fails asks whether the session ended, and a retry connects at once', async () => {
  const ended: string[] = []
  const stopListening = onSessionExpired(() => ended.push('ended'))
  try {
    const product = useProduct()
    product.start()
    const refused = channels.last()
    refused.end(1006)
    expect(product.connecting).toBe(true)
    // The web browser does not say why the upgrade failed; the session's read
    // does, and its 401 ends the session.
    await waitFor(() => ended.length > 0, () => `session reads: ${sessionReads}`)
    expect(sessionReads).toBe(1)
    product.reconnect()
    expect(channels.last()).not.toBe(refused)
    expect(product.load).toBe('loading')
    channels.last().connect(productState())
    expect(product.load).toBe('ready')
  } finally {
    stopListening()
  }
})

test('a channel closed as session_ended ends the session and connects no more', () => {
  jest.useFakeTimers()
  const ended: string[] = []
  const stopListening = onSessionExpired(() => ended.push('ended'))
  try {
    started()
    const count = channels.opened.length
    channels.last().end(4002, 'session_ended')
    expect(ended).toEqual(['ended'])
    jest.advanceTimersByTime(60_000)
    expect(channels.opened.length).toBe(count)
  } finally {
    stopListening()
  }
})

test('a page that returns connects its closed channel at once instead of waiting', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    started()
    for (let closes = 0; closes < 4; closes += 1) {
      channels.last().end(1006)
      jest.advanceTimersByTime(30_000)
    }
    const count = channels.opened.length
    channels.last().end(1006)
    pageReturned()
    jest.advanceTimersByTime(0)
    expect(channels.opened.length).toBe(count + 1)
    // A channel that connects, or one heard from lately, stays as it is.
    pageReturned()
    jest.advanceTimersByTime(0)
    channels.last().connect(productState())
    pageReturned()
    jest.advanceTimersByTime(0)
    expect(channels.opened.length).toBe(count + 1)
  } finally {
    random.mockRestore()
  }
})

test('a page back from sleep replaces a channel silent for 75 seconds at once, and one silent for less when its 75 seconds are up by the clock', () => {
  jest.useFakeTimers()
  started()
  const slept = channels.last()
  // The laptop sleeps: the clock goes on, the watch's timer does not, and
  // no close reaches the page.
  jest.setSystemTime(Date.now() + 75_000)
  pageReturned()
  jest.advanceTimersByTime(0)
  expect(slept.closed).toBe(true)
  const next = channels.last()
  expect(next).not.toBe(slept)
  next.connect(productState())
  const heard = Date.now()
  jest.setSystemTime(heard + 74_000)
  pageReturned()
  jest.advanceTimersByTime(0)
  expect(next.closed).toBe(false)
  // Its 75 seconds since the snapshot are up a moment later by the clock.
  jest.advanceTimersByTime(heard + 75_000 - Date.now() - 1)
  expect(next.closed).toBe(false)
  jest.advanceTimersByTime(1)
  // The judgement waits for the messages already queued.
  jest.advanceTimersByTime(0)
  expect(next.closed).toBe(true)
})

// The return asks the
// channel for a pong; one that answers was never broken, and the page shows
// nothing of the check. Cost: fake timers, a few milliseconds.
test('a page back whose channel answers the return\'s ping keeps it and shows no banner', async () => {
  jest.useFakeTimers()
  const product = started()
  const channel = channels.last()
  jest.setSystemTime(Date.now() + 60_000)
  const returned = pageReturned()
  jest.advanceTimersByTime(0)
  const pings = channel.fromPage.filter((sent): sent is { ping: number } => typeof sent === 'object' && sent !== null && 'ping' in sent)
  expect(pings).toHaveLength(1)
  channel.send({ type: 'pong', id: pings[0]!.ping })
  await returned
  jest.advanceTimersByTime(10_000)
  jest.advanceTimersByTime(0)
  expect(channel.closed).toBe(false)
  expect(channels.last()).toBe(channel)
  expect(product.connection).toBeNull()
})

test('a channel still connecting when the page went away is abandoned at the return and connects again at once', () => {
  jest.useFakeTimers()
  const random = spyOn(Math, 'random').mockReturnValue(0)
  try {
    const product = useProduct()
    product.start()
    const stale = channels.last()
    pageWentAway()
    pageReturned()
    jest.advanceTimersByTime(0)
    expect(stale.closed).toBe(true)
    // Not after the wait the failed attempt counts toward.
    jest.advanceTimersByTime(0)
    expect(channels.last()).not.toBe(stale)
  } finally {
    random.mockRestore()
  }
})

// The browser's word that
// it went offline counts once it has held for a second, as a network that
// macOS re-evaluates reports offline for a moment; one that holds shows the
// banner and closes no socket, which may still work through a VPN's tunnel,
// and coming back online checks each socket as at a return. Cost: fake
// timers, a few milliseconds.
test('a moment offline under a second shows no banner; one that holds shows it and keeps the channel, and back online the return keeps a live channel', () => {
  jest.useFakeTimers()
  try {
    const product = started()
    const channel = channels.last()
    browserWentOffline()
    jest.advanceTimersByTime(500)
    browserCameOnline()
    jest.advanceTimersByTime(5_000)
    expect(product.connection).toBeNull()

    browserWentOffline()
    jest.advanceTimersByTime(999)
    expect(product.connection).toBeNull()
    jest.advanceTimersByTime(1)
    expect(product.connection).toBe('offline')
    expect(channel.closed).toBe(false)
    // The channel goes on working through the tunnel while the browser says offline.
    channel.send({ type: 'heartbeat' })
    jest.advanceTimersByTime(30_000)
    browserCameOnline()
    jest.advanceTimersByTime(0)
    expect(product.connection).toBeNull()
    expect(channel.closed).toBe(false)
    expect(channels.last()).toBe(channel)
  } finally {
    // The page is online again for the tests that follow.
    browserCameOnline()
  }
})

test('back online after an offline that held, the return replaces a channel silent past the limit by the clock', () => {
  jest.useFakeTimers()
  try {
    started()
    const silent = channels.last()
    browserWentOffline()
    jest.advanceTimersByTime(1_000)
    // The laptop slept while offline: the clock went on, the watch's timer did not.
    jest.setSystemTime(Date.now() + 80_000)
    expect(silent.closed).toBe(false)
    browserCameOnline()
    jest.advanceTimersByTime(0)
    expect(silent.closed).toBe(true)
    expect(channels.last()).not.toBe(silent)
  } finally {
    browserCameOnline()
  }
})

test('the page reads the vendor catalog once as it starts, and its readers share that read', async () => {
  const deferred = Promise.withResolvers<Response>()
  vendorResponse = () => deferred.promise
  const product = started()
  const reader = product.loadVendors()
  expect(product.vendorLoad).toBe('loading')
  deferred.resolve(Response.json({ subscriptions: [], vendors: [] }))
  await reader
  expect(product.vendorLoad).toBe('ready')
  await product.loadVendors()
  expect(vendorReads).toBe(1)
})

test('a page of the build that read the vendor catalog holds it from its first frame and reads it no more', async () => {
  const kept = new Map<string, string>()
  Reflect.set(globalThis, 'localStorage', {
    getItem: (key: string) => kept.get(key) ?? null,
    setItem: (key: string, value: string) => void kept.set(key, value),
  })
  process.env.DEMI_WEB_BUILD = 'b1'
  try {
    const catalog = { subscriptions: [{ providerType: 'codex' }], vendors: [] }
    vendorResponse = async () => Response.json(catalog)
    await started().loadVendors()
    // A new page of the same build: the catalog is there before any read.
    useProduct().stop()
    disposePinia(pinia)
    pinia = createPinia()
    setActivePinia(pinia)
    expect(useProduct().vendors).toEqual(catalog)
    await started().loadVendors()
    expect(vendorReads).toBe(1)

    // Another build reads it again.
    useProduct().stop()
    disposePinia(pinia)
    pinia = createPinia()
    setActivePinia(pinia)
    process.env.DEMI_WEB_BUILD = 'b2'
    expect(useProduct().vendors).toBeNull()
    await started().loadVendors()
    expect(vendorReads).toBe(2)
  } finally {
    delete process.env.DEMI_WEB_BUILD
    Reflect.deleteProperty(globalThis, 'localStorage')
  }
})

test('a vendor catalog that failed to load as the page started loads on Retry', async () => {
  vendorResponse = async () => {
    throw new Error('offline')
  }
  const product = started()
  await waitFor(() => product.vendorLoad === 'failed')
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  await product.loadVendors()
  expect(product.vendorLoad).toBe('ready')
})

test('a stopped page drops its late vendor response', async () => {
  const deferred = Promise.withResolvers<Response>()
  vendorResponse = () => deferred.promise
  const product = started()
  const previous = product.loadVendors()
  product.stop()
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  await started().loadVendors()
  deferred.resolve(Response.json({ subscriptions: [{ providerType: 'codex' }], vendors: [] }))
  await expect(previous).rejects.toThrow()
  expect(product.vendors?.subscriptions).toEqual([])
})

/** What Automatic resolves to: the model `name`, or none. */
function smallModel(name: string | null) {
  return name === null ? null : {
    providerId: 'p1',
    model: { id: name, name, contextWindow: 200_000, outputLimit: null, thinking: [], acceptedExtensions: null },
    thinking: null,
    serviceTierId: null,
  }
}

// Planted defect: the catalog read by its own route after the snapshot,
// which leaves the model's name unknown while a conversation opens.
test('the catalog comes with the first snapshot, and a newer one on the channel shows with no read', () => {
  const product = started(productState({ models: { providers: [], smallModel: smallModel('Claude Haiku') } }))
  expect(product.catalogLoad).toBe('ready')
  expect(product.automaticSmallModel?.model.name).toBe('Claude Haiku')

  // Only the Conversation's Provider turned on: the catalog's owner announces what Automatic resolves to now.
  channels.last().send({ type: 'models', models: { providers: [], smallModel: null } })
  expect(product.automaticSmallModel).toBeNull()
  expect(modelRequests).toEqual([])
})

test('a refresh asks the backend to read the models again, and the catalog comes on the channel', async () => {
  const product = started()
  await product.reloadModels()
  expect(modelRequests).toEqual(['POST /api/models/refresh'])
  channels.last().send({ type: 'models', models: { providers: [], smallModel: smallModel('Claude Haiku') } })
  expect(product.automaticSmallModel?.model.name).toBe('Claude Haiku')
})

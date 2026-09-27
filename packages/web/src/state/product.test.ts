import { afterEach, beforeEach, expect, jest, spyOn, test } from 'bun:test'
import { waitFor } from '@demicodes/utils'
import { pageReturned } from '@demicodes/web-ui/transport/liveness'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { onSessionExpired } from '../api/client'
import type { ConversationSummary } from '../api/generated/web-api'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useProduct } from './product'

const realFetch = globalThis.fetch
const FIRST = '00000000-0000-4000-8000-000000000001'
const SECOND = '00000000-0000-4000-8000-000000000002'
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
let modelResponse: () => Promise<Response>
let vendorResponse: () => Promise<Response>
let vendorReads: number
let sessionReads: number

function summary(id: string, title: string, parts: Partial<ConversationSummary> = {}): ConversationSummary {
  return {
    id, title, pinned: false, archived: false, readRevision: 0, revision: 0, unread: false,
    titleCurrent: true, titleGenerating: false, draftRevision: 0, cwd: '/home/demi', target: { kind: 'cloud' },
    contextVersion: 0, model: null, createdAt: '2026-09-09T00:00:00.000Z', updatedAt: '2026-09-09T00:00:00.000Z',
    status: 'idle', ...parts,
  }
}

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  channels = playChannels()
  modelResponse = async () => Response.json({ providers: [] })
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  vendorReads = 0
  sessionReads = 0
  globalThis.fetch = (async (input) => {
    const path = String(input)
    if (path.startsWith('/api/models')) {
      return modelResponse()
    }
    if (path === '/api/providers/catalog') {
      vendorReads++
      return vendorResponse()
    }
    if (path === '/api/auth/me') {
      sessionReads++
      return Response.json({ code: 'unauthenticated', message: 'Sign in' }, { status: 401 })
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
  channel.send({ type: 'heartbeat' })
  expect(titles()).toEqual(['Renamed', 'Second'])
  // A later snapshot, as after a reconnect, replaces the whole copy.
  channel.send({ type: 'snapshot', state: productState({ conversations: [summary(SECOND, 'Only')] }) })
  expect(titles()).toEqual(['Only'])
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
    channels.last().end(1001, 'backend_closing')
    jest.advanceTimersByTime(999)
    expect(channels.opened.length).toBe(count)
    jest.advanceTimersByTime(1)
    expect(channels.opened.length).toBe(count + 1)
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
    expect(broken.closed).toBe(true)
    jest.advanceTimersByTime(1_000)
    expect(channels.last()).not.toBe(broken)
  } finally {
    random.mockRestore()
  }
})

test('a first connection that fails shows the failure, asks whether the session ended, and a retry connects at once', async () => {
  const ended: string[] = []
  const stopListening = onSessionExpired(() => ended.push('ended'))
  try {
    const product = useProduct()
    product.start()
    const refused = channels.last()
    refused.end(1006)
    expect(product.load).toBe('failed')
    // The browser does not say why the upgrade failed; the session's read
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
    expect(channels.opened.length).toBe(count + 1)
    // A channel that connects, or one heard from lately, stays as it is.
    pageReturned()
    channels.last().connect(productState())
    pageReturned()
    expect(channels.opened.length).toBe(count + 1)
  } finally {
    random.mockRestore()
  }
})

test('a page back from sleep replaces a channel silent for 75 seconds at once, and keeps one silent for less', () => {
  jest.useFakeTimers()
  started()
  const slept = channels.last()
  const heard = Date.now()
  // The laptop sleeps: the clock goes on, the watch's timer does not, and
  // no close reaches the page.
  jest.setSystemTime(heard + 74_000)
  pageReturned()
  expect(slept.closed).toBe(false)
  jest.setSystemTime(heard + 75_000)
  pageReturned()
  expect(slept.closed).toBe(true)
  expect(channels.last()).not.toBe(slept)
})

test('vendor readers share one request and reopening uses cached data', async () => {
  const product = started()
  const deferred = Promise.withResolvers<Response>()
  vendorResponse = () => deferred.promise
  const first = product.loadVendors()
  const second = product.loadVendors()
  expect(product.vendorLoad).toBe('loading')
  expect(vendorReads).toBe(1)
  deferred.resolve(Response.json({ subscriptions: [], vendors: [] }))
  await Promise.all([first, second])
  expect(product.vendorLoad).toBe('ready')
  await product.loadVendors()
  expect(vendorReads).toBe(1)
})

test('failed catalog reads can retry, and a cached catalog survives a failed refresh', async () => {
  modelResponse = async () => {
    throw new Error('offline')
  }
  const product = started()
  expect(product.load).toBe('ready')
  await product.loadModels().catch(() => {})
  expect(product.catalogLoad).toBe('failed')
  const deferred = Promise.withResolvers<Response>()
  modelResponse = () => deferred.promise
  const retry = product.loadModels(true)
  expect(product.catalogLoad).toBe('loading')
  deferred.resolve(Response.json({ providers: [] }))
  await retry
  expect(product.catalogLoad).toBe('ready')
  modelResponse = async () => {
    throw new Error('offline')
  }
  await expect(product.loadModels(true)).rejects.toThrow('offline')
  expect(product.catalogLoad).toBe('ready')
  vendorResponse = async () => {
    throw new Error('offline')
  }
  await expect(product.loadVendors()).rejects.toThrow('offline')
  expect(product.vendorLoad).toBe('failed')
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  await product.loadVendors()
  expect(product.vendorLoad).toBe('ready')
})

test('an account change cannot populate the next account with a late vendor response', async () => {
  const product = started()
  const deferred = Promise.withResolvers<Response>()
  vendorResponse = () => deferred.promise
  const previous = product.loadVendors()
  product.stop()
  started()
  vendorResponse = async () => Response.json({ subscriptions: [], vendors: [] })
  await product.loadVendors()
  deferred.resolve(
    Response.json({
      subscriptions: [{ providerType: 'old-account', configured: true }],
      vendors: [],
    }),
  )
  await expect(previous).rejects.toThrow()
  expect(product.vendors?.subscriptions).toEqual([])
})

test('conversations and the channel\'s messages reuse one catalog until it is a minute old; concurrent reads share one request', async () => {
  const clock = spyOn(Date, 'now').mockReturnValue(1000)
  let calls = 0
  modelResponse = async () => {
    calls++
    return Response.json({ providers: [] })
  }
  try {
    const product = started()
    await product.loadModels()
    expect(calls).toBe(1)
    for (const id of ['first', 'second', 'new-conversation']) {
      product.activeConversationId = id
      channels.last().send({ type: 'providers', providers: [] })
      await product.loadModels()
    }
    expect(calls).toBe(1)
    clock.mockReturnValue(61_000)
    const pending = Promise.withResolvers<Response>()
    modelResponse = () => {
      calls++
      return pending.promise
    }
    const first = product.loadModels()
    const second = product.loadModels()
    expect(calls).toBe(2)
    expect(product.catalogLoad).toBe('ready')
    pending.resolve(Response.json({ providers: [] }))
    await Promise.all([first, second])
    await product.loadModels()
    expect(calls).toBe(2)
  } finally {
    clock.mockRestore()
  }
})

test('the first snapshot does not wait for the catalog; a sign-out leaves a late catalog to nobody', async () => {
  const pending = Promise.withResolvers<Response>()
  modelResponse = () => pending.promise
  const product = started()
  expect(product.load).toBe('ready')
  expect(product.catalogLoad).toBe('loading')
  const old = product.loadModels().catch(error => error)
  product.stop()
  modelResponse = async () => Response.json({ providers: [] })
  started()
  await product.loadModels()
  pending.resolve(Response.json({ providers: [] }))
  expect(await old).toBeInstanceOf(Error)
  expect(product.catalogLoad).toBe('ready')
})

test('a change of an entry loads the catalog again with a refresh, dropping a read under way whose configuration looks the same', async () => {
  const pending = Promise.withResolvers<Response>()
  const paths: string[] = []
  modelResponse = () => pending.promise
  const fetch = globalThis.fetch
  globalThis.fetch = (async (input, init) => {
    paths.push(String(input))
    return fetch(input, init)
  }) as typeof fetch
  const product = started()
  const old = product.loadModels().catch(error => error)
  modelResponse = async () => Response.json({ providers: [] })
  await product.reloadModels()
  expect(paths.filter((path) => path.startsWith('/api/models'))).toEqual(['/api/models', '/api/models?refresh=true'])
  pending.resolve(Response.json({ providers: [] }))
  expect(await old).toBeInstanceOf(Error)
  expect(product.catalogLoad).toBe('ready')
})

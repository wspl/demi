import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { nextTick } from 'vue'
import { waitFor } from '@demicodes/utils'
import { ApiError } from '../api/client'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { needsHomeScreen, subscribeFor, useNotifications, type BrowserSubscription, type Subscribing } from './notifications'
import { useProduct } from './product'

// Which browser shows how to add Demi to the Home Screen instead of asking
// for notifications. Pure; under a millisecond.

test('Safari on an iPhone or iPad outside the Home Screen needs Demi added there; the Home Screen app and other browsers do not', () => {
  expect(needsHomeScreen({ standalone: false })).toBe(true)
  expect(needsHomeScreen({ standalone: true })).toBe(false)
  expect(needsHomeScreen({})).toBe(false)
})

// A browser's subscription serves one session. A stand-in browser and backend;
// under a millisecond.

/** A browser whose subscriptions are named in order, with what happened to them. */
function browser(maker: string | null, subscribedAs: string | null, refuse: string[] = []) {
  const log: string[] = []
  let made = 0
  let kept = maker
  const subscription = (endpoint: string): BrowserSubscription => ({
    endpoint,
    unsubscribe: async () => {
      log.push(`unsubscribe ${endpoint}`)
      return true
    },
  })
  let current = subscribedAs === null ? null : subscription(subscribedAs)
  const steps: Subscribing<BrowserSubscription> = {
    current: async () => current,
    make: async () => {
      made += 1
      current = subscription(`new-${made}`)
      log.push(`make new-${made}`)
      return current
    },
    put: async (sent) => {
      if (refuse.includes(sent.endpoint)) {
        log.push(`put ${sent.endpoint} refused`)
        throw new ApiError(409, 'push_endpoint_taken', 'Another session holds it')
      }
      log.push(`put ${sent.endpoint}`)
    },
    maker: () => kept,
    remember: (session) => {
      kept = session
    },
  }
  return { steps, log, maker: () => kept }
}

test('a session keeps the subscription it made, and replaces one another session made before it sends one', async () => {
  const own = browser('session-a', 'old')
  await subscribeFor('session-a', own.steps)
  expect(own.log).toEqual(['put old'])

  const other = browser('session-a', 'old')
  await subscribeFor('session-b', other.steps)
  expect(other.log).toEqual(['unsubscribe old', 'make new-1', 'put new-1'])
  expect(other.maker()).toBe('session-b')

  // Without a note of who made it, the backend says whether another live
  // session holds it; then a new one is made.
  const unknown = browser(null, 'old', ['old'])
  const subscription = await subscribeFor('session-b', { ...unknown.steps, maker: () => 'session-b' })
  expect(unknown.log).toEqual(['put old refused', 'unsubscribe old', 'make new-1', 'put new-1'])
  expect(subscription.endpoint).toBe('new-1')
})

// A fixed bug: the backend announces the sessions after each subscription it
// keeps, as each says whether its browser has notifications on, and every
// part the channel brought made the page send its subscription again, about
// twice a second without end. A stand-in browser, push service and backend;
// a few milliseconds.
describe('a page with notifications on', () => {
  const realFetch = globalThis.fetch
  const stubbed = ['localStorage', 'Notification', 'PushManager', 'document', 'caches'] as const
  const realGlobals = stubbed.map((name) => Reflect.get(globalThis, name))
  let pinia: ReturnType<typeof createPinia>
  let channels: ReturnType<typeof playChannels>
  let puts: Array<{ events: unknown }>

  beforeEach(() => {
    pinia = createPinia()
    setActivePinia(pinia)
    puts = []
    const stored = new Map<string, string>([['demi.notifications.session', 'session-a']])
    Reflect.set(globalThis, 'localStorage', {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => stored.set(key, value),
      removeItem: (key: string) => stored.delete(key),
    })
    Reflect.set(globalThis, 'Notification', { permission: 'granted' })
    Reflect.set(globalThis, 'PushManager', class {})
    Reflect.set(globalThis, 'document', { visibilityState: 'visible', hasFocus: () => true, addEventListener: () => {} })
    Reflect.set(globalThis, 'caches', { open: async () => ({ put: async () => {} }) })
    const subscription = {
      endpoint: 'https://push.example.test/a',
      toJSON: () => ({ keys: { p256dh: 'key', auth: 'auth' } }),
      unsubscribe: async () => true,
    }
    const registration = { pushManager: { getSubscription: async () => subscription } }
    Object.defineProperty(navigator, 'serviceWorker', {
      configurable: true,
      value: { ready: Promise.resolve(registration), addEventListener: () => {} },
    })
    globalThis.fetch = (async (input, init) => {
      const path = String(input)
      if (path === '/api/push/subscription' && init?.method === 'PUT') {
        puts.push(JSON.parse(String(init.body)))
        return new Response(null, { status: 204 })
      }
      throw new Error(`Unexpected request: ${path}`)
    }) as typeof fetch
    channels = playChannels()
    Reflect.set(window, 'addEventListener', () => {})
  })

  afterEach(() => {
    useNotifications().stop()
    useProduct().stop()
    disposePinia(pinia)
    channels.restore()
    globalThis.fetch = realFetch
    Reflect.deleteProperty(navigator, 'serviceWorker')
    stubbed.forEach((name, index) => Reflect.set(globalThis, name, realGlobals[index]))
  })

  test('sends its subscription once, and again only when the events it chose change', async () => {
    const sessions = [{
      id: 'session-a',
      name: 'Safari on iPhone',
      createdAt: '2026-10-18T07:00:00.000Z',
      lastActiveAt: '2026-10-19T04:00:00.000Z',
      current: true,
      notifications: true,
    }]
    useProduct().start()
    channels.last().connect(productState({ sessions }))
    const notifications = useNotifications()
    notifications.settings.enabled = true
    notifications.start(() => {})
    await waitFor(() => puts.length === 1)

    // The backend announces the sessions after keeping the subscription.
    channels.last().send({ type: 'sessions', sessions })
    await nextTick()
    // The user stops notifying of failed turns: the one change to send.
    notifications.settings.turnFails = false
    await waitFor(() => puts.some((put) => (put.events as { turnFails: boolean }).turnFails === false))
    expect(puts.map((put) => put.events)).toEqual([
      { turnFinishes: true, turnFails: true, needsPermission: true, agentNotifies: true },
      { turnFinishes: true, turnFails: false, needsPermission: true, agentNotifies: true },
    ])
  })
})

import { expect, test } from 'bun:test'
import { ApiError } from '../api/client'
import { needsHomeScreen, subscribeFor, type BrowserSubscription, type Subscribing } from './notifications'

// Which browser shows how to add Demi to the Home Screen instead of asking
// for notifications (`turns.md` § Notifications). Pure; under a millisecond.

test('Safari on an iPhone or iPad outside the Home Screen needs Demi added there; the Home Screen app and other browsers do not', () => {
  expect(needsHomeScreen({ standalone: false })).toBe(true)
  expect(needsHomeScreen({ standalone: true })).toBe(false)
  expect(needsHomeScreen({})).toBe(false)
})

// A browser's subscription serves one session (`web-api-settings.md` § Push
// notifications). A stand-in browser and backend; under a millisecond.

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

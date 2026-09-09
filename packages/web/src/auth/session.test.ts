import { afterEach, beforeEach, describe, expect, test } from 'bun:test'
import { createPinia, setActivePinia } from 'pinia'
import { ApiError } from '../api/client'
import { useSession } from './session'

const user = {
  id: 'user-1',
  email: 'person@example.com',
  nickname: 'Person',
  role: 'user' as const,
  createdAt: '2026-09-09T00:00:00.000Z',
}
const originalFetch = globalThis.fetch

beforeEach(() => setActivePinia(createPinia()))
afterEach(() => {
  globalThis.fetch = originalFetch
})

function respond(body: unknown, status = 200): void {
  globalThis.fetch = (async () => Response.json(body, { status })) as unknown as typeof fetch
}

describe('cookie session', () => {
  test('restores the server identity and uses same-origin credentials', async () => {
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      expect(input).toBe('/api/auth/me')
      expect(init?.credentials).toBe('same-origin')
      expect(init?.cache).toBe('no-store')
      return Response.json({ user })
    }) as unknown as typeof fetch
    const session = useSession()
    expect(session.signedIn).toBe(false)
    await session.restore()
    expect(session.user).toEqual(user)
  })

  test('a missing cookie is signed out; an ended session is expired', async () => {
    const session = useSession()
    respond({ code: 'unauthenticated', message: 'Sign in first' }, 401)
    await session.restore()
    expect(session.current).toEqual({ status: 'signedOut', reason: undefined })
    respond({ user })
    await session.signIn(user.email, 'secret', new AbortController().signal)
    respond({ code: 'unauthenticated', message: 'Sign in first' }, 401)
    await session.restore()
    expect(session.current).toEqual({ status: 'signedOut', reason: 'expired' })
  })

  test('posts credentials and reports backend rate limiting', async () => {
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      expect(input).toBe('/api/auth/login')
      expect(init?.method).toBe('POST')
      expect(JSON.parse(String(init?.body))).toEqual({
        email: user.email,
        password: 'secret',
      })
      return Response.json({
        code: 'too_many_attempts',
        message: 'Try again in a minute',
      }, { status: 429 })
    }) as unknown as typeof fetch
    const session = useSession()
    await expect(session.signIn(
      user.email, 'secret', new AbortController().signal,
    )).rejects.toMatchObject({
      status: 429,
      code: 'too_many_attempts',
      message: 'Try again in a minute',
    })
    expect(session.signedIn).toBe(false)
  })

  test('invalid identity never authenticates and cancellation ignores late success', async () => {
    const session = useSession()
    respond({ user: { ...user, role: 'root' } })
    await expect(session.signIn(
      user.email, 'secret', new AbortController().signal,
    )).rejects.toThrow('invalid account response')
    expect(session.signedIn).toBe(false)
    respond({ user })
    const controller = new AbortController()
    const request = session.signIn(user.email, 'secret', controller.signal)
    controller.abort()
    await expect(request).rejects.toThrow()
    expect(session.signedIn).toBe(false)
  })

  test('failed logout preserves identity; successful logout removes it', async () => {
    const session = useSession()
    respond({ user })
    await session.restore()
    respond({ code: 'unavailable', message: 'Unavailable' }, 503)
    await expect(session.signOut()).rejects.toBeInstanceOf(ApiError)
    expect(session.user).toEqual(user)
    globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      expect(input).toBe('/api/auth/logout')
      expect(init?.method).toBe('POST')
      return new Response(null, { status: 204 })
    }) as unknown as typeof fetch
    await session.signOut()
    expect(session.current).toEqual({ status: 'signedOut' })
    expect(session.user).toBeNull()
  })

  test('outages do not expire an established session', async () => {
    const session = useSession()
    respond({ user })
    await session.restore()
    respond({ code: 'unavailable', message: 'Unavailable' }, 503)
    await expect(session.restore()).rejects.toBeInstanceOf(ApiError)
    expect(session.user).toEqual(user)
  })
})

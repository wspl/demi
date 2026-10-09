import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { waitFor } from '@demicodes/utils'
import { pageReturned } from '@demicodes/web-ui/transport/liveness'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useSession } from '../auth/session'
import { useProduct } from '../state/product'
import { ApiError, apiRequest, jsonBody } from './client'

// Every request of the page waits in its one HTTP client while the backend
// cannot be reached, and goes once the page reaches it again; only the
// backend's own answer fails (`web-application.md` § A page of another
// build, § Page synchronization). No timer: the page's return ends each
// wait, as it does in the product. Cost: a few milliseconds each.

const realFetch = globalThis.fetch
const user = {
  id: 'user-1',
  email: 'person@example.com',
  nickname: 'Person',
  role: 'user' as const,
  createdAt: '2026-09-09T00:00:00.000Z',
}
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** Whether the backend is away: a proxy in front of it answers a bare 502, as Vite's does. */
let away: boolean
/** The requests a proxy answered for the backend that was away. */
let refused: string[]
/** The requests the backend answered: their method, path and body. */
let answered: string[]

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  away = false
  refused = []
  answered = []
  globalThis.fetch = (async (input, init) => {
    const request = `${init?.method ?? 'GET'} ${String(input)}`
    if (request.startsWith('GET /api/models')) {
      // The catalog each snapshot loads, which these tests do not follow.
      return Response.json({ providers: [] })
    }
    if (away) {
      refused.push(request)
      return new Response('Bad Gateway', { status: 502 })
    }
    answered.push(`${request} ${init?.body ?? ''}`.trim())
    if (request === 'GET /api/auth/me') {
      return Response.json({ user })
    }
    if (request === 'PATCH /api/conversations/c-1') {
      return new Response(null, { status: 204 })
    }
    if (request === 'POST /api/conversations/c-1/fork') {
      return Response.json({ code: 'fork_conflict', message: 'Another fork is on its way' }, { status: 409 })
    }
    throw new Error(`Unexpected request: ${request}`)
  }) as typeof fetch
  channels = playChannels()
})

afterEach(() => {
  globalThis.fetch = realFetch
  useProduct().stop()
  disposePinia(pinia)
  channels.restore()
})

/** The page follows the state, and its channel brings the backend's first. */
function connected(): void {
  useProduct().start()
  channels.last().connect(productState())
}

/** The backend goes away as it restarts: it closes the channel saying so. */
function restarting(): void {
  away = true
  channels.last().end(1001, 'backend_closing')
}

/** The backend is back: the page returns at once, and its channel brings the snapshot. */
function back(): void {
  away = false
  pageReturned()
  channels.last().connect(productState())
}

test('a write the page sends while the backend restarts goes once, when it is back', async () => {
  connected()
  restarting()
  const rename = apiRequest('/conversations/c-1', { method: 'PATCH', ...jsonBody({ title: 'Renamed' }) })
  await waitFor(() => refused.length === 1, () => 'the rename was not tried')
  back()
  expect((await rename).status).toBe(204)
  expect(answered).toEqual(['PATCH /api/conversations/c-1 {"title":"Renamed"}'])
})

test("the backend's own answer rejects at once, even while the banner shows", async () => {
  connected()
  channels.last().end(1001, 'backend_closing')
  expect(useProduct().connection).toBe('restarting')
  const fork = apiRequest('/conversations/c-1/fork', { method: 'POST' })
  await expect(fork).rejects.toThrow(ApiError)
  expect(answered).toEqual(['POST /api/conversations/c-1/fork'])
})

test('a waiting request whose caller leaves rejects with the abort and is never sent', async () => {
  connected()
  restarting()
  const caller = new AbortController()
  const rename = apiRequest('/conversations/c-1', { method: 'PATCH', signal: caller.signal, ...jsonBody({ title: 'Left' }) })
  await waitFor(() => refused.length === 1, () => 'the rename was not tried')
  const reason = new Error('The caller left')
  caller.abort(reason)
  await expect(rename).rejects.toBe(reason)
  back()
  // A request sent now reaches the backend, and the one dropped is not among what it got.
  await apiRequest('/auth/me')
  expect(answered).toEqual(['GET /api/auth/me'])
})

test('a page loaded while the backend cannot be reached connects, never signs out, and signs in once it is back', async () => {
  const session = useSession()
  const product = useProduct()
  away = true
  // As the page starts: the channel beside the session check, and neither reaches the backend.
  product.start()
  const restored = session.restore()
  channels.last().end()
  await waitFor(() => refused.includes('GET /api/auth/me'), () => 'the session was not checked')
  expect(session.current.status).toBe('checking')
  expect(product.connecting).toBe(true)
  expect(product.load).toBe('loading')
  back()
  await restored
  expect(session.user).toEqual(user)
  expect(product.connecting).toBe(false)
})

test('an unchanged answer is read to its end, so the browser counts it answered, not abandoned', async () => {
  let answer: Response | null = null
  // The stub has fetch's shape; it answers every request the same.
  globalThis.fetch = (async (_input, _init) => {
    // As Chrome gives a fetch its 304: an empty body still to read.
    answer = new Response(new ReadableStream({ start: (stream) => stream.close() }), {
      status: 304,
      headers: { etag: '"v1"' },
    })
    return answer
  }) as typeof fetch
  const response = await apiRequest('/conversations/c1/changes', { headers: { 'if-none-match': '"v1"' }, allowNotModified: true })
  expect(response.status).toBe(304)
  expect(response.headers.get('etag')).toBe('"v1"')
  expect(answer!.bodyUsed).toBe(true)
})

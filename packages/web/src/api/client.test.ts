import { afterEach, beforeEach, expect, test } from 'bun:test'
import { createPinia, disposePinia, setActivePinia } from 'pinia'
import { waitFor } from '@demicodes/utils'
import { pageReturned } from '@demicodes/web-ui/transport/liveness'
import { productState } from '../__tests__/product-state'
import { playChannels } from '../__tests__/sync-channel'
import { useSession } from '../auth/session'
import { useProduct } from '../state/product'
import { createPageI18n } from '@demicodes/web-ui/i18n/text'
import { formError, shownError } from '@demicodes/web-ui/infra/errors'
import { CODE_CORRECTIONS, EMAIL_CORRECTIONS } from '../settings/corrections'
import { ApiError, apiError, apiRequest, jsonBody } from './client'

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
  disabled: false,
  choosesPassword: false,
}
let pinia: ReturnType<typeof createPinia>
let channels: ReturnType<typeof playChannels>
/** Whether the backend is away: a proxy in front of it answers for it. */
let away: boolean
/** What that proxy answers: a bare 502, as Vite's does, unless a test says otherwise. */
let proxyAnswer: () => Response
/** The requests a proxy answered for the backend that was away. */
let refused: string[]
/** The requests the backend answered: their method, path and body. */
let answered: string[]

beforeEach(() => {
  pinia = createPinia()
  setActivePinia(pinia)
  away = false
  proxyAnswer = () => new Response('Bad Gateway', { status: 502 })
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
      return proxyAnswer()
    }
    answered.push(`${request} ${init?.body ?? ''}`.trim())
    if (request === 'GET /api/auth/me') {
      return Response.json({ user })
    }
    if (request === 'PATCH /api/conversations/c-1') {
      return new Response(null, { status: 204 })
    }
    if (request === 'POST /api/conversations/c-1/read') {
      // The conversation's output is at revision 3.
      const { revision } = JSON.parse(String(init?.body)) as { revision: number }
      return revision <= 3
        ? new Response(null, { status: 204 })
        : Response.json({ requestId: 'req_test000000000002', code: 'invalid_revision', message: 'Cannot read beyond current output' }, { status: 409 })
    }
    if (request === 'POST /api/conversations/c-1/fork') {
      return Response.json({ requestId: 'req_test000000000001', code: 'fork_conflict', message: 'Another fork is on its way' }, { status: 409 })
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
async function back(): Promise<void> {
  away = false
  await pageReturned()
  channels.last().connect(productState())
}

test('a write the page sends while the backend restarts goes once, when it is back', async () => {
  connected()
  restarting()
  const rename = apiRequest('/conversations/c-1', { method: 'PATCH', ...jsonBody({ title: 'Renamed' }) })
  await waitFor(() => refused.length === 1, () => 'the rename was not tried')
  await back()
  expect((await rename).status).toBe(204)
  expect(answered).toEqual(['PATCH /api/conversations/c-1 {"title":"Renamed"}'])
})

test("a read acknowledgement Cloudflare answers 520 for, as the origin gave it nothing, goes again once the backend is reached", async () => {
  // req_h8fjy63f18930c5m: the origin's connection lost packets, Cloudflare
  // answered its own 520 page, and the page showed "Could Not Update Read
  // Status" for what was only a connection failure.
  connected()
  away = true
  proxyAnswer = () => new Response('<!DOCTYPE html><title>Web server is returning an unknown error</title>', { status: 520, headers: { 'Content-Type': 'text/html' } })
  const read = apiRequest('/conversations/c-1/read', { method: 'POST', ...jsonBody({ revision: 3 }) })
  await waitFor(() => refused.length === 1, () => 'the read was not tried')
  channels.last().end()
  await back()
  expect((await read).status).toBe(204)
  expect(answered).toEqual(['POST /api/conversations/c-1/read {"revision":3}'])
})

test('the backend refusing a read acknowledgement rejects with its answer', async () => {
  connected()
  const read = apiRequest('/conversations/c-1/read', { method: 'POST', ...jsonBody({ revision: 4 }) })
  await expect(read).rejects.toMatchObject({ status: 409, code: 'invalid_revision' })
  expect(answered).toEqual(['POST /api/conversations/c-1/read {"revision":4}'])
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
  await back()
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
  await back()
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

// A refusal shows the catalogue's text for its code, filled with its
// params, in the page's language; a code the catalogue has no text for, as
// a newer backend may send, shows the backend's own message
// (`localization.md` § Text from the backend). No I/O.
test('a refusal is worded from its code and params, and from its message for a code the page does not know', () => {
  const full = JSON.stringify({ code: 'saved_full', message: 'The conversation keeps 200 messages saved for later', params: { limit: 200 }, requestId: 'req_full000000000001' })
  const unknown = JSON.stringify({ code: 'invalid_query', message: 'Unknown query parameter refresh', requestId: 'req_query00000000001' })
  try {
    createPageI18n('en')
    expect(shownError(apiError(409, full)).message).toBe('A conversation keeps up to 200 messages saved for later.')
    createPageI18n('zh-Hans')
    expect(shownError(apiError(409, full)).message).toBe('一个对话最多保留 200 条稍后发送的消息。')
    expect(shownError(apiError(400, unknown))).toEqual({ message: 'Unknown query parameter refresh', requestId: 'req_query00000000001' })
  } finally {
    createPageI18n('en')
  }
})

// A newer backend may refuse with a code this page does not know: the
// refusal keeps its code, says its message and names its request.
test('a refusal with a code the page does not know keeps its message and request', () => {
  const body = JSON.stringify({ code: 'from_a_newer_backend', message: 'The workspace is sealed', requestId: 'req_newer0000000001' })
  const refused = apiError(409, body)
  expect([refused.code, refused.message, refused.requestId]).toEqual(['from_a_newer_backend', 'The workspace is sealed', 'req_newer0000000001'])
  expect(shownError(refused)).toEqual({ message: 'The workspace is sealed', requestId: 'req_newer0000000001' })
})

// Changing the account's email to a taken address: the form says the
// catalogue's words in the page's language, and names no request, since the
// user corrects it there.
test('the email form says a taken address in the catalogue’s words without a request', () => {
  const taken = JSON.stringify({ code: 'email_taken', message: 'The email address is taken', requestId: 'req_taken0000000001' })
  try {
    createPageI18n('zh-Hans')
    for (const corrections of [EMAIL_CORRECTIONS, CODE_CORRECTIONS]) {
      expect(formError(apiError(409, taken), corrections)).toEqual({ message: '另一个帐户已使用此电子邮件地址。', requestId: null })
    }
  } finally {
    createPageI18n('en')
  }
})

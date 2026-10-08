import { afterEach, expect, jest, test } from 'bun:test'
import type { PreviewPlace, UserStreamHandlers } from '@demicodes/plugin-sdk'
import {
  PREVIEW_BODY_CHUNK_BYTES,
  PREVIEW_BODY_WINDOW,
  PREVIEW_CHUNK_FRAME,
  PREVIEW_CONTROL_FRAME,
  PREVIEW_REQUEST_BODY_FRAME,
  type PreviewEngineMessage,
  type PreviewEnvironment,
  type PreviewRelayMessage,
} from '../../generated/plugin'
import { PreviewConnection } from '../connection'
import { PreviewFrameReader, encodeMessage } from '../frames'
import { PreviewRelay, type RelayTab, type TabEvent } from '../relay'

// The relay's rules (`preview.md` § The forwarder and the relay) over a
// scripted engine on its stream and scripted preview windows: which channel
// it binds to which label and tab, the real addresses and environments it
// sends, the labels it keeps, the requests it keeps behind tokens, the order
// of a document's cookie write, and the runtime it delivers. Labels are the
// engine's: the scripted engine names them from a table. No DOM: windows are
// objects with a parent. Each test takes a few milliseconds.

const PLACE: PreviewPlace = {
  scheme: 'https',
  domain: 'demi-preview.dev',
  namespace: 'k3f9x2ab',
  host: 'host-1',
  runtime: { release: '0.1.18', url: '/runtime/0.1.18.js' },
}
const APP: PreviewEnvironment = { origin: 'http://localhost:5173', top: 'http://localhost', cross: false }
const OTHER: PreviewEnvironment = { origin: 'https://other.test', top: 'https://other.test', cross: false }
const encoder = new TextEncoder()
const decoder = new TextDecoder()

/** The labels the scripted engine computes, as the real one would for these environments. */
const APP_LABEL = 'selbnt2qp6d94in3'
const OTHER_LABEL = '5h8v0c2kq7m1p3ra'
const ENGINE_LABELS = new Map([[APP.origin, APP_LABEL], [OTHER.origin, OTHER_LABEL]])

/** The preview origin of `label` in `PLACE`. */
function origin(label: string): string {
  return `https://k3f9x2ab--${label}.demi-preview.dev`
}

/** What the scripted engine received of one request, and how it answers. */
interface Received {
  message: Extract<PreviewRelayMessage, { type: 'request' }>
  body: Uint8Array
}

/** An engine on the stream: it answers each request as `answer` says, its body a window ahead of the pulls. */
type Answer = { status: number; headers?: [string, string][]; labels?: Record<string, PreviewEnvironment>; body?: string } | { failed: string }

function scriptedEngine(answer: (received: Received) => Answer | Promise<Answer>) {
  const received: Received[] = []
  const said: PreviewRelayMessage[] = []
  const kept: string[] = []
  let handlers: UserStreamHandlers | null = null
  const bodies = new Map<number, Uint8Array[]>()
  const answers = new Map<number, Uint8Array>()
  const send = (message: PreviewEngineMessage) => handlers?.data(encodeEngine(message))
  const open = (opened: UserStreamHandlers) => {
    handlers = opened
    let pending = new Uint8Array(0)
    return {
      send(bytes: Uint8Array) {
        const joined = new Uint8Array(pending.length + bytes.length)
        joined.set(pending)
        joined.set(bytes, pending.length)
        pending = joined
        while (pending.length >= 4) {
          const length = new DataView(pending.buffer, pending.byteOffset).getUint32(0)
          if (pending.length < 4 + length) {
            break
          }
          const frame = pending.subarray(4, 4 + length)
          pending = pending.slice(4 + length)
          if (frame[0] === PREVIEW_CONTROL_FRAME) {
            control(JSON.parse(decoder.decode(frame.subarray(1))) as PreviewRelayMessage)
          } else if (frame[0] === PREVIEW_REQUEST_BODY_FRAME) {
            const id = new DataView(frame.buffer, frame.byteOffset + 1).getUint32(0)
            const data = frame.slice(5)
            if (data.length > 0) {
              bodies.get(id)!.push(data)
              send({ type: 'pull', id })
            } else {
              respond(id)
            }
          }
        }
      },
      close() {
        handlers = null
      },
    }
  }
  const requests = new Map<number, Received['message']>()
  function respond(id: number) {
    const message = requests.get(id)!
    const parts = bodies.get(id) ?? []
    const body = new Uint8Array(parts.reduce((total, part) => total + part.length, 0))
    let at = 0
    for (const part of parts) {
      body.set(part, at)
      at += part.length
    }
    const entry = { message, body }
    received.push(entry)
    void Promise.resolve(answer(entry)).then((answered) => answerWith(id, answered))
  }
  function answerWith(id: number, answered: Answer) {
    if ('failed' in answered) {
      send({ type: 'failed', id, reason: answered.failed })
      return
    }
    answers.set(id, encoder.encode(answered.body ?? ''))
    send({
      type: 'response',
      id,
      status: answered.status,
      headers: (answered.headers ?? []).map(([name, value]) => ({ name, value })),
      labels: answered.labels ?? {},
    })
    for (let sent = 0; sent < PREVIEW_BODY_WINDOW; sent++) {
      if (!sendChunk(id)) {
        break
      }
    }
  }
  /** The answer's next chunk; false once its end went. */
  function sendChunk(id: number): boolean {
    const rest = answers.get(id)
    if (rest === undefined) {
      return false
    }
    const chunk = rest.subarray(0, PREVIEW_BODY_CHUNK_BYTES)
    if (chunk.length === 0) {
      answers.delete(id)
    } else {
      answers.set(id, rest.subarray(chunk.length))
    }
    handlers?.data(encodeChunk(id, chunk))
    return chunk.length > 0
  }
  function control(message: PreviewRelayMessage) {
    said.push(message)
    if (message.type === 'request') {
      requests.set(message.id, message)
      if (message.request.body) {
        bodies.set(message.id, [])
        send({ type: 'pull', id: message.id })
      } else {
        respond(message.id)
      }
    } else if (message.type === 'state_take') {
      send(message.tab === 't1'
        ? { type: 'state', id: message.id, url: 'http://localhost:5173/', title: 'App', mobile: true, storage: null, too_large: true }
        : { type: 'failed', id: message.id, reason: 'The agent’s tab is gone.' })
    } else if (message.type === 'state_keep') {
      kept.push(message.token)
      send({ type: 'state_kept', id: message.id })
    } else if (message.type === 'labels') {
      const labels = Object.fromEntries(message.environments.map((environment) => [ENGINE_LABELS.get(environment.origin)!, environment]))
      send({ type: 'labels', id: message.id, labels })
    } else if (message.type === 'pull') {
      sendChunk(message.id)
    }
  }
  return { open, received, said, kept }
}

function encodeEngine(message: PreviewEngineMessage): Uint8Array {
  const json = encoder.encode(JSON.stringify(message))
  const bytes = new Uint8Array(5 + json.length)
  new DataView(bytes.buffer).setUint32(0, 1 + json.length)
  bytes[4] = PREVIEW_CONTROL_FRAME
  bytes.set(json, 5)
  return bytes
}

function encodeChunk(id: number, data: Uint8Array): Uint8Array {
  const bytes = new Uint8Array(9 + data.length)
  const view = new DataView(bytes.buffer)
  view.setUint32(0, 5 + data.length)
  bytes[4] = PREVIEW_CHUNK_FRAME
  view.setUint32(5, id)
  bytes.set(data, 9)
  return bytes
}

/** The Demi page, a tab of the user's browser in it, and the windows of that tab's frame. */
function world(engine: ReturnType<typeof scriptedEngine>, runtime = async () => encoder.encode('/* runtime */').buffer) {
  const page = { addEventListener() {} }
  const top = { parent: page }
  const nested = { parent: top }
  const events: TabEvent[] = []
  const navigations: unknown[] = []
  const relay = new PreviewRelay(page, runtime, client)
  const tab: RelayTab = {
    id: 'p1',
    place: () => PLACE,
    frameWindow: () => top,
    mobile: () => false,
    connection: new PreviewConnection(engine.open),
    report: (event) => events.push(event),
    openWindow: () => {},
    navigate: (navigation) => navigations.push(navigation),
    close: () => {},
    opener: null,
  }
  relay.register(tab)
  return { relay, tab, top, nested, events, navigations }
}

/** The channel the relay gives the window `source` of `origin` that asks for one; null when it gives none. */
async function connect(
  relay: PreviewRelay,
  origin: string,
  source: unknown,
  purpose: 'forwarder' | 'document' = 'forwarder',
): Promise<MessagePort | null> {
  const reply = new MessageChannel()
  let given: MessagePort | null = null
  reply.port1.onmessage = (event) => {
    given = event.ports[0] ?? null
  }
  await relay.receive({ data: { type: 'demi-preview-connect', purpose }, origin, source, ports: [reply.port2] })
  // The reply crosses a channel: one turn of the event loop delivers it.
  await new Promise((resolve) => setTimeout(resolve, 0))
  reply.port1.close()
  return given
}

/** A forwarder's request, as `sw.js` sends it. */
function fetchMessage(id: number, url: string, extra: Record<string, unknown> = {}) {
  return {
    type: 'fetch',
    id,
    url,
    method: 'GET',
    headers: [['accept', '*/*']],
    body: null,
    mode: 'no-cors',
    destination: 'image',
    credentials: 'include',
    redirect: 'follow',
    referrer: '',
    referrerPolicy: '',
    token: null,
    announcedReferrer: null,
    ...extra,
  }
}

/** The head the relay answers a forwarder's request with, then its body as text. */
async function exchange(port: MessagePort, message: ReturnType<typeof fetchMessage>): Promise<{ head: Record<string, unknown>; body: string | null }> {
  const head = await new Promise<Record<string, unknown>>((resolve) => {
    port.onmessage = (event) => resolve(event.data as Record<string, unknown>)
    port.postMessage(message)
  })
  if (head['error'] || !head['body']) {
    return { head, body: null }
  }
  let body = ''
  for (;;) {
    const next = await new Promise<Record<string, unknown>>((resolve) => {
      port.onmessage = (event) => resolve(event.data as Record<string, unknown>)
      port.postMessage({ type: 'pull', id: message.id })
    })
    if (next['type'] === 'end') {
      return { head, body }
    }
    body += decoder.decode(new Uint8Array(next['bytes'] as ArrayBuffer))
  }
}

afterEach(() => {
  jest.useRealTimers()
})

test('a channel binds to the label of the origin that asked, in the tab whose frame holds the window', async () => {
  const engine = scriptedEngine(() => ({ status: 200 }))
  const { relay, top, nested } = world(engine)
  relay.know(PLACE, { [APP_LABEL]: APP })
  const app = origin(APP_LABEL)
  // The tab's top frame, and a frame inside it.
  expect(await connect(relay, app, top)).not.toBeNull()
  expect(await connect(relay, app, nested)).not.toBeNull()
  // Another namespace's origin, a site's own, and a window no tab holds get nothing.
  expect(await connect(relay, app.replace('k3f9x2ab', 'z9z9z9z9'), top)).toBeNull()
  expect(await connect(relay, 'https://evil.test', top)).toBeNull()
  expect(await connect(relay, app, { parent: { parent: null } })).toBeNull()
})

test('a tab in Mobile describes an Android Chrome of the user’s version to the Host', async () => {
  const engine = scriptedEngine(() => ({ status: 200 }))
  const { relay, tab, top } = world(engine)
  let mobile = false
  tab.mobile = () => mobile
  relay.know(PLACE, { [APP_LABEL]: APP })
  const port = (await connect(relay, origin(APP_LABEL), top))!
  await exchange(port, fetchMessage(1, `${origin(APP_LABEL)}/`))
  mobile = true
  await exchange(port, fetchMessage(2, `${origin(APP_LABEL)}/`))
  expect(engine.said.filter((message) => message.type === 'request').map((message) => message.client)).toEqual([
    client(),
    {
      ...client(),
      userAgent: 'Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Mobile Safari/537.36',
      mobile: true,
      platform: 'Android',
    },
  ])
})

test('a runtime’s registration keeps only the labels the engine names for its environments', async () => {
  const engine = scriptedEngine(() => ({ status: 200 }))
  const { relay, top } = world(engine)
  relay.know(PLACE, { [APP_LABEL]: APP })
  const port = (await connect(relay, origin(APP_LABEL), top, 'document'))!
  // The runtime claims a label of its own for the other site; the engine names another.
  const claimed = '0000000000000000'
  port.postMessage({ type: 'labels', entries: { [claimed]: OTHER } })
  await waitUntil(() => engine.said.some((message) => message.type === 'labels'))
  expect(engine.said.find((message) => message.type === 'labels')).toEqual({ type: 'labels', id: expect.any(Number), environments: [OTHER] })
  expect(await relay.environmentOf(PLACE, OTHER_LABEL)).toEqual(OTHER)
  const unknown = await Promise.race([
    relay.environmentOf(PLACE, claimed),
    new Promise((resolve) => setTimeout(() => resolve('still waiting'), 20)),
  ])
  expect(unknown).toBe('still waiting')
})

test('requests go with real addresses, the receiving environment the channel is bound to, and its initiator', async () => {
  const engine = scriptedEngine(() => ({ status: 200, headers: [['content-type', 'text/plain']], body: 'hello' }))
  const { relay, top } = world(engine)
  const app = APP_LABEL
  const other = OTHER_LABEL
  relay.know(PLACE, { [app]: APP, [other]: OTHER })
  const port = (await connect(relay, origin(app), top))!
  const own = `${origin(app)}/api/items?__demi_integrity=sha256-x#top`
  const answered = await exchange(port, fetchMessage(1, own, {
    referrer: `${origin(app)}/editor`,
    // A page's script writes what it wants in its message: the relay takes the channel's label.
    environment: OTHER,
  }))
  expect(answered).toEqual({ head: expect.objectContaining({ status: 200, body: true }), body: 'hello' })
  const sent = engine.received[0]!.message
  expect(sent.environment).toEqual(APP)
  // The engine's parameters stay, for the engine reads them.
  expect(sent.request.url).toBe('http://localhost:5173/api/items?__demi_integrity=sha256-x#top')
  expect(sent.request.referrer).toBe('http://localhost:5173/editor')
  // A subresource's initiator is the document that receives it.
  expect(sent.request.initiator).toEqual(APP)
  expect(sent.request.user).toBe(false)
  // A navigation names its initiator by its referrer's label; a referrer outside the namespace is unknown.
  await exchange(port, fetchMessage(2, `${origin(app)}/next`, {
    mode: 'navigate',
    destination: 'iframe',
    referrer: `${origin(other)}/from`,
  }))
  expect(engine.received[1]!.message.request.initiator).toEqual(OTHER)
  expect(engine.received[1]!.message.request.referrer).toBe('https://other.test/from')
  await exchange(port, fetchMessage(3, `${origin(app)}/next`, {
    mode: 'navigate',
    destination: 'iframe',
    referrer: 'https://evil.test/',
  }))
  expect(engine.received[2]!.message.request.initiator).toBeNull()
  expect(engine.received[2]!.message.request.referrer).toBe('')
})

test('a request whose label is not registered waits for it, and fails as a network error after ten seconds', async () => {
  const engine = scriptedEngine(() => ({ status: 204 }))
  const { relay, top } = world(engine)
  const app = APP_LABEL
  const other = OTHER_LABEL
  relay.know(PLACE, { [app]: APP })
  const port = (await connect(relay, origin(app), top))!
  const waiting = exchange(port, fetchMessage(1, `${origin(other)}/logo.png`))
  await new Promise((resolve) => setTimeout(resolve, 5))
  expect(engine.received).toHaveLength(0)
  relay.know(PLACE, { [other]: OTHER })
  expect((await waiting).head).toEqual(expect.objectContaining({ status: 204, body: false }))
  expect(engine.received[0]!.message.request.url).toBe('https://other.test/logo.png')

  jest.useFakeTimers()
  const unknown = `${origin(other).replace(other, '0000000000000000')}/x`
  const failing = exchange(port, fetchMessage(2, unknown))
  // The label's wait starts once the message crossed the channel.
  await Promise.resolve()
  await new Promise<void>((resolve) => setImmediate(resolve))
  jest.advanceTimersByTime(10_000)
  expect((await failing).head).toEqual({ type: 'head', id: 2, error: true })
  expect(engine.received).toHaveLength(1)
})

test('a kept request goes only to its target label’s channel, with its method, body and initiator', async () => {
  const engine = scriptedEngine(() => ({ status: 200 }))
  const { relay, top } = world(engine)
  const app = APP_LABEL
  const other = OTHER_LABEL
  relay.know(PLACE, { [app]: APP, [other]: OTHER })
  // A form on the app's page posts to the other site.
  const reply = new MessageChannel()
  const token = new Promise<string>((resolve) => {
    reply.port1.onmessage = (event) => resolve((event.data as { token: string }).token)
  })
  await relay.receive({
    data: {
      type: 'demi-preview-keep',
      request: { method: 'POST', url: `${origin(other)}/login`, contentType: 'application/x-www-form-urlencoded', body: encoder.encode('user=a').buffer },
    },
    origin: origin(app),
    source: top,
    ports: [reply.port2],
  })
  const kept = await token
  // The app's own channel announcing the token takes nothing: its navigation is a GET of its own.
  const appPort = (await connect(relay, origin(app), top))!
  await exchange(appPort, fetchMessage(1, `${origin(app)}/login`, { mode: 'navigate', destination: 'iframe', token: kept }))
  expect(engine.received[0]!.message.request.method).toBe('GET')
  // The target's channel takes it, once.
  const otherPort = (await connect(relay, origin(other), top))!
  const navigation = { mode: 'navigate', destination: 'iframe', token: kept, announcedReferrer: `${origin(app)}/form` }
  await exchange(otherPort, fetchMessage(2, `${origin(other)}/login`, navigation))
  const posted = engine.received[1]!
  expect(posted.message.request.method).toBe('POST')
  expect(decoder.decode(posted.body)).toBe('user=a')
  expect(posted.message.request.headers).toEqual([{ name: 'content-type', value: 'application/x-www-form-urlencoded' }])
  expect(posted.message.request.initiator).toEqual(APP)
  expect(posted.message.environment).toEqual(OTHER)
  await exchange(otherPort, fetchMessage(3, `${origin(other)}/login`, navigation))
  expect(engine.received[2]!.message.request.method).toBe('GET')
})

test('the user’s own opening carries no initiator, and its top frame’s failure reaches the tab', async () => {
  const engine = scriptedEngine((received) => (received.message.request.url.endsWith('/down') ? { failed: 'refused: local-network' } : { status: 200 }))
  const { relay, top, events } = world(engine)
  const app = APP_LABEL
  relay.know(PLACE, { [app]: APP })
  const boot = new URL(relay.bootAddress({ label: app, environment: APP, origin: origin(app) }, { url: 'http://localhost:5173/down', initiator: null }))
  const token = decodeURIComponent(/token=([^&]*)/.exec(boot.hash)![1]!)
  expect(boot.pathname).toBe('/__demi/v1/boot.html')
  const port = (await connect(relay, origin(app), top))!
  const answered = await exchange(port, fetchMessage(1, `${origin(app)}/down`, { mode: 'navigate', destination: 'iframe', token }))
  expect(answered.head).toEqual({ type: 'head', id: 1, error: true })
  expect(engine.received[0]!.message.request.user).toBe(true)
  expect(engine.received[0]!.message.request.initiator).toBeNull()
  expect(events).toEqual([{ type: 'failed', reason: 'refused: local-network' }])
})

test('requests after a document’s cookie write wait for its answer', async () => {
  let release: () => void = () => {}
  // The write's answer is held until released, as a far Host takes its time.
  const held = new Promise<void>((resolve) => {
    release = resolve
  })
  const engine = scriptedEngine(async (received) => {
    if (received.message.request.method === 'POST') {
      await held
    }
    return { status: 200, body: 'a=1' }
  })
  const { relay, top } = world(engine)
  const app = APP_LABEL
  relay.know(PLACE, { [app]: APP })
  const port = (await connect(relay, origin(app), top))!
  const heads = new Map<number, Record<string, unknown>>()
  port.onmessage = (event) => {
    const data = event.data as Record<string, unknown>
    heads.set(data['id'] as number, data)
  }
  const cookie = `${origin(app)}/__demi/host/cookie?url=${encodeURIComponent('http://localhost:5173/')}`
  port.postMessage(fetchMessage(1, cookie, { method: 'POST', mode: 'cors', destination: '', body: encoder.encode('a=1').buffer }))
  port.postMessage(fetchMessage(2, `${origin(app)}/api`, { mode: 'cors', destination: '' }))
  await waitUntil(() => engine.received.length === 1)
  // The next request has not left while the write waits.
  await new Promise((resolve) => setTimeout(resolve, 5))
  expect(engine.received).toHaveLength(1)
  release()
  await waitUntil(() => heads.size === 2)
  expect(engine.received.map((entry) => entry.message.request.url)).toEqual([
    cookie.replace(origin(app), 'http://localhost:5173'),
    'http://localhost:5173/api',
  ])
})

test('the runtime of the engine’s release comes from the web app’s build, and another release’s is a network error', async () => {
  const engine = scriptedEngine(() => ({ status: 200 }))
  let fetched = 0
  const { relay, top } = world(engine, async () => {
    fetched++
    return encoder.encode('/* runtime */').buffer
  })
  const app = APP_LABEL
  relay.know(PLACE, { [app]: APP })
  const port = (await connect(relay, origin(app), top))!
  const runtime = `${origin(app)}/__demi/page/runtime/0.1.18.js`
  const first = await exchange(port, fetchMessage(1, runtime, { destination: 'script' }))
  expect(first.body).toBe('/* runtime */')
  expect(first.head['headers']).toContainEqual(['content-type', 'text/javascript'])
  await exchange(port, fetchMessage(2, runtime, { destination: 'script' }))
  expect(fetched).toBe(1)
  const other = await exchange(port, fetchMessage(3, `${origin(app)}/__demi/page/runtime/0.1.17.js`, { destination: 'script' }))
  expect(other.head).toEqual({ type: 'head', id: 3, error: true })
  expect(engine.received).toHaveLength(0)
})

test('a stream that ends fails what it carried, and the next request opens a new one with hello first', async () => {
  const engine = scriptedEngine(() => ({ status: 200 }))
  let opens = 0
  let handlers: UserStreamHandlers | null = null
  const connection = new PreviewConnection((opened) => {
    opens++
    handlers = opened
    return engine.open(opened)
  })
  const exchange = connection.request(PLACE, APP, request('http://localhost:5173/'), client(), null)
  await expect(exchange.head).resolves.toEqual(expect.objectContaining({ status: 200 }))
  expect(engine.said[0]).toEqual({ type: 'hello', scheme: 'https', domain: 'demi-preview.dev', namespace: 'k3f9x2ab', host: 'host-1' })
  // A frame the protocol refuses ends the stream.
  const waiting = connection.request(PLACE, APP, request('http://localhost:5173/a'), client(), null)
  handlers!.data(new Uint8Array([0, 0, 0, 1, 9]))
  void waiting
  const again = connection.request(PLACE, APP, request('http://localhost:5173/b'), client(), null)
  await expect(again.head).resolves.toEqual(expect.objectContaining({ status: 200 }))
  expect(opens).toBe(2)
  expect(engine.said.filter((message) => message.type === 'hello')).toHaveLength(2)
})

test('page states are questions on the stream: the engine’s answer, or why it failed', async () => {
  const engine = scriptedEngine(() => ({ status: 200 }))
  const connection = new PreviewConnection(engine.open)
  await expect(connection.takeState(PLACE, 't1')).resolves.toEqual({ url: 'http://localhost:5173/', title: 'App', mobile: true, storage: null, tooLarge: true })
  await expect(connection.takeState(PLACE, 't9')).rejects.toThrow('The agent’s tab is gone.')
  await connection.keepState(PLACE, 'kept-1', ['http://localhost:5173'], null)
  expect(engine.kept).toEqual(['kept-1'])
  // A question the stream's end leaves unanswered fails.
  const silent = new PreviewConnection(() => ({ send() {}, close() {} }))
  const waiting = silent.keepState(PLACE, 'kept-2', [], null)
  silent.close()
  await expect(waiting).rejects.toThrow('the panel closed')
})

test('a small answer arrives whole with its head, in one round trip: no read waits for a pull', async () => {
  // The engine sends a window of chunks ahead of any pull, so a page's request over a far relay takes
  // one round trip rather than one per chunk and one more for the end.
  const body = 'small page'
  const engine = scriptedEngine(() => ({ status: 200, body }))
  const connection = new PreviewConnection(engine.open)
  const exchange = connection.request(PLACE, APP, request('http://localhost:5173/'), client(), null)
  await exchange.head
  const pulls = () => engine.said.filter((message) => message.type === 'pull').length
  expect(pulls()).toBe(0)
  const first = await exchange.pull()
  expect(decoder.decode(first!)).toBe(body)
  expect(await exchange.pull()).toBeNull()
  // Each chunk read lets the engine send one more, so the window stays as wide: one pull, for the one chunk.
  expect(pulls()).toBe(1)
})

test('pulls made before their chunks arrive get the body in order, as a stream’s reads do', async () => {
  // A forwarder's stream pulls again while a read still waits; each pull takes the next chunk.
  const body = 'x'.repeat(PREVIEW_BODY_CHUNK_BYTES) + 'tail'
  const engine = scriptedEngine(() => ({ status: 200, body }))
  const connection = new PreviewConnection(engine.open)
  const exchange = connection.request(PLACE, APP, request('http://localhost:5173/big'), client(), null)
  await exchange.head
  const pulls = [exchange.pull(), exchange.pull(), exchange.pull(), exchange.pull()]
  const chunks = await Promise.all(pulls)
  expect(chunks.map((chunk) => (chunk === null ? null : chunk.length))).toEqual([PREVIEW_BODY_CHUNK_BYTES, 4, null, null])
  expect(decoder.decode(chunks[1]!)).toBe('tail')
})

test('a request body goes one chunk per pull, and the frames refuse what the engine never sends', () => {
  const reader = new PreviewFrameReader()
  expect(() => reader.read(new Uint8Array([0, 0, 0, 0]))).toThrow()
  expect(() => new PreviewFrameReader().read(encodeMessage({ type: 'cancel', id: 1 }))).toThrow()
  expect(() => new PreviewFrameReader().read(new Uint8Array([0, 0, 0, 7, 4, 0, 0, 0, 1, 0, 0xff]))).toThrow()
  const big = new Uint8Array(PREVIEW_BODY_CHUNK_BYTES * 2 + 3).fill(7)
  const sent: Uint8Array[] = []
  const connection = new PreviewConnection((handlers) => ({
    send: (bytes) => {
      sent.push(bytes)
      const frame = bytes.subarray(4)
      if (frame[0] === PREVIEW_CONTROL_FRAME && JSON.parse(decoder.decode(frame.subarray(1))).type === 'request') {
        handlers.data(encodeEngine({ type: 'pull', id: 1 }))
      }
    },
    close: () => {},
  }))
  connection.request(PLACE, APP, { ...request('http://localhost:5173/upload'), method: 'POST' }, client(), big)
  const bodies = sent.filter((bytes) => bytes[4] === PREVIEW_REQUEST_BODY_FRAME)
  // One pull, one chunk, at most a frame's worth.
  expect(bodies).toHaveLength(1)
  expect(bodies[0]!.length).toBe(9 + PREVIEW_BODY_CHUNK_BYTES)
})

function request(url: string) {
  return {
    url,
    method: 'GET',
    headers: [],
    mode: 'navigate' as const,
    destination: 'iframe',
    credentials: 'include' as const,
    referrer: '',
    referrerPolicy: '',
    keepalive: false,
    initiator: null,
    user: true,
  }
}

function client() {
  return { userAgent: 'Mozilla/5.0 Chrome/154.0.0.0', brands: '', mobile: false, platform: 'macOS', acceptLanguage: 'en-US' }
}

async function waitUntil(condition: () => boolean): Promise<void> {
  const deadline = Date.now() + 5_000
  while (!condition()) {
    if (Date.now() > deadline) {
      throw new Error('the condition never held')
    }
    await new Promise((resolve) => setTimeout(resolve, 1))
  }
}

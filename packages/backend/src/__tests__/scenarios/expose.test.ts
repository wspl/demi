import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { login } from '../session'
import { model } from './driver'
import { World } from './world'
import { FakeProvisioner } from './fake-provisioner'
import { waitFor } from '@demicodes/utils'

/**
 * The expose scenarios (`expose.md` § Acceptance), one test per item, over a
 * real runner (the paired `laptop` device) and the fake-provisioner Cloud.
 * The fixture service is a plain HTTP server on this machine: the runner
 * connects to it on the device's network, which is the host's here. Every
 * test creates its own expose, so each runs alone.
 */

const FIXTURE_BYTES = 8 * 1024 * 1024

interface Seen {
  method: string
  path: string
  host: string
  headers: Record<string, string>
  bodyLength: number
  bodySha256: string
}

let world!: World
let fake!: FakeProvisioner
let fixture!: ReturnType<typeof startFixture>
let clock = Date.now()

/** The backend's injected expiry clock; every test resets it to the wall. */
function now(): number {
  return clock
}

function backendPort(): string {
  return new URL(world.url).port
}

function relay(host: string, path: string, init?: RequestInit): Promise<Response> {
  return fetch(`http://localhost:${backendPort()}${path}`, {
    ...init,
    headers: { ...(init?.headers as Record<string, string> | undefined), host },
  })
}

function sha256(bytes: Uint8Array): string {
  return createHash('sha256').update(bytes).digest('hex')
}

function bigBytes(): Uint8Array {
  const bytes = new Uint8Array(FIXTURE_BYTES)
  for (let i = 0; i < bytes.length; i++)
    bytes[i] = i % 251
  return bytes
}

/** A fresh expose on the paired device, at the current clock. */
async function pairedExpose(): Promise<{ id: string; host: string }> {
  clock = Date.now()
  const { expose } = await world.api<{ expose: { id: string } }>('/api/exposes', {
    deviceId: world.device('laptop').deviceId,
    address: String(fixture.port),
  })
  return { id: expose.id, host: `${expose.id}.expose.localhost` }
}

/** Wakes the user's Cloud with one turn, then exposes the fixture on it. */
async function cloudExpose(): Promise<{ id: string; host: string; deviceId: string }> {
  clock = Date.now()
  const conversation = await world.conversation('cloud')
  await conversation.turn({ model: [model.shell('wake', 'true'), model.say('done')] })
  const state = await world.api<{ devices: Array<{ id: string; kind: string }> }>('/api/state')
  const deviceId = state.devices.find(device => device.kind === 'managed')!.id
  const { expose } = await world.api<{ expose: { id: string } }>('/api/exposes', {
    deviceId,
    address: String(fixture.port),
  })
  return { id: expose.id, host: `${expose.id}.expose.localhost`, deviceId }
}

function startFixture() {
  let seen: Seen | null = null
  let lastClose: { code: number; reason: string } | null = null
  const server = Bun.serve<URLSearchParams>({
    port: 0,
    idleTimeout: 0,
    fetch: async (request, srv) => {
      const url = new URL(request.url)
      if (url.pathname === '/ws')
        return srv.upgrade(request, { data: url.searchParams }) ? undefined : new Response('no', { status: 400 })
      const headers: Record<string, string> = {}
      request.headers.forEach((value, name) => {
        headers[name] = value
      })
      const body = request.body
        ? new Uint8Array(await new Response(request.body).arrayBuffer())
        : new Uint8Array(0)
      seen = {
        method: request.method,
        path: url.pathname + url.search,
        host: headers['host'] ?? '',
        headers,
        bodyLength: body.byteLength,
        bodySha256: sha256(body),
      }
      if (url.pathname === '/seen')
        return Response.json(seen)
      if (url.pathname === '/hash')
        return new Response(seen.bodySha256)
      if (url.pathname === '/sse') {
        const encode = new TextEncoder()
        return new Response(
          new ReadableStream<Uint8Array>({
            async start(controller) {
              for (let i = 1; i <= 3; i++) {
                await Bun.sleep(30)
                controller.enqueue(encode.encode(`event: tick\ndata: ${i}\n\n`))
              }
              controller.close()
            },
          }),
          { headers: { 'content-type': 'text/event-stream' } }
        )
      }
      if (url.pathname === '/big') {
        const bytes = bigBytes()
        return new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              // Small slices: streaming, not one buffered body.
              for (let at = 0; at < bytes.length; at += 64 * 1024)
                controller.enqueue(bytes.subarray(at, at + 64 * 1024))
              controller.close()
            },
          }),
          { headers: { 'content-type': 'application/octet-stream' } }
        )
      }
      if (url.pathname === '/hold')
        return new Response(
          new ReadableStream<Uint8Array>({
            start(controller) {
              queueMicrotask(() =>
                controller.enqueue(new TextEncoder().encode('held\n'))
              )
              // Never closed: the connection stays open until the relay ends it.
            },
          })
        )
      if (url.pathname === '/lastclose')
        return Response.json(lastClose)
      return new Response('fixture', { status: 404 })
    },
    websocket: {
      open(ws) {
        ws.send('hello')
      },
      message(ws, message) {
        ws.send(message)
        // Only the literal "finish" message asks for the service-side close;
        // everything before it (text and binary) must echo undisturbed.
        const data = ws.data
        if (message === 'finish' && data.get('close'))
          ws.close(Number(data.get('close')), data.get('reason') ?? '')
      },
      close(_ws, code, reason) {
        lastClose = { code, reason: reason ?? '' }
      },
    },
  })
  return {
    server,
    port: server.port,
    seen: () => seen,
    lastClose: () => lastClose,
  }
}

beforeAll(async () => {
  fixture = startFixture()
  fake = new FakeProvisioner()
  clock = Date.now()
  world = await World.create({
    runners: ['laptop'],
    exposeDomain: 'expose.localhost',
    expose: { now, sweepMs: 50, idleTimeoutMs: 2_000 },
    managedHosts: {
      provisioner: fake,
      config: { sweepMs: 100, checkpointIntervalMs: 200 },
    },
  })
})

afterAll(async () => {
  await world?.close()
  await fake?.close()
  fixture?.server.stop(true)
})

describe('expose acceptance', () => {
  test('1: add on a paired device prints a URL; the relay rewrites Host and adds the forwarded headers', async () => {
    const driver = await world.conversation('runner:laptop')
    const turn = await driver.turn({
      model: [
        model.shell('t1', `demi host expose add ${fixture.port}`),
        model.say('done'),
      ],
    })
    const output = turn.received.join('\n')
    expect(output).toContain(`Exposed 127.0.0.1:${fixture.port} on laptop`)
    expect(output).toMatch(/Expires in 60 minutes \(expose \w{26}\)/)
    const id = /https?:\/\/(\w{26})\.expose\.localhost(?::\d+)?\//.exec(output)![1]!
    const response = await relay(`${id}.expose.localhost`, '/seen')
    expect(response.status).toBe(200)
    const seen = fixture.seen()!
    expect(seen.method).toBe('GET')
    expect(seen.host).toBe(`127.0.0.1:${fixture.port}`)
    expect(seen.headers['x-forwarded-host']).toBe(`${id}.expose.localhost`)
    expect(seen.headers['x-forwarded-proto']).toBe('http')
    expect(seen.headers['x-forwarded-for'].length).toBeGreaterThan(0)
    expect(seen.headers['cookie']).toBeUndefined()
  }, 30_000)

  test('1: add on Cloud works too, and a checkpoint keeps the expose while a stop destroys it', async () => {
    const expose = await cloudExpose()
    expect((await relay(expose.host, '/seen')).status).toBe(200)
    // A checkpoint pauses and resumes the machine; exposes survive it.
    await waitFor(
      () => fake.calls.includes(`checkpoint:${expose.deviceId}`),
      () => fake.calls.join('\n'),
      { timeoutMs: 10_000 }
    )
    expect((await relay(expose.host, '/seen')).status).toBe(200)
    let list = await world.api<{ exposes: Array<{ id: string }> }>('/api/exposes')
    expect(list.exposes.map(entry => entry.id)).toContain(expose.id)
    // Leaving the running state — the idle stop here — destroys it.
    await world.backend.managedHosts!.hibernate(expose.deviceId)
    list = await world.api<{ exposes: Array<{ id: string }> }>('/api/exposes')
    expect(list.exposes.map(entry => entry.id)).not.toContain(expose.id)
    expect((await relay(expose.host, '/seen')).status).toBe(404)
  }, 60_000)

  test('2: 8 MiB bodies arrive byte-equal both ways; an event stream reaches the visitor', async () => {
    const { host } = await pairedExpose()
    const bytes = bigBytes()
    const up = await relay(host, '/hash', {
      method: 'POST',
      body: bytes,
      headers: { 'content-type': 'application/octet-stream' },
    })
    expect(await up.text()).toBe(sha256(bytes))
    const down = await relay(host, '/big')
    expect(down.status).toBe(200)
    expect(sha256(new Uint8Array(await down.arrayBuffer()))).toBe(sha256(bytes))
    const sse = await relay(host, '/sse')
    expect(await sse.text())
      .toBe('event: tick\ndata: 1\n\nevent: tick\ndata: 2\n\nevent: tick\ndata: 3\n\n')
  }, 120_000)

  test('3: a WebSocket echo carries text and binary and the close codes both ways', async () => {
    const { host } = await pairedExpose()
    const socket = new WebSocket(
      `ws://localhost:${backendPort()}/ws?close=4001&reason=bye`,
      { headers: { host } } as unknown as ConstructorParameters<typeof WebSocket>[1]
    )
    await new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true })
      socket.addEventListener('error', () => reject(new Error('ws open failed')), { once: true })
    })
    const texts: string[] = []
    socket.addEventListener('message', event => {
      if (typeof event.data === 'string')
        texts.push(event.data)
    })
    const textSeen = async (text: string) => {
      await waitFor(() => texts.includes(text), () => texts.join(','), { timeoutMs: 5_000 })
      return text
    }
    const binarySeen = new Promise<Uint8Array>(resolve =>
      socket.addEventListener('message', event => {
        if (typeof event.data !== 'string')
          resolve(new Uint8Array(event.data as ArrayBuffer))
      })
    )
    const closed = new Promise<{ code: number; reason: string }>(resolve =>
      socket.addEventListener('close', event =>
        resolve({
          code: (event as CloseEvent).code,
          reason: (event as CloseEvent).reason,
        })
      )
    )
    // The service greets on open; the echo answers each message.
    expect(await textSeen('hello')).toBe('hello')
    const binary = crypto.getRandomValues(new Uint8Array(4096))
    socket.send('ping')
    socket.send(binary)
    expect(await textSeen('ping')).toBe('ping')
    expect(Buffer.from(await binarySeen).equals(Buffer.from(binary))).toBe(true)
    // Any further message triggers the service-side close 4001 "bye".
    socket.send('finish')
    expect(await closed).toEqual({ code: 4001, reason: 'bye' })
    // The client-side close code and reason reach the service.
    const second = new WebSocket(`ws://localhost:${backendPort()}/ws`, {
      headers: { host },
    } as unknown as ConstructorParameters<typeof WebSocket>[1])
    await new Promise<void>(resolve =>
      second.addEventListener('open', () => resolve(), { once: true })
    )
    second.close(4321, 'client-done')
    await waitFor(
      () => fixture.lastClose()?.code === 4321,
      () => JSON.stringify(fixture.lastClose()),
      { timeoutMs: 5_000 }
    )
    expect(fixture.lastClose()).toEqual({ code: 4321, reason: 'client-done' })
  }, 30_000)

  test('4: expiry destroys the record; a renewal before it extends the record', async () => {
    const expiring = await pairedExpose()
    clock += 61 * 60_000
    await Bun.sleep(100)
    const gone = await relay(expiring.host, '/seen')
    expect(gone.status).toBe(404)
    const list = await world.api<{ exposes: Array<{ id: string }> }>('/api/exposes')
    expect(list.exposes.map(entry => entry.id)).not.toContain(expiring.id)
    // Renewed before its hour: alive past the original deadline.
    const renewed = await pairedExpose()
    clock += 50 * 60_000
    await world.api(`/api/exposes/${renewed.id}/renew`, undefined, 'POST')
    clock += 50 * 60_000
    expect((await relay(renewed.host, '/seen')).status).toBe(200)
  }, 30_000)

  test('5: a paired device going offline keeps its exposes and answers 502 until reconnect', async () => {
    const { id, host } = await pairedExpose()
    await world.killRunner('laptop')
    await Bun.sleep(100)
    const offline = await relay(host, '/seen')
    expect(offline.status).toBe(502)
    expect(await offline.text()).toContain('device_offline')
    const kept = await world.api<{ exposes: Array<{ id: string }> }>('/api/exposes')
    expect(kept.exposes.map(entry => entry.id)).toContain(id)
    await world.returnRunner('laptop')
    expect((await relay(host, '/seen')).status).toBe(200)
  }, 60_000)

  test('6: another user cannot list, renew or remove; the URL itself needs no session', async () => {
    const { id, host } = await pairedExpose()
    const email = `other-${Date.now()}@example.test`
    await world.api('/api/users', { email, password: 'other-pass-1', role: 'user' })
    const other = await login(world.backend, email, 'other-pass-1')
    expect(await (await other.fetch('/api/exposes')).json()).toEqual({ exposes: [] })
    expect((await other.fetch(`/api/exposes/${id}/renew`, { method: 'POST' })).status).toBe(404)
    expect((await other.fetch(`/api/exposes/${id}`, { method: 'DELETE' })).status).toBe(404)
    // The relay never sees a session: the URL works for an anonymous visitor.
    const anonymous = await fetch(`http://localhost:${backendPort()}/seen`, {
      headers: { host },
    })
    expect(anonymous.status).toBe(200)
  }, 30_000)

  test('7: remove while a connection is open ends that connection', async () => {
    const { id, host } = await pairedExpose()
    const held = await relay(host, '/hold')
    expect(held.status).toBe(200)
    const body = held.body!.getReader()
    expect(new TextDecoder().decode((await body.read()).value)).toBe('held\n')
    await world.api(`/api/exposes/${id}`, undefined, 'DELETE')
    expect((await body.read()).done).toBe(true)
  }, 30_000)

  test('8: the 65th concurrent connection answers 503 and a closed one admits again', async () => {
    const { host } = await pairedExpose()
    const held: Array<ReturnType<ReadableStream<Uint8Array>['getReader']>> = []
    for (let i = 0; i < 64; i++) {
      const response = await relay(host, '/hold')
      expect(response.status).toBe(200)
      // Pull the first byte so the head has arrived and the slot is held.
      const reader = response.body!.getReader()
      await reader.read()
      held.push(reader)
    }
    expect((await relay(host, '/hold')).status).toBe(503)
    await held.pop()!.cancel()
    await Bun.sleep(200)
    const admitted = await relay(host, '/hold')
    expect(admitted.status).toBe(200)
    for (const reader of held)
      await reader.cancel()
    await admitted.body!.cancel()
  }, 60_000)

  test('the idle rule: a visitor that disappears is torn down after the limit and the slot is released', async () => {
    const { host } = await pairedExpose()
    // A held response, first byte read, then the visitor simply stops:
    // no cancellation, no further reads — the abandoned exchange of the
    // limits table (`expose.md` § The public relay).
    const held = await relay(host, '/hold')
    expect(held.status).toBe(200)
    const reader = held.body!.getReader()
    expect(new TextDecoder().decode((await reader.read()).value)).toBe('held\n')
    // No bytes in either direction past this point: the relay closes the
    // connection after its idle limit, and both pipe ends report.
    const before = world.frames.filter(f => f.message.type === 'pipe_done').length
    await waitFor(
      () => world.frames.filter(f => f.message.type === 'pipe_done').length >= before + 2,
      () => 'waiting for the idle teardown to report both pipe ends',
      { timeoutMs: 8_000 }
    )
    // The released slot admits a fresh connection again.
    expect((await relay(host, '/seen')).status).toBe(200)
  }, 30_000)

  test('9: without DEMI_EXPOSE_DOMAIN, add answers expose_unavailable and the state hides the feature', async () => {
    const bare = await World.create({ runners: ['laptop'] })
    try {
      const response = await bare.backend.session.fetch('/api/exposes', {
        method: 'POST',
        body: JSON.stringify({
          deviceId: bare.device('laptop').deviceId,
          address: '1234',
        }),
        headers: { 'content-type': 'application/json' },
      })
      expect(response.status).toBe(409)
      expect(await response.json()).toMatchObject({ code: 'expose_unavailable' })
      const state = await bare.api<{ exposeDomain: string | null; exposes: unknown[] }>('/api/state')
      expect(state.exposeDomain).toBeNull()
      expect(state.exposes).toEqual([])
      const conversation = await bare.conversation('runner:laptop')
      const turn = await conversation.turn({
        model: [model.shell('t9', 'demi host expose add 1234'), model.say('done')],
      })
      expect(turn.received.join('\n')).toContain('expose_unavailable')
    } finally {
      await bare.close()
    }
  }, 60_000)
})

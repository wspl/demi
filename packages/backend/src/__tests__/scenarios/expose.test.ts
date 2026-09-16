import { afterAll, beforeAll, describe, expect, test } from 'bun:test'
import { createHash } from 'node:crypto'
import { login } from '../session'
import { Driver, model } from './driver'
import { World } from './world'

/**
 * The expose scenarios (`expose.md` § Acceptance), one test per item, over a
 * real runner (the paired `laptop` device) and the fake-provisioner Cloud.
 * The fixture service is a plain HTTP server on this machine: the runner
 * connects to it on the device's network, which is the host's here.
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
let fixture!: ReturnType<typeof startFixture>
let clock = Date.now()
let driver!: Driver

function relay(path: string, init?: RequestInit & { headers?: Record<string, string> }, hostname?: string): Promise<Response> {
  const host = hostname ?? fixture.exposeHost!
  return fetch(`http://localhost:${port(world)}${path}`, {
    ...init,
    verbose: true,
    headers: { ...(init?.headers ?? {}), host },
  } as RequestInit & { verbose: boolean })
}

function port(of: World): string {
  return new URL(of.url).port
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
      const body = request.body ? new Uint8Array(await new Response(request.body).arrayBuffer()) : new Uint8Array(0)
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
        const stream = new ReadableStream<Uint8Array>({
          async start(controller) {
            const encode = new TextEncoder()
            for (let i = 1; i <= 3; i++) {
              await Bun.sleep(30)
              controller.enqueue(encode.encode(`event: tick\ndata: ${i}\n\n`))
            }
            controller.close()
          },
        })
        return new Response(stream, { headers: { 'content-type': 'text/event-stream' } })
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
              queueMicrotask(() => controller.enqueue(new TextEncoder().encode('held\n')))
              // Never closed: the connection stays open until the relay ends it.
            },
          })
        )
      if (url.pathname === '/lastclose')
        return Response.json(lastClose)
      return new Response('fixture', { status: 404 })
    },
    websocket: {
      open() {},
      message(ws, message) {
        ws.send(message)
        const data = ws.data as URLSearchParams
        const code = Number(data.get('close'))
        if (code)
          ws.close(code, data.get('reason') ?? '')
      },
      close(ws, code, reason) {
        lastClose = { code, reason: reason ?? '' }
      },
    },
  })
  return {
    server,
    port: server.port,
    seen: () => seen,
    lastClose: () => lastClose,
    exposeHost: null as string | null,
    cloudHost: null as string | null,
    cloudDeviceId: null as string | null,
    exposeId: null as string | null,
  }
}

beforeAll(async () => {
  fixture = startFixture()
  clock = Date.now()
  world = await World.create({
    runners: ['laptop'],
    exposeDomain: 'expose.localhost',
    expose: { now: () => clock, sweepMs: 50 },
  })
  driver = await world.conversation('runner:laptop')
})

afterAll(async () => {
  await world?.close()
  fixture?.server.stop(true)
})

describe('expose acceptance', () => {
  test('1: add prints a URL; the relay rewrites Host and adds the forwarded headers', async () => {
    const turn = await driver.turn({
      model: [
        model.shell('t1', `demi host expose add ${fixture.port}`),
        model.say('done'),
      ],
    })
    const output = turn.received.join('\n')
    expect(output).toContain(`Exposed 127.0.0.1:${fixture.port} on laptop`)
    const url = /https?:\/\/(\w+)\.expose\.localhost\//.exec(output)![1]!
    fixture.exposeHost = `${url}.expose.localhost`
    fixture.exposeId = url
    const response = await relay('/seen')
    expect(response.status).toBe(200)
    const seen = fixture.seen()!
    expect(seen.method).toBe('GET')
    expect(seen.host).toBe(`127.0.0.1:${fixture.port}`)
    expect(seen.headers['x-forwarded-host']).toBe(fixture.exposeHost)
    expect(seen.headers['x-forwarded-proto']).toBe('http')
    expect(seen.headers['x-forwarded-for'].length).toBeGreaterThan(0)
    expect(seen.headers['cookie']).toBeUndefined()
  })

  test('1: add on Cloud prints a URL too', async () => {
    const cloud = await world.conversation('cloud')
    await cloud.turn({ model: [model.shell('t2', 'true'), model.say('done')] })
    const state = await world.api<{ devices: Array<{ id: string; kind: string }> }>('/api/state')
    const cloudDevice = state.devices.find(device => device.kind === 'managed')!
    const { expose } = await world.api<{ expose: { url: string } }>('/api/exposes', {
      deviceId: cloudDevice.id,
      address: String(fixture.port),
    })
    const hostname = new URL(expose.url).hostname
    const response = await relay('/seen', undefined, hostname)
    expect(response.status).toBe(200)
    expect(fixture.seen()!.host).toBe(`127.0.0.1:${fixture.port}`)
    fixture.cloudHost = hostname
    fixture.cloudDeviceId = cloudDevice.id
  })

  test('2: 8 MiB bodies arrive byte-equal both ways; SSE streams', async () => {
    const bytes = bigBytes()
    const up = await relay('/hash', {
      method: 'POST',
      body: bytes,
      headers: { 'content-type': 'application/octet-stream' },
    })
    expect(await up.text()).toBe(sha256(bytes))
    const down = await relay('/big')
    expect(down.status).toBe(200)
    expect(sha256(new Uint8Array(await down.arrayBuffer()))).toBe(sha256(bytes))
    const sse = await relay('/sse')
    expect(await sse.text()).toBe('event: tick\ndata: 1\n\nevent: tick\ndata: 2\n\nevent: tick\ndata: 3\n\n')
  })

  test('3: a WebSocket echo carries text and binary and the close codes both ways', async () => {
    const url = `ws://localhost:${port(world)}/ws?close=4001&reason=bye`
    const socket = new WebSocket(url, { headers: { host: fixture.exposeHost! } } as unknown as ConstructorParameters<typeof WebSocket>[1])
    const opened = new Promise<void>((resolve, reject) => {
      socket.addEventListener('open', () => resolve(), { once: true })
      socket.addEventListener('error', () => reject(new Error('ws open failed')), { once: true })
    })
    await opened
    const binary = crypto.getRandomValues(new Uint8Array(4096))
    const received: string[] = []
    const receivedBinary = new Promise<Uint8Array>(resolve =>
      socket.addEventListener('message', event => {
        if (typeof event.data === 'string')
          received.push(event.data)
        else
          resolve(new Uint8Array(event.data as ArrayBuffer))
      })
    )
    const closed = new Promise<{ code: number; reason: string }>(resolve =>
      socket.addEventListener('close', event =>
        resolve({ code: (event as CloseEvent).code, reason: (event as CloseEvent).reason })
      )
    )
    socket.send('ping')
    socket.send(binary)
    expect(received.at(-1)).toBe('ping')
    expect(Buffer.from(await receivedBinary).equals(Buffer.from(binary))).toBe(true)
    // Any further message triggers the service-side close 4001 "bye".
    socket.send('finish')
    expect(await closed).toEqual({ code: 4001, reason: 'bye' })
    // The client-side close code and reason reach the service.
    const second = new WebSocket(`ws://localhost:${port(world)}/ws`, { headers: { host: fixture.exposeHost! } } as unknown as ConstructorParameters<typeof WebSocket>[1])
    await new Promise<void>(resolve => second.addEventListener('open', () => resolve(), { once: true }))
    second.close(4321, 'client-done')
    await Bun.sleep(200)
    expect(fixture.lastClose()).toEqual({ code: 4321, reason: 'client-done' })
  })

  test('4: expiry destroys the record; renew extends it', async () => {
    const expiring = await world.api<{ expose: { id: string } }>('/api/exposes', {
      deviceId: world.device('laptop').deviceId,
      address: String(fixture.port),
    })
    const host = `${expiring.expose.id}.expose.localhost`
    clock += 61 * 60_000
    await Bun.sleep(100)
    const gone = await relay('/seen', undefined, host)
    expect(gone.status).toBe(404)
    const list = await world.api<{ exposes: unknown[] }>('/api/exposes')
    expect(list.exposes.map((expose) => (expose as { id: string }).id))
      .not.toContain(expiring.expose.id)
    // Renewal before expiry keeps the expose past the original hour.
    clock += 50 * 60_000
    await world.api(`/api/exposes/${fixture.exposeId!}/renew`, undefined, 'POST')
    clock += 50 * 60_000
    const renewed = await relay('/seen')
    expect(renewed.status).toBe(200)
  })

  test('5: a Cloud stop destroys its exposes; a paired device going offline keeps them', async () => {
    await world.backend.managedHosts!.hibernate(fixture.cloudDeviceId!)
    const list = await world.api<{ exposes: Array<{ id: string }> }>('/api/exposes')
    expect(list.exposes.map(expose => expose.id)).not.toContain(
      (fixture.cloudHost!.match(/^(\w+)\./)![1]!)
    )
    expect((await relay('/seen', undefined, fixture.cloudHost!)).status).toBe(404)
    // The paired device keeps its exposes while offline and answers 502.
    await world.killRunner('laptop')
    await Bun.sleep(100)
    const offline = await relay('/seen')
    expect(offline.status).toBe(502)
    expect(await offline.text()).toContain('device_offline')
    const kept = await world.api<{ exposes: unknown[] }>('/api/exposes')
    expect(kept.exposes.length).toBeGreaterThan(0)
    await world.returnRunner('laptop')
    expect((await relay('/seen')).status).toBe(200)
  })

  test('6: another user cannot list, renew or remove; the URL itself needs no session', async () => {
    await world.api('/api/users', {
      email: 'other@example.test',
      password: 'other-pass-1',
      role: 'user'
    })
    const other = await login(world.backend, 'other@example.test', 'other-pass-1')
    const listed = await other.fetch('/api/exposes')
    expect(await (listed as Response).json()).toEqual({ exposes: [] })
    const renew = await other.fetch(`/api/exposes/${fixture.exposeId!}/renew`, { method: 'POST' })
    expect(renew.status).toBe(404)
    const remove = await other.fetch(`/api/exposes/${fixture.exposeId!}`, { method: 'DELETE' })
    expect(remove.status).toBe(404)
    // The relay never saw a session cookie in any test: the URL works anonymous.
    const anonymous = await fetch(`http://localhost:${port(world)}/seen`, {
      headers: { host: fixture.exposeHost! },
    })
    expect(anonymous.status).toBe(200)
  })

  test('7: remove while a connection is open ends that connection', async () => {
    const held = await relay('/hold')
    expect(held.status).toBe(200)
    const body = held.body!.getReader()
    expect(new TextDecoder().decode((await body.read()).value)).toBe('held\n')
    await world.api(`/api/exposes/${fixture.exposeId!}`, undefined, 'DELETE')
    const rest = await body.read()
    expect(rest.done).toBe(true)
  })

  test('8: the 65th concurrent connection answers 503 and a closed one admits again', async () => {
    const add = async () => {
      const { expose } = await world.api<{ expose: { id: string } }>('/api/exposes', {
        deviceId: world.device('laptop').deviceId,
        address: String(fixture.port),
      })
      fixture.exposeId = expose.id
      fixture.exposeHost = `${expose.id}.expose.localhost`
    }
    await add()
    const held: Array<Response> = []
    for (let i = 0; i < 64; i++) {
      const response = await relay('/hold')
      expect(response.status).toBe(200)
      // Pull the first byte so the head has arrived and the slot is held.
      await response.body!.getReader().read()
      held.push(response)
    }
    const refused = await relay('/hold')
    expect(refused.status).toBe(503)
    await held.pop()!.body!.cancel()
    await Bun.sleep(150)
    const admitted = await relay('/hold')
    expect(admitted.status).toBe(200)
    for (const response of held)
      await response.body!.cancel()
    await admitted.body!.cancel()
  })

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
  })
})

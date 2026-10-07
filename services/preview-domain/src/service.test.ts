// The preview domain service through its fetch handler, with namespaces in
// memory, the static files read from this package's `static/` as Cloudflare's
// asset server serves them, and a counting rate limiter. The whole file takes
// about 50 ms.
import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test'
import { join } from 'node:path'
import { NAMESPACE_LIFETIME_MS, createService } from './service'

const STATIC = join(import.meta.dir, '../static')
const DAY_MS = 24 * 60 * 60 * 1000
const ORIGIN = 'https://demi.example.com'

interface Harness {
  handle: (request: Request) => Promise<Response>
  clock: { now: number }
  stored: Map<string, string>
}

function harness(options: { random?: (bytes: Uint8Array<ArrayBuffer>) => Uint8Array<ArrayBuffer> } = {}): Harness {
  const clock = { now: Date.UTC(2026, 9, 1) }
  const stored = new Map<string, string>()
  const created = new Map<string, number>()
  const handle = createService({
    domain: 'demi-preview.dev',
    store: {
      get: async (name) => stored.get(name) ?? null,
      put: async (name, value) => { stored.set(name, value) },
    },
    files: {
      // The asset server answers a file by its path, and 404 for anything else.
      fetch: async (request) => {
        const file = Bun.file(join(STATIC, new URL(request.url).pathname))
        return await file.exists() ? new Response(file) : new Response('Not Found', { status: 404 })
      },
    },
    creationLimiter: {
      limit: async ({ key }) => {
        const count = (created.get(key) ?? 0) + 1
        created.set(key, count)
        return { success: count <= 3 }
      },
    },
    now: () => clock.now,
    random: options.random,
  })
  return { handle, clock, stored }
}

function api(method: string, path: string, init: { body?: unknown, secret?: string, address?: string } = {}): Request {
  const headers = new Headers({ 'cf-connecting-ip': init.address ?? '198.51.100.7' })
  if (init.secret) headers.set('authorization', `Bearer ${init.secret}`)
  if (init.body !== undefined) headers.set('content-type', 'application/json')
  return new Request(`https://demi-preview.dev${path}`, {
    method,
    headers,
    body: init.body === undefined ? undefined : JSON.stringify(init.body),
  })
}

interface Created { namespace: string, secret: string, expiresAt: string }

async function register(handle: Harness['handle'], origins: string[] = [ORIGIN]): Promise<Created> {
  const response = await handle(api('POST', '/api/v1/namespaces', { body: { origins } }))
  expect(response.status).toBe(201)
  return await response.json() as Created
}

/** A request a browser makes on a preview origin of `namespace`. */
function previewRequest(namespace: string, path: string, mode: 'navigate' | 'same-origin' | 'no-cors' = 'no-cors'): Request {
  return new Request(`https://${namespace}--p4q8h2m6c1v9a7b2.demi-preview.dev${path}`, {
    headers: { 'sec-fetch-mode': mode },
  })
}

describe('namespaces', () => {
  test('a registered namespace lets only its origins and preview frames embed it, until it is deleted', async () => {
    const { handle } = harness()
    const created = await register(handle, [ORIGIN, 'http://localhost:5173'])
    expect(created.namespace).toMatch(/^[0-9a-v]{8}$/)
    expect(created.expiresAt).toBe(new Date(Date.UTC(2026, 9, 1) + NAMESPACE_LIFETIME_MS).toISOString())

    const page = await handle(previewRequest(created.namespace, '/__demi/v1/boot.html'))
    expect(page.status).toBe(200)
    expect(page.headers.get('content-security-policy'))
      .toBe('frame-ancestors https://demi.example.com http://localhost:5173 https://*.demi-preview.dev')

    const replaced = await handle(api('PUT', `/api/v1/namespaces/${created.namespace}/origins`, {
      body: { origins: ['https://other.example.com'] },
      secret: created.secret,
    }))
    expect(replaced.status).toBe(200)
    const after = await handle(previewRequest(created.namespace, '/__demi/v1/boot.html'))
    expect(after.headers.get('content-security-policy'))
      .toBe('frame-ancestors https://other.example.com https://*.demi-preview.dev')

    const deleted = await handle(api('DELETE', `/api/v1/namespaces/${created.namespace}`, { secret: created.secret }))
    expect(deleted.status).toBe(204)
    expect((await handle(previewRequest(created.namespace, '/__demi/v1/boot.html'))).status).toBe(410)
    const renewed = await handle(api('POST', `/api/v1/namespaces/${created.namespace}/renew`, { secret: created.secret }))
    expect(renewed.status).toBe(410)
  })

  test('a namespace name is never given out twice, a deleted one included', async () => {
    // The first two namespaces draw the same name bytes; every other draw differs.
    let draw = 0
    const { handle } = harness({
      random: (bytes) => {
        draw += 1
        const repeated = bytes.length === 5 && draw <= 3
        return bytes.fill(repeated ? 7 : draw)
      },
    })
    const first = await register(handle)
    await handle(api('DELETE', `/api/v1/namespaces/${first.namespace}`, { secret: first.secret }))
    const second = await register(handle)
    expect(second.namespace).not.toBe(first.namespace)
  })

  test('changing a namespace takes its own secret', async () => {
    const { handle } = harness()
    const mine = await register(handle)
    const theirs = await register(handle)
    const path = `/api/v1/namespaces/${mine.namespace}/renew`
    for (const secret of [undefined, 'wrong', theirs.secret]) {
      const response = await handle(api('POST', path, { secret }))
      expect(response.status).toBe(401)
      expect(response.headers.get('www-authenticate')).toBe('Bearer')
    }
    const put = await handle(api('PUT', `/api/v1/namespaces/${mine.namespace}/origins`, {
      body: { origins: ['https://evil.example'] },
      secret: theirs.secret,
    }))
    expect(put.status).toBe(401)
    expect((await handle(api('DELETE', `/api/v1/namespaces/${mine.namespace}`, { secret: theirs.secret }))).status).toBe(401)
    expect((await handle(api('POST', path, { secret: mine.secret }))).status).toBe(200)
  })

  test('a namespace expires 90 days after its last renewal, and stays expired', async () => {
    const { handle, clock } = harness()
    const created = await register(handle)
    clock.now += 89 * DAY_MS
    const renewed = await handle(api('POST', `/api/v1/namespaces/${created.namespace}/renew`, { secret: created.secret }))
    expect(renewed.status).toBe(200)
    expect((await renewed.json() as Created).expiresAt).toBe(new Date(clock.now + NAMESPACE_LIFETIME_MS).toISOString())

    clock.now += 89 * DAY_MS
    expect((await handle(previewRequest(created.namespace, '/__demi/v1/boot.html'))).status).toBe(200)
    clock.now += 1 * DAY_MS
    const expired = await handle(previewRequest(created.namespace, '/__demi/v1/boot.html'))
    expect(expired.status).toBe(410)
    expect(expired.headers.get('content-security-policy')).toBe(`frame-ancestors ${ORIGIN} https://*.demi-preview.dev`)
    const late = await handle(api('POST', `/api/v1/namespaces/${created.namespace}/renew`, { secret: created.secret }))
    expect(late.status).toBe(410)
  })

  test('origins are 1 to 8 secure-context origins, written as origins', async () => {
    const { handle } = harness()
    const accepted = [
      ['https://demi.example.com'],
      ['http://localhost:5173', 'http://127.0.0.1:3000', 'https://demi.example.com:8443'],
      Array.from({ length: 8 }, (_, index) => `https://demi${index}.example.com`),
    ]
    const refused: unknown[] = [
      [],
      Array.from({ length: 9 }, (_, index) => `https://demi${index}.example.com`),
      ['http://demi.example.com'],
      ['http://192.168.1.4:3000'],
      ['https://demi.example.com/'],
      ['https://demi.example.com/chat'],
      ['https://*.example.com'],
      ['https://a.example;sandbox'],
      ['HTTPS://DEMI.EXAMPLE.COM'],
      'https://demi.example.com',
      [42],
    ]
    for (const origins of accepted) {
      const response = await handle(api('POST', '/api/v1/namespaces', { body: { origins }, address: `${origins.length}` }))
      expect(response.status).toBe(201)
    }
    const { namespace, secret } = await register(handle)
    for (const origins of refused) {
      const created = await handle(api('POST', '/api/v1/namespaces', { body: { origins }, address: JSON.stringify(origins) }))
      expect(created.status).toBe(400)
      const replaced = await handle(api('PUT', `/api/v1/namespaces/${namespace}/origins`, { body: { origins }, secret }))
      expect(replaced.status).toBe(400)
    }
    const malformed = new Request('https://demi-preview.dev/api/v1/namespaces', {
      method: 'POST',
      headers: { 'cf-connecting-ip': '203.0.113.9' },
      body: '{"origins": [',
    })
    expect((await handle(malformed)).status).toBe(400)
  })

  test('creation is limited per source address', async () => {
    const { handle, stored } = harness()
    for (let count = 0; count < 3; count += 1) await register(handle)
    const limited = await handle(api('POST', '/api/v1/namespaces', { body: { origins: [ORIGIN] } }))
    expect(limited.status).toBe(429)
    expect(stored.size).toBe(3)
    const elsewhere = await handle(api('POST', '/api/v1/namespaces', { body: { origins: [ORIGIN] }, address: '203.0.113.9' }))
    expect(elsewhere.status).toBe(201)
  })
})

describe('preview origins', () => {
  let namespace: string
  let handle: Harness['handle']

  beforeEach(async () => {
    handle = harness().handle
    namespace = (await register(handle)).namespace
  })

  test('the versioned files carry the namespace headers, and the forwarder may control the whole origin', async () => {
    const expected: Record<string, string> = {
      'boot.html': 'text/html; charset=utf-8',
      'boot.js': 'text/javascript; charset=utf-8',
      'client.js': 'text/javascript; charset=utf-8',
      'sw.js': 'text/javascript; charset=utf-8',
      'state.html': 'text/html; charset=utf-8',
      'state.js': 'text/javascript; charset=utf-8',
      'state-frame.js': 'text/javascript; charset=utf-8',
    }
    for (const [file, type] of Object.entries(expected)) {
      const response = await handle(previewRequest(namespace, `/__demi/v1/${file}`, 'same-origin'))
      expect(response.status).toBe(200)
      expect(response.headers.get('content-type')).toBe(type)
      expect(response.headers.get('content-security-policy')).toBe(`frame-ancestors ${ORIGIN} https://*.demi-preview.dev`)
      expect(response.headers.get('referrer-policy')).toBe('same-origin')
      expect(response.headers.get('service-worker-allowed')).toBe(file === 'sw.js' ? '/' : null)
      expect(await response.text()).toBe(await Bun.file(join(STATIC, '__demi/v1', file)).text())
    }
    const policy = await handle(previewRequest(namespace, '/__demi/v1/policy', 'same-origin'))
    expect(policy.headers.get('referrer-policy')).toBe('same-origin')
    expect(await policy.json()).toEqual({ frameAncestors: `${ORIGIN} https://*.demi-preview.dev` })
  })

  describe('a path the forwarder did not take', () => {
    let logged: ReturnType<typeof spyOn>[]
    beforeEach(() => {
      logged = (['log', 'info', 'warn', 'error', 'debug'] as const).map((method) => spyOn(console, method))
    })
    afterEach(() => {
      for (const spy of logged) {
        expect(spy).not.toHaveBeenCalled()
        spy.mockRestore()
      }
    })

    test('gets the boot page for a document navigation, and nothing otherwise', async () => {
      const navigation = await handle(previewRequest(namespace, '/dashboard?tab=1', 'navigate'))
      expect(navigation.status).toBe(200)
      expect(navigation.headers.get('content-type')).toBe('text/html; charset=utf-8')
      expect(navigation.headers.get('content-security-policy')).toBe(`frame-ancestors ${ORIGIN} https://*.demi-preview.dev`)
      expect(await navigation.text()).toBe(await Bun.file(join(STATIC, '__demi/v1/boot.html')).text())

      // A site's own service worker script, a later version's file and anything else get an empty 404.
      for (const path of ['/sw.js', '/__demi/v1/../v1/sw.js.map', '/__demi/v9/sw.js', '/__demi/v9/policy', '/app.js']) {
        const response = await handle(previewRequest(namespace, path, 'same-origin'))
        expect(response.status).toBe(404)
        expect(response.headers.get('content-security-policy')).toBe(`frame-ancestors ${ORIGIN} https://*.demi-preview.dev`)
        expect(response.headers.get('referrer-policy')).toBe('same-origin')
        expect(await response.text()).toBe('')
      }
    })
  })

  test('an unknown namespace or an address off the domain has nothing', async () => {
    expect((await handle(previewRequest('00000000', '/__demi/v1/boot.html'))).status).toBe(404)
    for (const url of [
      `http://${namespace}--p4q8h2m6c1v9a7b2.demi-preview.dev/__demi/v1/boot.html`,
      `https://${namespace}--p4q8h2m6c1v9a7b2.demi-preview.dev:8443/__demi/v1/boot.html`,
      `https://${namespace}--short.demi-preview.dev/__demi/v1/boot.html`,
      `https://${namespace}--p4q8h2m6c1v9a7b2--x.demi-preview.dev/__demi/v1/boot.html`,
      `https://${namespace}--p4q8h2m6c1v9a7b2.demi-preview.dev.evil.example/__demi/v1/boot.html`,
    ]) {
      expect((await handle(new Request(url))).status).toBe(404)
    }
  })
})

test('the root page says what the domain is for and how to report abuse', async () => {
  const { handle } = harness()
  const root = await handle(new Request('https://demi-preview.dev/'))
  expect(root.status).toBe(200)
  expect(await root.text()).toContain('report abuse')
})

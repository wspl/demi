import { afterAll, beforeAll, beforeEach, expect, test } from 'bun:test'

// The page's service worker, loaded as the web browser loads it: a script
// that listens on its global scope. The scope, its caches and the network
// are the test's. The file takes a few milliseconds.

const ORIGIN = 'https://demi.test'

/** One cache of the worker's, by URL. */
class TestCache {
  readonly entries = new Map<string, Response>()

  async match(request: RequestInfo | URL): Promise<Response | undefined> {
    return this.entries.get(urlOf(request))?.clone()
  }

  async put(request: RequestInfo | URL, response: Response): Promise<void> {
    this.entries.set(urlOf(request), new Response(await response.arrayBuffer(), response))
  }

  async delete(request: RequestInfo | URL): Promise<boolean> {
    return this.entries.delete(urlOf(request))
  }

  async keys(): Promise<Request[]> {
    return [...this.entries.keys()].map((url) => new Request(url))
  }
}

/** The origin's caches, as the worker sees `caches`. */
class TestCaches {
  readonly named = new Map<string, TestCache>()

  async open(name: string): Promise<TestCache> {
    const cache = this.named.get(name) ?? new TestCache()
    this.named.set(name, cache)
    return cache
  }

  async keys(): Promise<string[]> {
    return [...this.named.keys()]
  }

  async delete(name: string): Promise<boolean> {
    return this.named.delete(name)
  }

  async match(request: RequestInfo | URL): Promise<Response | undefined> {
    for (const cache of this.named.values()) {
      const found = await cache.match(request)
      if (found) {
        return found
      }
    }
    return undefined
  }
}

function urlOf(request: RequestInfo | URL): string {
  if (typeof request === 'string') {
    return new URL(request, ORIGIN).href
  }
  return request instanceof URL ? request.href : request.url
}

/** A worker's global scope: where it listens, its origin and its clients, the pages `open` names. */
function workerScope() {
  return Object.assign(new EventTarget(), {
    location: { origin: ORIGIN },
    clients: {
      claim: async () => {},
      get: async () => undefined,
      matchAll: async () => [...open].map((id) => ({ id })),
    },
    skipWaiting: async () => {},
  })
}
/** The pages open, by client id. */
const open = new Set<string>()
let scope = workerScope()
/** How many workers the web browser started. */
let starts = 0

/**
 * The web browser starts the worker: its script runs anew, with nothing in
 * memory, on a scope of its own. The caches stay.
 */
async function startWorker(): Promise<void> {
  scope = workerScope()
  Reflect.set(globalThis, 'self', scope)
  starts += 1
  // The worker is a script of its own, which names nothing for the page to
  // import; the query makes each start run it again.
  const worker = `./service-worker.ts?start=${starts}`
  await import(worker)
}
let caches = new TestCaches()
/** What the network answers; null while it is down. */
let network: ((url: string) => Response) | null = null
const real = { self: Reflect.get(globalThis, 'self'), caches: Reflect.get(globalThis, 'caches'), fetch: globalThis.fetch }

beforeAll(async () => {
  Reflect.set(globalThis, 'caches', { open: (name: string) => caches.open(name), keys: () => caches.keys(), delete: (name: string) => caches.delete(name), match: (request: RequestInfo | URL) => caches.match(request) })
  globalThis.fetch = Object.assign(async (input: RequestInfo | URL) => {
    if (!network) {
      throw new TypeError('Failed to fetch')
    }
    return network(urlOf(input))
  }, { preconnect: real.fetch.preconnect })
})

afterAll(() => {
  Reflect.set(globalThis, 'self', real.self)
  Reflect.set(globalThis, 'caches', real.caches)
  globalThis.fetch = real.fetch
})

beforeEach(async () => {
  caches = new TestCaches()
  network = null
  open.clear()
  await startWorker()
})

/** The backend of build `build`: its page at every address of the app, its hashed files, and a share's page. */
function backend(build: string): (url: string) => Response {
  return (url) => {
    const { pathname } = new URL(url)
    if (pathname.startsWith('/assets/')) {
      return new Response(`${build} ${pathname}`, { headers: { 'content-type': 'text/javascript' } })
    }
    const page = pathname.startsWith('/s/') ? 'share' : 'app'
    return new Response(
      `<html><head><meta name="demi-build" content="${build}" /></head><body>${page} of ${build}</body></html>`,
      { headers: { 'content-type': 'text/html' } },
    )
  }
}

/** A request of the page's, as the worker hears it: what it answered, if anything, once its work is done. */
async function request(
  path: string,
  options: { navigate?: boolean; clientId?: string; resultingClientId?: string } = {},
): Promise<string | null> {
  // Set by the worker's handler, if it answers.
  const answered: { response: Promise<Response> | null } = { response: null }
  const work: Promise<unknown>[] = []
  scope.dispatchEvent(Object.assign(new Event('fetch'), {
    request: new Request(new URL(path, ORIGIN), { mode: options.navigate ? 'navigate' : 'cors' }),
    clientId: options.clientId ?? '',
    resultingClientId: options.resultingClientId ?? '',
    respondWith: (response: Promise<Response>) => {
      answered.response = response
    },
    waitUntil: (promise: Promise<unknown>) => void work.push(promise),
  }))
  const text = answered.response === null ? null : await (await answered.response).text()
  await Promise.all(work)
  return text
}

/** A page of `build` tells the worker it runs, as each page does as it starts. */
async function pageRuns(clientId: string, build: string, files: string[] = []): Promise<void> {
  const work: Promise<unknown>[] = []
  scope.dispatchEvent(Object.assign(new Event('message'), {
    data: { type: 'demi-page', build, files },
    source: { id: clientId },
    waitUntil: (promise: Promise<unknown>) => void work.push(promise),
  }))
  await Promise.all(work)
}

test('a navigation that the network or a proxy fails is answered with the page of the build that last ran, never a share’s page', async () => {
  network = backend('b1')
  expect(await request('/chat/first', { navigate: true, resultingClientId: 'page-1' })).toContain('app of b1')
  await pageRuns('page-1', 'b1')

  network = null
  expect(await request('/chat/second', { navigate: true })).toContain('app of b1')
  // A proxy in front of the backend that says it is away fails it the same way.
  for (const status of [502, 503, 504]) {
    network = () => new Response('Bad Gateway', { status })
    expect(await request('/chat/second', { navigate: true })).toContain('app of b1')
  }
  network = null
  // A share's page goes to the network as the worker never saw it.
  expect(await request('/s/token', { navigate: true })).toBeNull()
  // Nor does anything else of the backend come from the cache.
  expect(await request('/build.json')).toBeNull()
  expect(await request('/api/conversations')).toBeNull()

  // A page of a newer build runs: the earlier build goes.
  network = backend('b2')
  expect(await request('/chat/first', { navigate: true, resultingClientId: 'page-2' })).toContain('app of b2')
  await pageRuns('page-2', 'b2')
  network = null
  expect(await request('/', { navigate: true })).toContain('app of b2')
  expect((await caches.keys()).filter((name) => name.startsWith('demi-build-'))).toEqual(['demi-build-b2'])
})

test('the hashed files a page of the build fetched are answered from the cache, those of the first visit included', async () => {
  network = backend('b1')
  await request('/chat', { navigate: true, resultingClientId: 'page-1' })
  expect(await request('/assets/chat-1a2b.js', { clientId: 'page-1' })).toBe('b1 /assets/chat-1a2b.js')
  // A page the worker did not serve, as on the first visit, names the files it fetched before.
  await pageRuns('page-0', 'b1', ['/assets/index-3c4d.js'])
  await pageRuns('page-1', 'b1')

  network = null
  expect(await request('/assets/chat-1a2b.js', { clientId: 'page-3' })).toBe('b1 /assets/chat-1a2b.js')
  expect(await request('/assets/index-3c4d.js', { clientId: 'page-3' })).toBe('b1 /assets/index-3c4d.js')
})

test('a worker the web browser stopped and started again keeps the files a page fetches later', async () => {
  network = backend('b1')
  open.add('page-1')
  await request('/chat', { navigate: true, resultingClientId: 'page-1' })
  await pageRuns('page-1', 'b1')
  // Idle, the worker stops; the page stays open and fetches a language later.
  await startWorker()
  expect(await request('/assets/ja-5e6f.js', { clientId: 'page-1' })).toBe('b1 /assets/ja-5e6f.js')

  network = null
  expect(await request('/assets/ja-5e6f.js', { clientId: 'page-1' })).toBe('b1 /assets/ja-5e6f.js')
})

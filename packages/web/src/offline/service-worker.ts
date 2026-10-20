import {
  OPEN_CONVERSATION,
  WORDS_CACHE,
  WORDS_KEY,
  notificationWordsSchema,
  productPage,
  pushedNotification,
  type NotificationWords,
} from './notifications'

/**
 * The page's one service worker, served from the root so that its scope is the
 * whole origin. It has three jobs:
 *
 * - The direct channel's raw requests
 *   (`packages/web/src/direct/raw-bridge.ts`): a `GET` of a conversation's raw
 *   file route, which an image, a video, a PDF or a download makes, goes to the
 *   page that made it. The page answers it over its direct channel when it has
 *   one to the conversation's device, or sends it to the network.
 * - Opening offline: a navigation of the app goes to the network first and,
 *   only when the network fails, is answered with the page of the build that
 *   last ran. That build's hashed files are kept as its pages fetch them and
 *   answered from the cache. One build is kept.
 * - Push notifications: it shows every push the backend sends, worded from the
 *   words the page left it, since a browser shows one of its own for a push
 *   that shows none, or ends the subscription. One whose words cannot be read
 *   still shows. A click on one opens its conversation, in a page of the
 *   product that is open, never a share's, or a new one.
 *
 * It answers nothing else from a cache: not `build.json`, the API, the sockets
 * or a share's page. It imports only what needs no page.
 */

declare const self: ServiceWorkerGlobalScope

/** The raw file route of a conversation. */
const RAW_ROUTE = /^\/api\/conversations\/[^/]+\/fs\/raw$/

/** Where a build's hashed files lie, whose names change with their content. */
const HASHED = '/assets/'

/**
 * The prefix of a build's cache, `demi-build-<id>`: the build's page under
 * `PAGE_KEY`, its hashed files under their own URLs, and, once a page of the
 * build ran, `RAN_KEY`.
 */
const BUILD_CACHE = 'demi-build-'
const PAGE_KEY = '/'
/** Marks the build whose page last ran, the one a failed navigation is answered with. */
const RAN_KEY = '/.demi-ran'

/** What a proxy in front of the backend answers while the backend is away. */
const PROXY_AWAY = new Set([502, 503, 504])

/** The build a page names in its `<meta name="demi-build">`, which `vite build` writes in. */
const BUILD_META = /<meta name="demi-build" content="([^"]+)"/

/**
 * The build each page of this origin runs, by its client id, as soon as
 * the worker knows it: from the page its navigation brought, or from the
 * page's own word (`demi-page`) when the worker did not answer its
 * navigation, as on the first visit. The web browser stops an idle worker
 * while its pages stay open, so each is kept in `CLIENTS` too, which a
 * worker started again reads.
 */
const clientBuilds = new Map<string, Promise<string | null>>()

/** The cache of each page's build, under `/<client id>`, for a worker started again. */
const CLIENTS = 'demi-clients'

/** Notes that client `id` runs the build `build` resolves with, once it does. */
function setClientBuild(id: string, build: Promise<string | null>): Promise<void> {
  clientBuilds.set(id, build)
  return build.then(async (known) => {
    if (known)
      await (await caches.open(CLIENTS)).put(`/${id}`, new Response(known))
  })
}

/** The build client `id` runs; read back from `CLIENTS` by a worker started again. */
function clientBuild(id: string): Promise<string | null> {
  let build = clientBuilds.get(id)
  if (!build) {
    build = caches.open(CLIENTS)
      .then((cache) => cache.match(`/${id}`))
      .then((kept) => kept?.text() ?? null)
    clientBuilds.set(id, build)
  }
  return build
}

/** What the page answers first: that it does not serve the request, or the answer's status and headers. */
type First =
  | { type: 'network' }
  | { type: 'answer'; status: number; headers: Record<string, string>; body: boolean }

/** What the page answers to each pull of the body. */
type Next = { type: 'bytes'; bytes: Uint8Array } | { type: 'end' } | { type: 'failed' }

self.addEventListener('install', () => {
  void self.skipWaiting()
})

self.addEventListener('activate', (event) => {
  event.waitUntil(self.clients.claim())
})

self.addEventListener('fetch', (event) => {
  const url = new URL(event.request.url)
  if (event.request.method !== 'GET' || url.origin !== self.location.origin)
    return
  if (RAW_ROUTE.test(url.pathname))
    event.respondWith(answer(event))
  else if (event.request.mode === 'navigate' && isAppPage(url.pathname))
    event.respondWith(navigate(event))
  else if (url.pathname.startsWith(HASHED))
    event.respondWith(hashed(event))
})

self.addEventListener('message', (event) => {
  const said = readPage(event.data)
  const source = event.source
  // A page is a client of the worker, the one source with an id.
  if (!said || !source || !('id' in source))
    return
  event.waitUntil(Promise.all([setClientBuild(source.id, Promise.resolve(said.build)), ran(said.build, said.files)]))
})

self.addEventListener('push', (event) => {
  event.waitUntil(showPush(event.data))
})

self.addEventListener('notificationclick', (event) => {
  event.notification.close()
  const conversationId = conversationOf(event.notification.data)
  if (conversationId)
    event.waitUntil(openConversation(conversationId))
})

/** Shows the push `data` carries; one that does not read still shows, as every push must. */
async function showPush(data: PushMessageData | null): Promise<void> {
  let message: unknown = null
  try {
    message = data?.json() ?? null
  } catch (error) {
    console.error('A push is not JSON', error)
  }
  const shown = await pushedNotification(message, words)
  await self.registration.showNotification(shown.title, shown.options)
}

/** The words the page left; none before it left any, or when they do not read. */
async function words(): Promise<NotificationWords | null> {
  const kept = await (await caches.open(WORDS_CACHE)).match(WORDS_KEY)
  if (!kept)
    return null
  const parsed = notificationWordsSchema.safeParse(await kept.json().catch(() => null))
  return parsed.success ? parsed.data : null
}

/** The conversation a notification's data names. */
function conversationOf(data: unknown): string | null {
  return isRecord(data) && typeof data.conversationId === 'string' ? data.conversationId : null
}

/**
 * Opens the conversation in a page of the product: the focused one, else
 * one that is visible, else any open one, brought forward, which opens it
 * as a click of its own would; else a new one.
 */
async function openConversation(conversationId: string): Promise<void> {
  const page = productPage(await self.clients.matchAll({ type: 'window', includeUncontrolled: true }))
  if (!page) {
    await self.clients.openWindow(`/chat/${encodeURIComponent(conversationId)}`)
    return
  }
  page.postMessage({ type: OPEN_CONVERSATION, conversationId })
  await page.focus()
}

/**
 * Whether a navigation to `path` is one of the app's pages, which the
 * backend answers with `index.html`: an extensionless path outside the API
 * and a share's page.
 */
function isAppPage(path: string): boolean {
  if (path.startsWith('/api/') || path.startsWith('/s/'))
    return false
  return !/\.[^/]*$/.test(path)
}

/**
 * A navigation of the app: the network's answer, whose page the worker
 * keeps under its build; only when the network fails, the page of the build
 * that last ran.
 */
async function navigate(event: FetchEvent): Promise<Response> {
  const settled = Promise.withResolvers<string | null>()
  if (event.resultingClientId)
    event.waitUntil(setClientBuild(event.resultingClientId, settled.promise))
  let response: Response
  try {
    response = await fetch(event.request)
  } catch (error) {
    settled.resolve(null)
    const page = await lastPage()
    if (page)
      return page
    throw error
  }
  const isPage = response.ok && (response.headers.get('content-type') ?? '').startsWith('text/html')
  if (!isPage) {
    settled.resolve(null)
    // A proxy in front of the backend that says it is away, as Cloudflare
    // does while a server restarts, fails the navigation as no network does.
    if (PROXY_AWAY.has(response.status))
      return (await lastPage()) ?? response
    return response
  }
  const kept = response.clone()
  event.waitUntil(keepPage(kept).then(settled.resolve, () => settled.resolve(null)))
  return response
}

/**
 * Keeps `response`, a page of the app, under the build it names, or only
 * when it names `only`; answers the build it kept it under.
 */
async function keepPage(response: Response, only?: string): Promise<string | null> {
  const text = await response.text()
  const build = BUILD_META.exec(text)?.[1] ?? null
  if (!build || (only !== undefined && build !== only))
    return null
  const cache = await caches.open(BUILD_CACHE + build)
  await cache.put(PAGE_KEY, new Response(text, { headers: { 'content-type': 'text/html' } }))
  return build
}

/** The page of the build that last ran, if the worker keeps one. */
async function lastPage(): Promise<Response | null> {
  for (const name of await caches.keys()) {
    if (!name.startsWith(BUILD_CACHE))
      continue
    const cache = await caches.open(name)
    if (await cache.match(RAN_KEY))
      return (await cache.match(PAGE_KEY)) ?? null
  }
  return null
}

/**
 * A hashed file of a build, which never changes: from the cache when a
 * build keeps it, else from the network, kept under the build of the page
 * that fetched it.
 */
async function hashed(event: FetchEvent): Promise<Response> {
  const kept = await caches.match(event.request)
  if (kept)
    return kept
  const response = await fetch(event.request)
  if (response.status === 200) {
    const copy = response.clone()
    event.waitUntil(keepFile(event.clientId, event.request, copy))
  }
  return response
}

async function keepFile(clientId: string, request: Request, response: Response): Promise<void> {
  const build = await clientBuild(clientId)
  if (!build)
    return
  const cache = await caches.open(BUILD_CACHE + build)
  await cache.put(request, response)
}

/**
 * A page of `build` ran: it is the build a failed navigation is answered
 * with, every other build's cache goes, and the files the page fetched
 * before the worker answered its requests are kept, with its page.
 */
async function ran(build: string, files: string[]): Promise<void> {
  const name = BUILD_CACHE + build
  // The builds of pages that are gone are no one's any more.
  const open = new Set((await self.clients.matchAll({ includeUncontrolled: true })).map((client) => `/${client.id}`))
  const clients = await caches.open(CLIENTS)
  for (const kept of await clients.keys()) {
    const key = new URL(kept.url).pathname
    if (!open.has(key))
      await clients.delete(key)
  }
  for (const other of await caches.keys()) {
    if (other.startsWith(BUILD_CACHE) && other !== name)
      await caches.delete(other)
  }
  const cache = await caches.open(name)
  await cache.put(RAN_KEY, new Response(null, { status: 204 }))
  if (!(await cache.match(PAGE_KEY))) {
    // The page came before the worker answered its navigation: the
    // backend's page now is kept if it is still of this build.
    // A page the network cannot give now is kept by the next navigation.
    const response = await fetch(PAGE_KEY, { headers: { accept: 'text/html' } }).catch(() => null)
    if (response?.ok)
      await keepPage(response, build)
  }
  for (const file of files) {
    if (await cache.match(file))
      continue
    // A file the network cannot give now is kept when a page fetches it next.
    const response = await fetch(file).catch(() => null)
    if (response?.status === 200)
      await cache.put(file, response)
  }
}

/** A page's word that it runs `build`, with the hashed files it fetched so far; none for another message. */
function readPage(value: unknown): { build: string; files: string[] } | null {
  if (!isRecord(value) || value.type !== 'demi-page' || typeof value.build !== 'string' || !Array.isArray(value.files))
    return null
  const files: string[] = []
  for (const file of value.files) {
    if (typeof file !== 'string' || !file.startsWith(HASHED))
      return null
    files.push(file)
  }
  return { build: value.build, files }
}

async function answer(event: FetchEvent): Promise<Response> {
  const request = event.request
  const client = event.clientId ? await self.clients.get(event.clientId) : undefined
  if (!client)
    return fetch(request)
  const channel = new MessageChannel()
  const port = channel.port1
  const replies = replyQueue(port)
  client.postMessage(
    {
      type: 'demi-direct-raw',
      url: request.url,
      range: request.headers.get('range'),
      ifNoneMatch: request.headers.get('if-none-match'),
    },
    [channel.port2],
  )
  const first = readFirst(await replies.next())
  if (!first || first.type === 'network') {
    port.close()
    return fetch(request)
  }
  if (!first.body) {
    port.close()
    return new Response(null, { status: first.status, headers: first.headers })
  }
  const body = new ReadableStream<Uint8Array>({
    async pull(controller) {
      port.postMessage({ type: 'pull' })
      const next = readNext(await replies.next())
      if (next?.type === 'bytes') {
        controller.enqueue(next.bytes)
        return
      }
      port.close()
      if (next?.type === 'end')
        controller.close()
      else
        controller.error(new Error('The direct channel failed under the transfer'))
    },
    cancel() {
      port.postMessage({ type: 'cancel' })
      port.close()
    },
  })
  return new Response(body, { status: first.status, headers: first.headers })
}

/** The port's messages in order, one at a time. */
function replyQueue(port: MessagePort): { next(): Promise<unknown> } {
  const queued: unknown[] = []
  let waiting: ((value: unknown) => void) | null = null
  port.onmessage = (event) => {
    if (waiting) {
      waiting(event.data)
      waiting = null
    } else {
      queued.push(event.data)
    }
  }
  return {
    next: () =>
      queued.length > 0
        ? Promise.resolve(queued.shift())
        : new Promise((resolve) => {
            waiting = resolve
          }),
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null
}

/** The page's first answer, checked as it arrives; none for one that does not read. */
function readFirst(value: unknown): First | null {
  if (!isRecord(value))
    return null
  if (value.type === 'network')
    return { type: 'network' }
  if (value.type !== 'answer' || typeof value.status !== 'number' || typeof value.body !== 'boolean' || !isRecord(value.headers))
    return null
  const headers: Record<string, string> = {}
  for (const [name, header] of Object.entries(value.headers)) {
    if (typeof header !== 'string')
      return null
    headers[name] = header
  }
  return { type: 'answer', status: value.status, headers, body: value.body }
}

/** The page's answer to a pull, checked as it arrives; none for one that does not read. */
function readNext(value: unknown): Next | null {
  if (!isRecord(value))
    return null
  if (value.type === 'bytes' && value.bytes instanceof Uint8Array)
    return { type: 'bytes', bytes: value.bytes }
  if (value.type === 'end' || value.type === 'failed')
    return { type: value.type }
  return null
}


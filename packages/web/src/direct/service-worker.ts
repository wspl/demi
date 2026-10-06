/**
 * The page's service worker (`direct-channel.md` § Bytes the browser
 * fetches itself): a `GET` of a conversation's raw file route, which an
 * image, a video, a PDF or a download makes, goes to the page that made
 * it, which answers it over its direct channel when it has one to the
 * conversation's device. Everything else, and every request the page does
 * not answer, goes to the network unchanged. It is a module that imports
 * nothing, served from the root.
 */

declare const self: ServiceWorkerGlobalScope

/** The raw file route of a conversation (`web-api.md` § File text and working tree changes). */
const RAW_ROUTE = /^\/api\/conversations\/[^/]+\/fs\/raw$/

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
  if (event.request.method !== 'GET' || url.origin !== self.location.origin || !RAW_ROUTE.test(url.pathname))
    return
  event.respondWith(answer(event))
})

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

export {}

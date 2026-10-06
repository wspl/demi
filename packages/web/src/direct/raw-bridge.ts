/**
 * The page's side of its service worker (`direct-channel.md` § Bytes the
 * browser fetches itself): the worker hands the page each `GET` of a raw
 * file route the page made, and the page, which holds the peer, answers it
 * over the conversation's direct channel with the relay's status and
 * headers, its bytes as the worker pulls them. A request the page cannot
 * answer directly goes to the network, the relay, which also answers a
 * version the file no longer has.
 */
import { z } from 'zod'
import { readOpenedSchema } from '../api/generated/web-api'
import { directRoute } from '.'
import { ChannelFailed, type DirectPeer, type OperationChannel } from './peer'
import { rawAnswer, type RawRequest } from './raw'

/** Where the built app and the development server serve the worker, so its scope is the whole origin. */
export const SERVICE_WORKER_URL = '/direct-sw.js'

/** A request the worker hands the page. */
const rawRequestSchema = z.object({
  type: z.literal('demi-direct-raw'),
  url: z.string(),
  range: z.string().nullable(),
  ifNoneMatch: z.string().nullable(),
})

/** What the worker asks of a body: its next bytes, or to stop. */
const pullSchema = z.object({ type: z.enum(['pull', 'cancel']) })

/** The raw route's path: `/api/conversations/:id/fs/raw`. */
const RAW_ROUTE = /^\/api\/conversations\/([^/]+)\/fs\/raw$/

/**
 * Registers the worker and answers its requests for the page's lifetime.
 * A browser without service workers, or a page served where it cannot
 * register one, fetches every byte from the relay.
 */
export function startRawBridge(): void {
  const container = typeof navigator === 'undefined' ? undefined : navigator.serviceWorker
  if (!container)
    return
  container.register(SERVICE_WORKER_URL, { scope: '/', type: 'module' }).catch((error: unknown) => {
    // The relay serves every byte meanwhile; the reason is for a developer.
    console.warn('The direct channel’s service worker did not register', error)
  })
  container.addEventListener('message', (event: MessageEvent) => {
    const request = rawRequestSchema.safeParse(event.data)
    const port = event.ports[0]
    if (!request.success || !port)
      return
    void serve(request.data, port)
  })
}

/** Answers one request on `port`: over the direct channel when the page can, or by sending it to the network. */
async function serve(request: z.infer<typeof rawRequestSchema>, port: MessagePort): Promise<void> {
  const url = new URL(request.url)
  const conversation = RAW_ROUTE.exec(url.pathname)?.[1]
  const path = url.searchParams.get('path')
  const route = conversation ? directRoute(decodeURIComponent(conversation)) : null
  const peer = route?.device.current()
  if (!route || !peer || !path) {
    port.postMessage({ type: 'network' })
    port.close()
    return
  }
  const raw: RawRequest = {
    path,
    version: url.searchParams.get('version'),
    download: url.searchParams.get('download') === 'true',
    range: request.range,
    ifNoneMatch: request.ifNoneMatch,
  }
  let body: OperationChannel<unknown> | null = null
  let length = 0
  try {
    const answer = await answerOf(peer, route.scope, raw)
    body = answer.body
    length = answer.length
    port.postMessage({ type: 'answer', status: answer.status, headers: answer.headers, body: body !== null })
  } catch (error) {
    if (error instanceof ChannelFailed)
      route.device.failed()
    // The relay answers what the channel did not: a version the file no
    // longer has, a refusal, or a channel that failed.
    port.postMessage({ type: 'network' })
    port.close()
    return
  }
  if (body)
    pump(body, length, port, () => route.device.failed())
  else
    port.close()
}

/** The relay's answer to `raw`, and the open channel of its bytes when it sends any. */
async function answerOf(
  peer: DirectPeer,
  scope: { conversation: string; cwd: string },
  raw: RawRequest,
): Promise<{ status: number; headers: Record<string, string>; body: OperationChannel<unknown> | null; length: number }> {
  // What the file is, its size and version, without its bytes; a version
  // the URL names that it no longer has is refused.
  const probe = await peer.open(
    { ...scope, op: 'read', path: raw.path, length: 0, ...(raw.version ? { version: raw.version } : {}) },
    readOpenedSchema,
  )
  const file = await probe.answer()
  probe.close()
  const answer = rawAnswer(raw, file)
  if (!answer.bytes)
    return { status: answer.status, headers: answer.headers, body: null, length: 0 }
  // The part, of that version only.
  const read = await peer.open(
    { ...scope, op: 'read', path: raw.path, offset: answer.bytes.offset, length: answer.bytes.length, version: file.version },
    readOpenedSchema,
  )
  await read.answer()
  return { status: answer.status, headers: answer.headers, body: read, length: answer.bytes.length }
}

/**
 * Hands the worker the body's bytes as it pulls them; its last answer says
 * whether the body ended complete, with all `length` of its bytes, or cut
 * short, which the browser sees as a failed transfer.
 */
function pump(body: OperationChannel<unknown>, length: number, port: MessagePort, failed: () => void): void {
  let received = 0
  let pulling = Promise.resolve()
  port.onmessage = (event) => {
    const asked = pullSchema.safeParse(event.data)
    if (!asked.success)
      return
    if (asked.data.type === 'cancel') {
      body.close()
      port.close()
      return
    }
    pulling = pulling.then(async () => {
      try {
        const message = await body.next()
        if (message === null) {
          port.postMessage({ type: received === length ? 'end' : 'failed' })
          return
        }
        if (message.kind === 'bytes') {
          received += message.bytes.length
          port.postMessage({ type: 'bytes', bytes: message.bytes }, [message.bytes.buffer])
        }
        else
          port.postMessage({ type: 'failed' })
      } catch {
        failed()
        port.postMessage({ type: 'failed' })
      }
    })
  }
}

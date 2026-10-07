// The preview domain service (docs/browser/preview.md § The preview domain
// service) as a fetch handler over three ports, so that the Worker
// (`worker.ts`) binds them to Cloudflare's KV, static assets and rate limiter,
// and the tests to memory. It keeps namespaces at the domain's root and serves
// the same versioned files on every preview origin; it never sees what a
// preview shows, and it logs nothing.
import { z } from 'zod'

/** How long a namespace lives after its creation or its last renewal. */
export const NAMESPACE_LIFETIME_MS = 90 * 24 * 60 * 60 * 1000
const MAXIMUM_ORIGINS = 8
// The version whose boot page takes a navigation that arrives before any
// forwarder is installed. A new version adds its directory under
// `static/__demi/` and raises this.
const LATEST_VERSION = 1

// Namespaces and labels are lowercase base32hex (RFC 4648 § 7), as the engine
// writes labels: 8 characters are 40 bits, 16 are 80.
const BASE32HEX = '0123456789abcdefghijklmnopqrstuv'
const NAMESPACE = /^[0-9a-v]{8}$/
const LABEL = /^[0-9a-v]{16}$/
const NAMESPACE_BYTES = 5
const SECRET_BYTES = 32
const API = /^\/api\/v1\/namespaces(?:\/([^/]+)(?:\/(renew|origins))?)?$/
// The files of a version, `policy` among them, which the Worker writes.
const VERSIONED = /^\/__demi\/v([1-9][0-9]*)\/(boot\.html|boot\.js|client\.js|sw\.js|policy)$/
const CONTENT_TYPES: Record<string, string> = {
  html: 'text/html; charset=utf-8',
  js: 'text/javascript; charset=utf-8',
}

/** Where namespaces are kept, by name: a KV namespace in production. */
export interface NamespaceStore {
  get(name: string): Promise<string | null>
  put(name: string, value: string): Promise<void>
}

/** The versioned static files: Cloudflare's static assets in production. */
export interface StaticFiles {
  fetch(request: Request): Promise<Response>
}

/** Limits namespace creation per source address: a rate limiting binding in production. */
export interface CreationLimiter {
  limit(options: { key: string }): Promise<{ success: boolean }>
}

export interface ServiceOptions {
  /** The preview domain with its port, if any: `demi-preview.dev`, or `demi-preview.localhost:3335` in development. */
  domain: string
  store: NamespaceStore
  files: StaticFiles
  creationLimiter: CreationLimiter
  now?: () => number
  /** Fills a buffer with random bytes. */
  random?: (bytes: Uint8Array<ArrayBuffer>) => Uint8Array<ArrayBuffer>
}

/**
 * An origin a Demi page may be served on: a secure context, so the previews
 * it embeds can install their forwarder, written exactly as `URL.origin`
 * writes it. The host may hold only letters, digits, dots and hyphens, since
 * it goes into a `Content-Security-Policy` source list as it is.
 */
const embedderOrigin = z.string().refine((value) => {
  const url = URL.parse(value)
  if (!url || url.origin !== value || !/^[a-z0-9.-]+$/.test(url.hostname)) return false
  return url.protocol === 'https:' || (url.protocol === 'http:' && ['localhost', '127.0.0.1'].includes(url.hostname))
}, 'not a secure-context origin')

const originsBody = z.object({
  origins: z.array(embedderOrigin).min(1).max(MAXIMUM_ORIGINS),
})

// A namespace is active, expired once `expiresAt` passes, or deleted. A
// deleted one stays as a tombstone, so its name is never given out again: a
// browser may still keep the previous owner's storage under it.
const namespaceRecord = z.discriminatedUnion('status', [
  z.object({
    status: z.literal('active'),
    secretDigest: z.string(),
    origins: z.array(embedderOrigin),
    expiresAt: z.number(),
  }),
  z.object({ status: z.literal('deleted') }),
])
type NamespaceRecord = z.infer<typeof namespaceRecord>
type ActiveRecord = Extract<NamespaceRecord, { status: 'active' }>

function base32hex(bytes: Uint8Array): string {
  let bits = 0
  let value = 0
  let output = ''
  for (const byte of bytes) {
    value = (value << 8) | byte
    bits += 8
    while (bits >= 5) {
      output += BASE32HEX[(value >>> (bits - 5)) & 31]
      bits -= 5
    }
  }
  return bits > 0 ? output + BASE32HEX[(value << (5 - bits)) & 31] : output
}

async function sha256(text: string): Promise<string> {
  return base32hex(new Uint8Array(await crypto.subtle.digest('SHA-256', new TextEncoder().encode(text))))
}

function plain(text: string, status: number, headers: Record<string, string> = {}): Response {
  return new Response(text, { status, headers: { ...headers, 'content-type': 'text/plain; charset=utf-8' } })
}

const ROOT_PAGE = `<!doctype html>
<html lang="en">
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>Demi preview domain</title>
<body style="font: 16px/1.5 system-ui, sans-serif; max-width: 40rem; margin: 3rem auto; padding: 0 1rem">
<h1>Demi preview domain</h1>
<p>This domain shows web pages that run on a computer you use with <a href="https://github.com/wspl/demi">Demi</a>,
inside the Demi page that opened them. It hosts no content of its own, and a page under it opened on its own shows nothing.</p>
<p>To report abuse, open a private report at
<a href="https://github.com/wspl/demi/security/advisories/new">github.com/wspl/demi/security</a>.</p>
</body>
</html>
`

/** Answers every request the preview domain receives. */
export function createService(options: ServiceOptions): (request: Request) => Promise<Response> {
  const { store, files, creationLimiter } = options
  const now = options.now ?? Date.now
  const random = options.random ?? ((bytes) => crypto.getRandomValues(bytes))
  // A `.localhost` domain is development's, which Chrome treats as a secure
  // context over plain HTTP; any other is served over HTTPS.
  const domain = new URL(`http://${options.domain}`)
  const scheme = domain.hostname === 'localhost' || domain.hostname.endsWith('.localhost') ? 'http:' : 'https:'
  const previewFrames = `${scheme}//*.${domain.host}`

  async function read(name: string): Promise<NamespaceRecord | null> {
    const stored = await store.get(name)
    return stored === null ? null : namespaceRecord.parse(JSON.parse(stored))
  }

  async function write(name: string, record: NamespaceRecord): Promise<void> {
    await store.put(name, JSON.stringify(record))
  }

  function expiry(): number {
    return now() + NAMESPACE_LIFETIME_MS
  }

  /** The record while its namespace lives, neither deleted nor past its expiry; null once it is gone for good. */
  function living(record: NamespaceRecord): ActiveRecord | null {
    return record.status === 'active' && record.expiresAt > now() ? record : null
  }

  async function parseOrigins(request: Request): Promise<string[] | null> {
    // A body that is not JSON is refused below like any other that is not a list of origins.
    const body = await request.json().catch(() => null)
    const parsed = originsBody.safeParse(body)
    return parsed.success ? parsed.data.origins : null
  }

  function invalidOrigins(): Response {
    return plain(`origins must be 1 to ${MAXIMUM_ORIGINS} secure-context origins: https, or http on localhost or 127.0.0.1`, 400)
  }

  async function create(request: Request): Promise<Response> {
    const address = request.headers.get('cf-connecting-ip')
    if (!address) return plain('The request has no source address', 400)
    if (!(await creationLimiter.limit({ key: address })).success) {
      return plain('Too many namespaces created from this address', 429, { 'retry-after': '60' })
    }
    const origins = await parseOrigins(request)
    if (!origins) return invalidOrigins()
    let name: string
    do {
      name = base32hex(random(new Uint8Array(NAMESPACE_BYTES)))
    } while (await store.get(name) !== null)
    const secret = base32hex(random(new Uint8Array(SECRET_BYTES)))
    const record: ActiveRecord = { status: 'active', secretDigest: await sha256(secret), origins, expiresAt: expiry() }
    await write(name, record)
    return Response.json({ namespace: name, secret, expiresAt: new Date(record.expiresAt).toISOString() }, { status: 201 })
  }

  /** The namespace's record when the request may change it, or the answer that says why not. */
  async function authorize(request: Request, name: string): Promise<ActiveRecord | Response> {
    const record = NAMESPACE.test(name) ? await read(name) : null
    if (!record) return plain('No such namespace', 404)
    const active = living(record)
    if (!active) return plain('This namespace expired; register a new one', 410)
    const secret = /^Bearer (\S+)$/.exec(request.headers.get('authorization') ?? '')?.[1]
    if (!secret || await sha256(secret) !== active.secretDigest) {
      return plain('Wrong or missing secret', 401, { 'www-authenticate': 'Bearer' })
    }
    return active
  }

  async function api(request: Request, path: string): Promise<Response> {
    const match = API.exec(path)
    if (!match) return plain('Not found', 404)
    const [, name, action] = match
    const allowed = !name ? 'POST' : action === 'renew' ? 'POST' : action === 'origins' ? 'PUT' : 'DELETE'
    if (request.method !== allowed) return plain('Method not allowed', 405, { allow: allowed })
    if (!name) return create(request)
    const record = await authorize(request, name)
    if (record instanceof Response) return record
    if (action === 'renew') {
      record.expiresAt = expiry()
    } else if (action === 'origins') {
      const origins = await parseOrigins(request)
      if (!origins) return invalidOrigins()
      record.origins = origins
    } else {
      await write(name, { status: 'deleted' })
      return new Response(null, { status: 204 })
    }
    await write(name, record)
    return Response.json({ namespace: name, origins: record.origins, expiresAt: new Date(record.expiresAt).toISOString() })
  }

  /** A versioned file's body, or null when no published version has it. */
  async function staticFile(path: string, base: URL): Promise<Response | null> {
    const response = await files.fetch(new Request(new URL(path, base).href))
    if (response.ok) return response
    // The body of a missing file is the asset server's own page; nothing reads it.
    await response.body?.cancel()
    return null
  }

  async function preview(request: Request, url: URL, name: string): Promise<Response> {
    const record = await read(name)
    // Only the registered Demi pages, and preview frames inside them, may embed the namespace.
    const origins = record?.status === 'active' ? record.origins : []
    const frameAncestors = [...origins, previewFrames].join(' ')
    const headers: Record<string, string> = {
      'content-security-policy': `frame-ancestors ${frameAncestors}`,
      'referrer-policy': 'same-origin',
      'cache-control': 'no-cache',
    }
    if (!record) return plain('No such preview namespace', 404, headers)
    if (!living(record)) return plain('This preview namespace expired', 410, headers)
    const versioned = VERSIONED.exec(url.pathname)
    if (versioned) {
      const [, version, file] = versioned
      if (file === 'policy') {
        // Every version that has a forwarder has a policy.
        const forwarder = await staticFile(`/__demi/v${version}/sw.js`, url)
        if (forwarder) {
          await forwarder.body?.cancel()
          return Response.json({ frameAncestors }, { headers })
        }
      } else {
        const found = await staticFile(url.pathname, url)
        if (found) {
          if (file === 'sw.js') headers['service-worker-allowed'] = '/'
          headers['content-type'] = CONTENT_TYPES[file.slice(file.lastIndexOf('.') + 1)]
          return new Response(found.body, { headers })
        }
      }
    }
    // Any other request arrives before the forwarder is installed. A document
    // navigation gets the boot page, whose target is then its own address;
    // anything else, a site's own service worker script among them, gets
    // nothing. Nothing about the request is kept or logged.
    if (request.headers.get('sec-fetch-mode') === 'navigate') {
      const boot = await staticFile(`/__demi/v${LATEST_VERSION}/boot.html`, url)
      if (!boot) throw new Error(`the static files have no version ${LATEST_VERSION}`)
      headers['content-type'] = CONTENT_TYPES.html
      return new Response(boot.body, { headers })
    }
    return new Response(null, { status: 404, headers })
  }

  return async (request) => {
    const url = new URL(request.url)
    if (url.protocol !== scheme || url.port !== domain.port) return plain('Not found', 404)
    if (url.hostname === domain.hostname) {
      if (url.pathname.startsWith('/api/')) return api(request, url.pathname)
      if (url.pathname === '/') return new Response(ROOT_PAGE, { headers: { 'content-type': CONTENT_TYPES.html } })
      return plain('Not found', 404)
    }
    const subdomain = url.hostname.endsWith(`.${domain.hostname}`) ? url.hostname.slice(0, -domain.hostname.length - 1) : ''
    const [name = '', label = '', ...rest] = subdomain.split('--')
    if (!NAMESPACE.test(name) || !LABEL.test(label) || rest.length > 0) return plain('Not found', 404)
    return preview(request, url, name)
  }
}

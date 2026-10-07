/**
 * The relay (`preview.md` § The forwarder and the relay): the Demi page's
 * side of every preview document. Each preview origin's forwarder hands the
 * relay the requests its documents make, and each document's runtime its
 * labels, WebSockets and windows, over channels the documents ask the page
 * for. The relay binds each channel to the label of the origin that asked,
 * finds the tab of the user's browser it belongs to by the asking window's
 * place in this page's frames, maps preview addresses back to real ones,
 * and sends each request on the tab's conversation's `preview` stream.
 *
 * One relay serves the page, every conversation's tabs: it listens to the
 * page's window once, and each tab registers itself while its content
 * lives.
 */
import type { PreviewPlace } from '@demicodes/plugin-sdk'
import { z } from 'zod'
import {
  previewCredentialsSchema,
  previewEnvironmentSchema,
  previewModeSchema,
  type PreviewClient,
  type PreviewEnvironment,
  type PreviewHeader,
} from '../generated/plugin'
import { previewClient } from './client'
import type { PreviewConnection, PreviewExchange, PreviewSocket } from './connection'
import { BOOT_PATH, labelOf, labelOfOrigin, previewOrigin, realAddress } from './labels'

/** How long a request waits for its label to be registered, after which it fails as a network error. */
export const LABEL_WAIT_MS = 10_000
/** Where a rewritten document loads the runtime of the engine's release from. */
const RUNTIME_PATH = /^\/__demi\/page\/runtime\/([^/]+)\.js$/
/** The engine answers a document's cookies on its own origin. */
const COOKIE_PATH = '/__demi/host/cookie'
const REDIRECTS = [301, 302, 303, 307, 308]
/** Statuses and methods whose answer has no body. */
const EMPTY_STATUSES = [101, 204, 205, 304]

/** What a tab's top document says of the page it shows, which the tab's bar and strip show. */
const tabPageSchema = z.object({
  url: z.string(),
  title: z.string(),
  /** The page's icon's real address; empty for none. */
  icon: z.string(),
  canGoBack: z.boolean(),
  canGoForward: z.boolean(),
})
export type TabPage = z.infer<typeof tabPageSchema>

/** The labels a runtime registers, each with its environment. */
const entriesSchema = z.record(z.string(), previewEnvironmentSchema)

/** What the relay tells a tab of what its documents do. */
export type TabEvent =
  | { type: 'page'; page: TabPage }
  /** The top document is leaving: the tab loads. */
  | { type: 'leaving' }
  /** The top frame's navigation failed: the forwarder answered it a network error. */
  | { type: 'failed'; reason: string }

/** Where a navigation of a tab comes from. */
export interface Navigation {
  /** The real address. */
  url: string
  /** The environment that started it; null for the user's own. */
  initiator: PreviewEnvironment | null
}

/** A message a preview window posts to the Demi page, as the relay reads it. */
export interface RelayMessage {
  readonly data: unknown
  /** The origin of the window that posted it, which the browser guarantees. */
  readonly origin: string
  /** The window that posted it. */
  readonly source: unknown
  readonly ports: readonly MessagePort[]
}

/** The window the relay listens on: the Demi page's. */
export interface RelayWindow {
  addEventListener(type: 'message', listener: (event: MessageEvent) => void): void
}

/** A tab of the user's browser, as the relay reaches it. */
export interface RelayTab {
  readonly id: string
  /** Where its conversation's previews live; null while they cannot. */
  place(): PreviewPlace | null
  /** Its frame's window, while its content shows a frame: its top document's window. */
  frameWindow(): unknown
  readonly connection: PreviewConnection
  /** Something its documents did. */
  report(event: TabEvent): void
  /** Opens a new tab of the user's browser beside this one, opened by `opener` as its popup `popup`. */
  openWindow(opener: RelayBinding, popup: number, navigation: Navigation | null): void
  /** Navigates the tab's top frame. */
  navigate(navigation: Navigation): void
  /** Closes the tab, as its page asked. */
  close(): void
  /** The binding of the window that opened this tab, and its popup id there. */
  readonly opener: { binding: RelayBinding; popup: number } | null
}

/** A channel the relay bound to the label of the origin that asked for it. */
export interface RelayBinding {
  readonly port: MessagePort
  readonly label: string
  readonly environment: PreviewEnvironment
  readonly tab: RelayTab
  /** The window that asked for the channel. */
  readonly source: unknown
}

interface Binding extends RelayBinding {
  readonly purpose: 'forwarder' | 'document'
  /** The last cookie write: the channel's later requests go after it (`preview.md` § Cookies). */
  cookieWrite: Promise<unknown>
  /** Bodies the forwarder pulls, by its request id. */
  readonly bodies: Map<number, { pull(): Promise<Uint8Array<ArrayBuffer> | null>; cancel(): void }>
  readonly sockets: Map<number, PreviewSocket>
  /** The labels its document was told of. */
  readonly told: Set<string>
}

/** A request the relay keeps for a navigation that cannot carry it. */
interface Kept {
  request: { method: string; contentType: string | null; body: ArrayBuffer | null; user: boolean }
  initiator: PreviewEnvironment | null
  /** The label of the navigation's target: only its channel takes the token. */
  label: string
}

/** What a forwarder sends for a request (the preview domain's `sw.js`), checked where it enters the page. */
const forwardedRequestSchema = z.object({
  type: z.literal('fetch'),
  id: z.number(),
  url: z.string(),
  method: z.string(),
  headers: z.array(z.tuple([z.string(), z.string()])),
  body: z.instanceof(ArrayBuffer).nullable(),
  mode: previewModeSchema,
  destination: z.string(),
  credentials: previewCredentialsSchema,
  referrer: z.string(),
  referrerPolicy: z.string(),
  token: z.string().nullable(),
  announcedReferrer: z.string().nullable(),
})
type ForwardedRequest = z.infer<typeof forwardedRequestSchema>

/** A request a document keeps behind a token (the runtime's form submission), checked where it enters the page. */
const keptRequestSchema = z.object({
  method: z.string(),
  url: z.string(),
  contentType: z.string().nullable(),
  body: z.instanceof(ArrayBuffer).nullable(),
})

/** What a document's runtime sends on its channel (`packages/preview-runtime`), checked where it enters the page. */
const documentMessageSchema = z.discriminatedUnion('type', [
  z.object({ type: z.literal('labels'), entries: entriesSchema }),
  z.object({ type: z.literal('label'), label: z.string() }),
  z.object({ type: z.literal('socket-open'), id: z.number(), url: z.string(), protocols: z.array(z.string()) }),
  z.object({ type: z.literal('socket-send'), id: z.number(), data: z.union([z.string(), z.instanceof(ArrayBuffer)]) }),
  z.object({ type: z.literal('socket-close'), id: z.number(), code: z.number().default(1000), reason: z.string().default('') }),
  z.object({ type: z.literal('tab-page'), page: tabPageSchema }),
  z.object({ type: z.literal('tab-leaving') }),
  z.object({ type: z.literal('tab-open'), id: z.number(), url: z.string() }),
  z.object({ type: z.literal('tab-navigate'), id: z.number(), url: z.string() }),
  z.object({ type: z.literal('tab-close'), id: z.number() }),
  // The page's targetOrigin, as postMessage reads it: as a string.
  z.object({ type: z.literal('tab-post'), id: z.number(), data: z.unknown(), targetOrigin: z.coerce.string() }),
  z.object({ type: z.literal('opener-post'), data: z.unknown(), targetOrigin: z.coerce.string() }),
  z.object({ type: z.literal('opener-navigate'), url: z.string() }),
  z.object({ type: z.literal('tab-navigate-self'), url: z.string() }),
  z.object({ type: z.literal('close-self') }),
])
type DocumentMessage = z.infer<typeof documentMessageSchema>
/** The messages of a document's windows: the tabs it opens, its opener, and its own tab. */
type WindowMessage = Exclude<
  DocumentMessage,
  { type: 'labels' | 'label' | 'socket-open' | 'socket-send' | 'socket-close' | 'tab-page' | 'tab-leaving' }
>

/** The key of a label within its namespace and Host. */
function labelKey(place: PreviewPlace, label: string): string {
  return `${place.namespace}\n${place.host}\n${label}`
}

export class PreviewRelay {
  private readonly tabs = new Set<RelayTab>()
  /** The environment each label stands for, kept once it checked out. */
  private readonly labels = new Map<string, PreviewEnvironment>()
  private readonly waiters = new Map<string, Set<(environment: PreviewEnvironment) => void>>()
  /** The document channels of each label, which hear of the labels its answers map. */
  private readonly documents = new Map<string, Set<Binding>>()
  private readonly kept = new Map<string, Kept>()
  /** The channel of each tab's latest top document. */
  private readonly tops = new Map<RelayTab, Binding>()
  private runtimes = new Map<string, Promise<ArrayBuffer>>()
  private listening = false

  constructor(
    private readonly window: RelayWindow,
    private readonly fetchRuntime: (url: string) => Promise<ArrayBuffer> = (url) => fetch(url).then((response) => {
      if (!response.ok) {
        throw new Error(`the preview runtime answered ${response.status}`)
      }
      return response.arrayBuffer()
    }),
    /** The user's browser, as the requests describe it. */
    private readonly client: () => PreviewClient = previewClient,
  ) {}

  /** Starts relaying for `tab` until the returned function is called. */
  register(tab: RelayTab): () => void {
    this.tabs.add(tab)
    this.listen()
    return () => {
      this.tabs.delete(tab)
      this.tops.delete(tab)
    }
  }

  private listen(): void {
    if (this.listening) {
      return
    }
    this.listening = true
    this.window.addEventListener('message', (event) => void this.receive(event))
  }

  /** A message to the Demi page: a preview document asking for a channel, or keeping a request. */
  receive(message: RelayMessage): Promise<void> {
    const type = (message.data as { type?: unknown } | null)?.type
    if (type === 'demi-preview-connect') {
      return this.connect(message)
    }
    if (type === 'demi-preview-keep') {
      return this.keep(message)
    }
    return Promise.resolve()
  }

  /**
   * The tab whose frame's window `source` is, or is inside: the window whose
   * parent is the Demi page, found by climbing from `source`.
   */
  tabOf(source: unknown): RelayTab | null {
    let candidate = source
    try {
      for (let parent = parentOf(candidate); parent !== null && parent !== this.window && parent !== candidate; parent = parentOf(candidate)) {
        candidate = parent
      }
    } catch {
      return null
    }
    if (!candidate) {
      return null
    }
    for (const tab of this.tabs) {
      if (tab.frameWindow() === candidate) {
        return tab
      }
    }
    return null
  }

  /**
   * Keeps `entries` whose label is the one their environment computes to
   * in `place`, and wakes the requests waiting for them. Answers the labels
   * kept.
   */
  async learn(place: PreviewPlace, entries: Record<string, PreviewEnvironment>): Promise<string[]> {
    const kept: string[] = []
    for (const [label, environment] of Object.entries(entries)) {
      const key = labelKey(place, label)
      if (this.labels.has(key)) {
        kept.push(label)
        continue
      }
      if ((await labelOf(place, environment)) !== label) {
        continue
      }
      this.labels.set(key, environment)
      kept.push(label)
      for (const resolve of this.waiters.get(key) ?? []) {
        resolve(environment)
      }
      this.waiters.delete(key)
    }
    return kept
  }

  /** The environment of `label`, waiting up to `LABEL_WAIT_MS` for it to be registered; null after. */
  environmentOf(place: PreviewPlace, label: string): Promise<PreviewEnvironment | null> {
    const key = labelKey(place, label)
    const known = this.labels.get(key)
    if (known) {
      return Promise.resolve(known)
    }
    return new Promise((resolve) => {
      const waiters = this.waiters.get(key) ?? new Set()
      this.waiters.set(key, waiters)
      const settle = (environment: PreviewEnvironment) => {
        clearTimeout(timer)
        resolve(environment)
      }
      const timer = setTimeout(() => {
        waiters.delete(settle)
        if (waiters.size === 0) {
          this.waiters.delete(key)
        }
        resolve(null)
      }, LABEL_WAIT_MS)
      waiters.add(settle)
    })
  }

  /** The environment of a preview address's label, or null for an address that is none. */
  private async environmentOfAddress(place: PreviewPlace, address: string): Promise<PreviewEnvironment | null> {
    const url = URL.parse(address)
    const label = url && labelOfOrigin(place, url.origin)
    return label ? this.environmentOf(place, label) : null
  }

  /**
   * The boot page address that opens `navigation` in a tab whose top label
   * is `label`: the relay keeps the request, and only that label's channel
   * takes it, by the token in the fragment (`preview.md` § Opening and
   * navigating).
   */
  bootAddress(place: PreviewPlace, label: string, navigation: Navigation): string {
    const token = crypto.randomUUID()
    this.kept.set(token, {
      request: { method: 'GET', contentType: null, body: null, user: navigation.initiator === null },
      initiator: navigation.initiator,
      label,
    })
    const target = new URL(navigation.url)
    return `${previewOrigin(place, label)}${BOOT_PATH}#token=${encodeURIComponent(token)}&to=${target.pathname}${target.search}${target.hash}`
  }

  /** A channel for the preview document that asks, bound to its origin's label. */
  private async connect(event: RelayMessage): Promise<void> {
    const reply = event.ports[0]
    const tab = this.tabOf(event.source)
    const place = tab?.place()
    const label = place ? labelOfOrigin(place, event.origin) : null
    if (!reply || !tab || !place || !label) {
      return
    }
    const environment = await this.environmentOf(place, label)
    if (!environment) {
      return
    }
    const purpose = (event.data as { purpose?: unknown }).purpose === 'document' ? 'document' : 'forwarder'
    const channel = new MessageChannel()
    const binding: Binding = {
      port: channel.port1,
      label,
      environment,
      tab,
      source: event.source,
      purpose,
      cookieWrite: Promise.resolve(),
      bodies: new Map(),
      sockets: new Map(),
      told: new Set(),
    }
    channel.port1.onmessage = (message) => {
      if (purpose === 'forwarder') {
        this.onForwarder(binding, message.data)
      } else {
        this.onDocument(binding, message.data)
      }
    }
    if (purpose === 'document') {
      const key = labelKey(place, label)
      const documents = this.documents.get(key) ?? new Set()
      documents.add(binding)
      this.documents.set(key, documents)
      channel.port1.addEventListener('close', () => documents.delete(binding))
    }
    channel.port1.addEventListener('close', () => {
      for (const body of binding.bodies.values()) {
        body.cancel()
      }
      for (const socket of binding.sockets.values()) {
        socket.close(1001, '')
      }
    })
    reply.postMessage({}, [channel.port2])
    if (purpose === 'document' && tab.frameWindow() === event.source) {
      this.tops.set(tab, binding)
      // A tab's top document hears whether a page opened its tab.
      if (tab.opener) {
        channel.port1.postMessage({ type: 'opener' })
      }
    }
  }

  /** A request a document keeps behind a token: a form submitted to another origin. */
  private async keep(event: RelayMessage): Promise<void> {
    const reply = event.ports[0]
    const tab = this.tabOf(event.source)
    const place = tab?.place()
    const label = place ? labelOfOrigin(place, event.origin) : null
    const parsed = keptRequestSchema.safeParse((event.data as { request?: unknown }).request)
    if (!reply || !place || !label || !parsed.success) {
      return
    }
    const request = parsed.data
    const target = URL.parse(request.url)
    const targetLabel = target && labelOfOrigin(place, target.origin)
    const initiator = await this.environmentOf(place, label)
    if (!targetLabel || !initiator) {
      return
    }
    const token = crypto.randomUUID()
    this.kept.set(token, {
      request: { method: request.method, contentType: request.contentType, body: request.body, user: false },
      initiator,
      label: targetLabel,
    })
    reply.postMessage({ token })
  }

  private onForwarder(binding: Binding, message: { type?: unknown; id?: unknown }): void {
    const id = typeof message.id === 'number' ? message.id : null
    if (id === null) {
      return
    }
    if (message.type === 'fetch') {
      const request = forwardedRequestSchema.safeParse(message)
      if (request.success) {
        void this.forward(binding, request.data)
      } else {
        // A request the forwarder cannot have sent is a network error.
        binding.port.postMessage({ type: 'head', id, error: true })
      }
    } else if (message.type === 'pull') {
      const body = binding.bodies.get(id)
      if (!body) {
        return
      }
      body.pull().then(
        (data) => {
          if (data === null) {
            binding.bodies.delete(id)
            binding.port.postMessage({ type: 'end', id })
          } else {
            const bytes = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength)
            binding.port.postMessage({ type: 'chunk', id, bytes }, [bytes])
          }
        },
        () => {
          binding.bodies.delete(id)
          binding.port.postMessage({ type: 'fail', id })
        },
      )
    } else if (message.type === 'cancel') {
      binding.bodies.get(id)?.cancel()
      binding.bodies.delete(id)
    }
  }

  /** Answers one request of a forwarder; any failure is a network error. */
  private async forward(binding: Binding, message: ForwardedRequest): Promise<void> {
    const fail = () => binding.port.postMessage({ type: 'head', id: message.id, error: true })
    try {
      if (!(await this.answer(binding, message))) {
        fail()
      }
    } catch {
      fail()
    }
  }

  /** Answers `message`; false when it fails as a network error. */
  private async answer(binding: Binding, message: ForwardedRequest): Promise<boolean> {
    const place = binding.tab.place()
    const url = URL.parse(message.url)
    if (!place || !url) {
      return false
    }
    const own = labelOfOrigin(place, url.origin) === binding.label
    const runtime = own ? RUNTIME_PATH.exec(url.pathname) : null
    if (runtime) {
      return this.runtime(binding, message.id, place, runtime[1]!)
    }
    const cookieWrite = own && url.pathname === COOKIE_PATH && message.method === 'POST'
    if (!cookieWrite) {
      // A document's later requests carry the cookie it wrote.
      await binding.cookieWrite
    }
    const navigation = message.mode === 'navigate'
    // Who started it: the request kept behind the token, which only the target label's channel takes; a
    // subresource's document, which is the receiving one; a navigation's referrer, as the boot page announced
    // it or the browser gave it.
    const held = message.token ? this.kept.get(message.token) : undefined
    const kept = held && held.label === binding.label ? held : undefined
    if (kept) {
      this.kept.delete(message.token!)
    }
    const from = navigation ? (message.announcedReferrer ?? message.referrer) : message.referrer
    let initiator: PreviewEnvironment | null
    if (kept) {
      initiator = kept.initiator
    } else if (!navigation) {
      initiator = binding.environment
    } else {
      initiator = await this.environmentOfAddress(place, from)
    }
    const target = await this.environmentOfAddress(place, message.url)
    if (!target) {
      return false
    }
    const user = kept?.request.user ?? false
    // A referrer outside this namespace's previews names nothing the site should see.
    const fromUrl = URL.parse(from)
    const fromEnvironment = !user && fromUrl ? await this.environmentOfAddress(place, from) : null
    const referrer = fromUrl && fromEnvironment ? realAddress(fromUrl, fromEnvironment) : ''
    const method = kept ? kept.request.method : message.method
    const headers: PreviewHeader[] = kept
      ? (kept.request.contentType ? [{ name: 'content-type', value: kept.request.contentType }] : [])
      : message.headers.map(([name, value]) => ({ name, value }))
    const body = kept ? kept.request.body : message.body
    const exchange = binding.tab.connection.request(
      place,
      binding.environment,
      {
        url: realAddress(url, target),
        method,
        headers,
        mode: message.mode,
        destination: message.destination,
        credentials: message.credentials,
        referrer,
        referrerPolicy: message.referrerPolicy,
        keepalive: false,
        initiator,
        user,
      },
      this.client(),
      body ? new Uint8Array(body) : null,
    )
    if (cookieWrite) {
      binding.cookieWrite = exchange.head.catch(() => {})
    }
    let head
    try {
      head = await exchange.head
    } catch (error) {
      if (navigation && binding.tab.frameWindow() === binding.source) {
        binding.tab.report({ type: 'failed', reason: error instanceof Error ? error.message : String(error) })
      }
      return false
    }
    // The browser reads these addresses back as soon as it reads the answer.
    const labels = await this.learn(place, head.labels)
    this.tell(place, binding.label, labels)
    const location = head.headers.find((header) => header.name.toLowerCase() === 'location')?.value
    const hasBody = !EMPTY_STATUSES.includes(head.status) && method !== 'HEAD'
    if (hasBody) {
      binding.bodies.set(message.id, exchange)
    } else {
      // The engine waits for pulls of a body nobody reads.
      exchange.cancel()
    }
    binding.port.postMessage({
      type: 'head',
      id: message.id,
      status: head.status,
      statusText: '',
      headers: head.headers.map((header) => [header.name, header.value]),
      redirect: location && REDIRECTS.includes(head.status) ? location : undefined,
      body: hasBody,
    })
    return true
  }

  /** The runtime of the engine's release, from the web app's build; another release's is a network error. */
  private async runtime(binding: Binding, id: number, place: PreviewPlace, release: string): Promise<boolean> {
    if (release !== place.runtime.release) {
      return false
    }
    let loading = this.runtimes.get(place.runtime.url)
    if (!loading) {
      loading = this.fetchRuntime(place.runtime.url)
      this.runtimes.set(place.runtime.url, loading)
      loading.catch(() => this.runtimes.delete(place.runtime.url))
    }
    const bytes = new Uint8Array(await loading)
    let sent = false
    binding.bodies.set(id, {
      pull: () => {
        const data = sent ? null : bytes
        sent = true
        return Promise.resolve(data)
      },
      cancel: () => {},
    })
    binding.port.postMessage({
      type: 'head',
      id,
      status: 200,
      statusText: '',
      headers: [['content-type', 'text/javascript'], ['cache-control', 'no-cache']],
      body: true,
    })
    return true
  }

  /** Tells the documents of `label` the labels its answers mapped, before they read them back. */
  private tell(place: PreviewPlace, label: string, labels: string[]): void {
    for (const binding of this.documents.get(labelKey(place, label)) ?? []) {
      const entries: Record<string, PreviewEnvironment> = {}
      for (const name of labels) {
        const environment = this.labels.get(labelKey(place, name))
        if (environment && !binding.told.has(name)) {
          binding.told.add(name)
          entries[name] = environment
        }
      }
      if (Object.keys(entries).length > 0) {
        binding.port.postMessage({ type: 'labels', entries })
      }
    }
  }

  /** A document's own channel: its labels, WebSockets, windows and what its tab shows. */
  private onDocument(binding: Binding, data: unknown): void {
    const place = binding.tab.place()
    const parsed = documentMessageSchema.safeParse(data)
    // A message the runtime cannot have sent changes nothing.
    if (!place || !parsed.success) {
      return
    }
    const message = parsed.data
    const top = binding.tab.frameWindow() === binding.source
    switch (message.type) {
      case 'labels':
        void this.learn(place, message.entries)
        return
      case 'label': {
        const label = message.label
        void this.environmentOf(place, label).then((environment) =>
          binding.port.postMessage({ type: 'label', label, environment }),
        )
        return
      }
      case 'socket-open':
        this.openSocket(binding, place, message.id, message.url, message.protocols)
        return
      case 'socket-send':
        binding.sockets.get(message.id)?.send(typeof message.data === 'string' ? message.data : new Uint8Array(message.data))
        return
      case 'socket-close':
        binding.sockets.get(message.id)?.close(message.code, message.reason)
        return
      case 'tab-page':
        if (top) {
          binding.tab.report({ type: 'page', page: message.page })
        }
        return
      case 'tab-leaving':
        if (top) {
          if (this.tops.get(binding.tab) === binding) {
            this.tops.delete(binding.tab)
          }
          binding.tab.report({ type: 'leaving' })
        }
        return
      default:
        this.onWindow(binding, message)
    }
  }

  /** A page's socket, opened upstream from the Host. */
  private openSocket(binding: Binding, place: PreviewPlace, id: number, url: string, protocols: string[]): void {
    const reply = (data: Record<string, unknown>, transfer: Transferable[] = []) =>
      binding.port.postMessage({ ...data, id }, transfer)
    const socket = binding.tab.connection.socket(
      place,
      binding.environment,
      url,
      protocols,
      this.client(),
      {
        opened: (protocol, extensions) => reply({ type: 'socket-open', protocol, extensions }),
        message: (data) => {
          if (typeof data === 'string') {
            reply({ type: 'socket-message', data })
          } else {
            const bytes = data.buffer.slice(data.byteOffset, data.byteOffset + data.byteLength)
            reply({ type: 'socket-message', data: bytes }, [bytes])
          }
        },
        closed: (code, reason) => {
          binding.sockets.delete(id)
          // A socket that failed reports an error before it closes, as the browser's does.
          if (code === 1006) {
            reply({ type: 'socket-error' })
          }
          reply({ type: 'socket-close', code, reason, wasClean: code !== 1006 })
        },
      },
    )
    binding.sockets.set(id, socket)
  }

  /** Windows: the tabs a page opens, its messages to them, and its own tab's navigation. */
  private onWindow(binding: Binding, message: WindowMessage): void {
    const tab = binding.tab
    const initiator = binding.environment
    /** The tab the page opened as its popup `id`. */
    const popup = (id: number) =>
      [...this.tabs].find((candidate) => candidate.opener?.binding === binding && candidate.opener.popup === id)
    switch (message.type) {
      case 'tab-open':
        tab.openWindow(binding, message.id, message.url ? { url: message.url, initiator } : null)
        return
      case 'tab-navigate':
        popup(message.id)?.navigate({ url: message.url, initiator })
        return
      case 'tab-close':
        popup(message.id)?.close()
        return
      case 'tab-post': {
        const target = popup(message.id)
        const top = target && this.tops.get(target)
        if (top && reaches(message.targetOrigin, binding, top.environment)) {
          top.port.postMessage({ type: 'opener-message', data: message.data, origin: initiator.origin, target: top.environment.origin })
        }
        return
      }
      case 'opener-post': {
        const opener = tab.opener
        if (opener && reaches(message.targetOrigin, binding, opener.binding.environment)) {
          opener.binding.port.postMessage({ type: 'tab-message', id: opener.popup, data: message.data, origin: initiator.origin })
        }
        return
      }
      case 'opener-navigate':
        tab.opener?.binding.tab.navigate({ url: message.url, initiator })
        return
      case 'tab-navigate-self':
        tab.navigate({ url: message.url, initiator })
        return
      case 'close-self':
        tab.close()
        return
    }
  }

  /** The environment of `tab`'s top document, while it has one. */
  topEnvironment(tab: RelayTab): PreviewEnvironment | null {
    return this.tops.get(tab)?.environment ?? null
  }

  /**
   * Asks `tab`'s top document's runtime for what the tab's bar does in the
   * page itself: Back, Forward, Reload and Stop. False when the tab shows
   * no document of the preview yet.
   */
  command(tab: RelayTab, command: 'back' | 'forward' | 'reload' | 'stop'): boolean {
    const top = this.tops.get(tab)
    if (!top) {
      return false
    }
    top.port.postMessage({ type: 'tab-command', command })
    return true
  }
}

/** The parent of a window, or null for anything that has none. */
function parentOf(window: unknown): unknown {
  return typeof window === 'object' && window !== null && 'parent' in window ? window.parent : null
}

/** Whether a message targeted at `targetOrigin` by `sender` reaches a document of `environment`. */
function reaches(targetOrigin: string, sender: RelayBinding, environment: PreviewEnvironment): boolean {
  const target = targetOrigin === '/' ? sender.environment.origin : targetOrigin
  return target === '*' || target === environment.origin
}

/** The relay of this page. */
let pageRelay: PreviewRelay | null = null

export function relay(): PreviewRelay {
  pageRelay ??= new PreviewRelay(window)
  return pageRelay
}

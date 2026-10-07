// What the page logged, requested, and sent or received on its sockets
// since the tool attached to the browser (browse.md § Watching), kept in the
// daemon so that `log` finds what happened before anyone asked. A daemon
// that ends for changed code leaves them in the slot's folder for the next
// one, which attaches to the same browser and goes on from them.
import { existsSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import type { BrowserContext, Page, Request, WebSocket } from 'playwright'
import { z } from 'zod'

export const LOG_KINDS = ['console', 'network', 'sockets'] as const
export type LogKind = typeof LOG_KINDS[number]

const entrySchema = z.object({
  sequence: z.number().int(),
  /** Milliseconds since the tool attached to the browser. */
  at: z.number(),
  text: z.string(),
  /** Whether a network entry is the dev server's module traffic, which `log network` leaves out. */
  module: z.boolean(),
})
type Entry = z.infer<typeof entrySchema>

/** The logs a daemon leaves for the next, with the browser they were kept for. */
const savedSchema = z.object({
  /** The DevTools endpoint of the browser the logs are of. */
  browser: z.string(),
  started: z.number(),
  sequence: z.number().int(),
  markAt: z.number().int(),
  markName: z.string().nullable(),
  entries: z.object({ console: z.array(entrySchema), network: z.array(entrySchema), sockets: z.array(entrySchema) }),
})

/** How many entries each kind keeps; older ones go. */
const LIMIT = 20_000

/** How much of a socket frame an entry keeps. */
const FRAME_TEXT = 300

export class Logs {
  private entries: Record<LogKind, Entry[]> = { console: [], network: [], sockets: [] }
  private sequence = 0
  private started = Date.now()
  /** The sequence `log mark` set; entries after it are the ones `log` prints. */
  private markAt = 0
  private markName: string | null = null

  /** Starts over for a new browser. */
  reset(): void {
    for (const kind of LOG_KINDS) {
      this.entries[kind] = []
    }
    this.started = Date.now()
    this.markAt = 0
    this.markName = null
  }

  /** Leaves the logs of the browser at `browser` in the file `path`, for the next daemon. */
  save(path: string, browser: string): void {
    const saved: z.infer<typeof savedSchema> = {
      browser,
      started: this.started,
      sequence: this.sequence,
      markAt: this.markAt,
      markName: this.markName,
      entries: this.entries,
    }
    writeFileSync(path, JSON.stringify(saved))
  }

  /**
   * Goes on from the logs a daemon left in `path` when they are of the
   * browser at `browser`, and starts over otherwise; the file is used once.
   */
  restore(path: string, browser: string): void {
    this.reset()
    if (!existsSync(path)) {
      return
    }
    const saved = savedSchema.parse(JSON.parse(readFileSync(path, 'utf8')))
    rmSync(path)
    if (saved.browser !== browser) {
      return
    }
    this.entries = saved.entries
    this.started = saved.started
    this.sequence = saved.sequence
    this.markAt = saved.markAt
    this.markName = saved.markName
  }

  add(kind: LogKind, text: string, module = false): void {
    this.sequence += 1
    const list = this.entries[kind]
    list.push({ sequence: this.sequence, at: Date.now() - this.started, text, module })
    if (list.length > LIMIT) {
      list.shift()
    }
  }

  mark(name: string | null): string {
    this.markAt = this.sequence
    this.markName = name
    return name ?? `at ${formatAt(Date.now() - this.started)}`
  }

  /** The entries of `kind` since the mark, or since the daemon attached to the browser when `all`. */
  read(kind: LogKind, options: { all: boolean, modules: boolean }): string[] {
    const since = options.all ? 0 : this.markAt
    return this.entries[kind]
      .filter((entry) => entry.sequence > since && (options.modules || !entry.module))
      .map((entry) => `${formatAt(entry.at)}  ${entry.text}`)
  }

  /** What the reader sees as the start of `read`'s entries. */
  since(all: boolean): string {
    if (all || this.markAt === 0) {
      return 'since the tool attached to the browser'
    }
    return this.markName ? `since the mark ${this.markName}` : 'since the mark'
  }

  /** Records what `context` and each of its pages do from now on. */
  watch(context: BrowserContext): void {
    for (const page of context.pages()) {
      this.watchPage(page)
    }
    context.on('page', (page) => this.watchPage(page))
    const started = new Map<Request, number>()
    context.on('request', (request) => started.set(request, Date.now()))
    context.on('requestfinished', (request) => {
      const took = Date.now() - (started.get(request) ?? Date.now())
      started.delete(request)
      // The response is there once a request finished.
      void request.response().then((response) => {
        const status = response?.status() ?? '-'
        this.add('network', `${request.method()} ${status} ${request.url()} ${took} ms`, isModule(request))
      })
    })
    context.on('requestfailed', (request) => {
      started.delete(request)
      const reason = request.failure()?.errorText ?? 'failed'
      this.add('network', `${request.method()} FAILED ${request.url()} ${reason}`, isModule(request))
    })
  }

  private watchPage(page: Page): void {
    page.on('console', (message) => this.add('console', `${message.type()}: ${message.text()}`))
    page.on('pageerror', (error) => this.add('console', `uncaught: ${error.message}`))
    page.on('websocket', (socket) => this.watchSocket(socket))
  }

  private watchSocket(socket: WebSocket): void {
    const url = new URL(socket.url())
    const name = url.pathname + url.search
    this.add('sockets', `open ${name}`)
    socket.on('framesent', (frame) => this.add('sockets', `sent ${name} ${frameText(frame.payload)}`))
    socket.on('framereceived', (frame) => this.add('sockets', `received ${name} ${frameText(frame.payload)}`))
    socket.on('socketerror', (error) => this.add('sockets', `error ${name} ${error}`))
    socket.on('close', () => this.add('sockets', `closed ${name}`))
  }
}

function frameText(payload: string | Buffer): string {
  if (typeof payload !== 'string') {
    return `<${payload.length} bytes>`
  }
  return payload.length > FRAME_TEXT ? `${payload.slice(0, FRAME_TEXT)}… (${payload.length} characters)` : payload
}

/**
 * Whether `request` loads the page's own code from a dev server, such as
 * Vite's modules, styles and fonts: hundreds of them drown what a check
 * looks for.
 */
function isModule(request: Request): boolean {
  const type = request.resourceType()
  if (['script', 'stylesheet', 'font', 'image', 'manifest'].includes(type)) {
    return true
  }
  const path = new URL(request.url()).pathname
  return path.startsWith('/@') || path.startsWith('/node_modules/') || path.startsWith('/src/')
}

function formatAt(ms: number): string {
  return `${(ms / 1000).toFixed(3).padStart(9)} s`
}

// What the page logged, requested, and sent or received on its sockets
// since the tool attached to the browser (browse.md § Watching), kept in the
// daemon so that `log` finds what happened before anyone asked. A page that
// reaches a runner over a direct channel (`direct-channel.md`) runs its file
// operations and its file watch on data channels, which no network event
// shows: a script in the page tells the console about them, and they are
// kept with the requests and the sockets. A daemon
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

/** What starts a console line the page's direct channel script writes, followed by its log kind. */
const DIRECT_MARK = '[browse direct]'

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
  async watch(context: BrowserContext): Promise<void> {
    await context.addInitScript(recordDirectChannels, DIRECT_MARK)
    for (const page of context.pages()) {
      this.watchPage(page)
      // A page open before the tool attached gets the script now; one a
      // daemon before this one gave it keeps it.
      await page.evaluate(recordDirectChannels, DIRECT_MARK).catch(() => undefined)
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
    page.on('console', (message) => this.console(`${message.type()}: ${message.text()}`))
    page.on('pageerror', (error) => this.add('console', `uncaught: ${error.message}`))
    page.on('websocket', (socket) => this.watchSocket(socket))
  }

  /** A line of the page's console, or of its direct channel script, which names the kind it belongs to. */
  console(line: string): void {
    const direct = new RegExp(`^debug: ${RegExp.escape(DIRECT_MARK)} (network|sockets) (.*)$`, 's').exec(line)
    if (!direct) {
      this.add('console', line)
      return
    }
    const [, kind, text] = direct
    this.add(kind === 'network' ? 'network' : 'sockets', kind === 'sockets' ? frameText(text) : text)
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

/**
 * Runs in the page, so it uses nothing outside itself: tells the console,
 * after `mark`, about each data channel the page opens to a runner
 * (`direct-channel.md` § Operations on the channel). The runner's answer
 * goes with the requests, as `direct <op> <ok or error code> <path> <ms>
 * ms`; a watch's messages after its header and answer, such as `paths` and
 * `changed`, go with the sockets, as the relay's watch socket's do. A file's
 * text or bytes are not logged. Installed once per page.
 */
export function recordDirectChannels(mark: string): void {
  const prototype = globalThis.RTCPeerConnection?.prototype
  if (!prototype || Object.hasOwn(prototype, 'browseRecordsDirectChannels')) {
    return
  }
  Object.defineProperty(prototype, 'browseRecordsDirectChannels', { value: true })
  const create = prototype.createDataChannel
  prototype.createDataChannel = function (this: RTCPeerConnection, ...args: Parameters<RTCPeerConnection['createDataChannel']>) {
    const channel = create.apply(this, args)
    const began = performance.now()
    /** What the page's first message names: the operation, and its path or stream. */
    let header: { op: string, path: string } | null = null
    let answered = false
    const send = channel.send
    channel.send = function (this: RTCDataChannel, ...data: unknown[]) {
      const [text] = data
      if (typeof text === 'string') {
        if (header === null) {
          let op = channel.label
          let path = ''
          try {
            const value: unknown = JSON.parse(text)
            if (value && typeof value === 'object') {
              op = 'op' in value && typeof value.op === 'string' ? value.op : op
              path = 'path' in value && typeof value.path === 'string' ? value.path : 'stream' in value && typeof value.stream === 'string' ? value.stream : ''
            }
          } catch {
            // A header that is not JSON is named by its channel alone.
          }
          header = { op, path }
        } else if (header.op === 'watch') {
          console.debug(`${mark} sockets sent direct watch ${text}`)
        }
      }
      Reflect.apply(send, channel, data)
    }
    /** The request entry of the channel's operation, as `log network` prints a request's. */
    const request = (answer: string) => {
      const op = header?.op ?? channel.label
      const path = header?.path ? ` ${header.path}` : ''
      console.debug(`${mark} network direct ${op} ${answer}${path} ${Math.round(performance.now() - began)} ms`)
    }
    channel.addEventListener('message', (event: MessageEvent) => {
      if (typeof event.data !== 'string') {
        return
      }
      if (answered) {
        if (header?.op === 'watch') {
          console.debug(`${mark} sockets received direct watch ${event.data}`)
        }
        return
      }
      answered = true
      let answer = 'answered'
      try {
        const value: unknown = JSON.parse(event.data)
        if (value && typeof value === 'object' && 'ok' in value) {
          answer = 'ok'
        } else if (value && typeof value === 'object' && 'error' in value && value.error && typeof value.error === 'object' && 'code' in value.error) {
          answer = String(value.error.code)
        }
      } catch {
        // An answer that is not JSON is told as answered.
      }
      request(answer)
    })
    channel.addEventListener('close', () => {
      if (!answered) {
        request('FAILED')
      }
    })
    return channel
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

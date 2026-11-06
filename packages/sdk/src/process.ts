/**
 * The process a program runs Demi's cores in, `demi-core`, and the HTTP/2
 * connection the SDK speaks to it on its standard input and output
 * (`crates/core-api/src/process/mod.rs`).
 *
 * ```text
 * program                          demi-core
 *   request(GET /cores/x/api/state) --> stream 1
 *   connect(/cores/x/api/connect,      stream 3, CONNECT :protocol websocket
 *           websocket) ---------------> the page connection's frames
 *   connect(/cores/x/api/devices/      stream 5, CONNECT :protocol h2c
 *           d/runner, h2c) -----------> a runner's channel
 * ```
 *
 * The process logs to its standard error, which goes to the program's. Its
 * connection's end closes every core, and the process exits.
 */
import { spawn, type ChildProcess } from 'node:child_process'
import http2 from 'node:http2'
import { Duplex, Readable } from 'node:stream'

/** How the process starts. */
export interface ProcessStart {
  /** The `demi-core` executable. */
  executable: string
  /** The release roots whose command packages the cores' runners run. */
  release: readonly string[]
  /** Where the program's server forwards the process's routes. */
  publicUrl?: string
}

/** The headers a stream of HTTP/2 does not carry, which a forwarded request drops. */
const CONNECTION_HEADERS = new Set(['connection', 'host', 'keep-alive', 'proxy-connection', 'transfer-encoding', 'upgrade', 'http2-settings'])

/** A running `demi-core`, and its connection. */
export class Process {
  private readonly session: http2.ClientHttp2Session
  /** Resolves once the process's settings came, which say it takes an extended CONNECT. */
  private readonly settled: Promise<void>
  /** Resolves with the process's exit status once it exited. */
  readonly exited: Promise<number | null>

  private constructor(private readonly child: ChildProcess) {
    // Node takes a pair of its own streams, which its types do not say.
    const stdio = Duplex.from({ readable: child.stdout!, writable: child.stdin! } as unknown as Parameters<typeof Duplex.from>[0])
    this.session = http2.connect('http://demi-core', { createConnection: () => stdio })
    this.settled = new Promise((resolve, reject) => {
      this.session.once('remoteSettings', () => resolve())
      this.session.once('close', () => reject(new Error('The process ended before it answered')))
    })
    // Its failure is each connection's to report.
    this.settled.catch(() => {})
    // A failed session ends every request on it, which says why.
    this.session.on('error', () => {})
    this.exited = new Promise((resolve) => child.once('exit', (code) => resolve(code)))
    // Node keeps a session whose socket ended open until its streams close,
    // and a runner's channel on it would stay open with its runner: the
    // process's exit ends them all.
    void this.exited.then(() => this.session.destroy())
  }

  /** Starts the process. */
  static start(start: ProcessStart): Process {
    const env: NodeJS.ProcessEnv = { ...process.env, DEMI_RELEASE: start.release.join(pathListSeparator()) }
    if (start.publicUrl !== undefined) {
      env.DEMI_PUBLIC_URL = start.publicUrl
    }
    const child = spawn(start.executable, [], { stdio: ['pipe', 'pipe', 'inherit'], env })
    return new Process(child)
  }

  /**
   * Answers `request`, whose URL's path and query name the process's route,
   * as `fetch` answers one: the response's body streams as it comes.
   */
  async fetch(request: Request): Promise<Response> {
    const url = new URL(request.url)
    const headers: http2.OutgoingHttpHeaders = {
      ':method': request.method,
      ':path': url.pathname + url.search,
    }
    request.headers.forEach((value, name) => {
      if (!CONNECTION_HEADERS.has(name)) {
        headers[name] = value
      }
    })
    const stream = this.session.request(headers, { endStream: request.body === null })
    // A stream that fails closes, which its reader sees; Bun also reports
    // each stream an ended session cancels as an error.
    stream.on('error', () => {})
    if (request.body !== null) {
      Readable.fromWeb(request.body as import('node:stream/web').ReadableStream).pipe(stream)
    }
    const abort = () => stream.close(http2.constants.NGHTTP2_CANCEL)
    request.signal.addEventListener('abort', abort, { once: true })
    stream.once('close', () => request.signal.removeEventListener('abort', abort))
    const answer = await answered(stream)
    const status = Number(answer[':status'])
    const responseHeaders = new Headers()
    for (const [name, value] of Object.entries(answer)) {
      if (name.startsWith(':') || value === undefined) {
        continue
      }
      for (const each of Array.isArray(value) ? value : [value]) {
        responseHeaders.append(name, String(each))
      }
    }
    const empty = request.method === 'HEAD' || status === 204 || status === 304
    if (empty) {
      stream.resume()
    }
    return new Response(empty ? null : webBody(stream), { status, headers: responseHeaders })
  }

  /**
   * Opens `path` by an extended CONNECT of `protocol` (RFC 8441), such as
   * `websocket` or `h2c`, and answers the stream once the process took it.
   */
  async connect(path: string, protocol: string, headers: http2.OutgoingHttpHeaders = {}): Promise<http2.ClientHttp2Stream> {
    await this.settled
    const stream = this.session.request({
      ...headers,
      ':method': 'CONNECT',
      ':protocol': protocol,
      ':scheme': 'http',
      ':path': path,
    })
    stream.on('error', () => {})
    const answer = await answered(stream)
    const status = Number(answer[':status'])
    if (status !== 200) {
      throw new ProcessRefusal(status, await new Response(webBody(stream)).text())
    }
    return stream
  }

  /** Ends the connection, which closes every core, and waits for the process to exit. */
  async close(): Promise<void> {
    if (!this.session.closed) {
      this.session.close()
    }
    this.child.stdin?.end()
    await this.exited
  }
}

/** The process refused an extended CONNECT: its status, and what it said. */
export class ProcessRefusal extends Error {
  constructor(readonly status: number, readonly body: string) {
    super(`The process refused the connection with ${status}: ${body}`)
    this.name = 'ProcessRefusal'
  }
}

/** The headers `stream` is answered with; throws when it closes unanswered. */
function answered(stream: http2.ClientHttp2Stream): Promise<http2.IncomingHttpHeaders> {
  return new Promise((resolve, reject) => {
    const closed = () => reject(new Error(`The process closed the stream unanswered (${stream.rstCode ?? 'no code'})`))
    stream.once('close', closed)
    stream.once('response', (headers) => {
      stream.off('close', closed)
      resolve(headers)
    })
  })
}

/**
 * The body `stream` brings, as a web stream that reads it as it is read and
 * cancels it once cancelled. Node's `Readable.toWeb` is not used: it throws
 * from its listener when data comes after the reader cancelled, which
 * crashes the program.
 */
function webBody(stream: http2.ClientHttp2Stream): ReadableStream<Uint8Array> {
  let settled = false
  return new ReadableStream<Uint8Array>({
    start(controller) {
      stream.pause()
      stream.on('data', (chunk: Buffer) => {
        if (settled) {
          return
        }
        controller.enqueue(new Uint8Array(chunk.buffer, chunk.byteOffset, chunk.byteLength))
        if ((controller.desiredSize ?? 0) <= 0) {
          stream.pause()
        }
      })
      stream.once('end', () => {
        if (!settled) {
          settled = true
          controller.close()
        }
      })
      stream.once('close', () => {
        if (!settled) {
          settled = true
          controller.error(new Error(`The process reset the stream (${stream.rstCode})`))
        }
      })
    },
    pull() {
      stream.resume()
    },
    cancel() {
      settled = true
      stream.close(http2.constants.NGHTTP2_CANCEL)
    },
  })
}

/** What separates the paths of a list, as `PATH` writes them. */
function pathListSeparator(): string {
  return process.platform === 'win32' ? ';' : ':'
}

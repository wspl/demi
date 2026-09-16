import type { Context } from 'hono'
import type { UpgradeWebSocket } from 'hono/ws'
import type { Server } from 'bun'
import { RemoteNetError, type RemoteNet } from '@demicodes/host-remote'
import type { ConversationTargets } from '../conversation/target'
import type { PipeBroker, PipeWriter } from '../runner/pipes'
import type { Exposes } from './records'
import {
  CHUNKED_EOF,
  CHUNK_END,
  HttpResponseParser,
  chunkHeader,
  isHopByHop,
  serializeRequestHead
} from './http1'
import {
  WS_BINARY,
  WS_CLOSE,
  WS_PONG,
  WS_TEXT,
  WsFrameParser,
  encodeClientFrame,
  encodeClosePayload,
  wsHandshake,
  wsKey,
  type ServerFrame
} from './ws'

/** A connection with no bytes in either direction is closed after this. */
const IDLE_TIMEOUT_MS = 10 * 60_000

/** What the relay talks to the service with: both ends of the stream. */
interface RelayStream {
  writer: PipeWriter
  source: AsyncIterable<Uint8Array>
  /** Fails both pipes: the abnormal ending of the connection. */
  dispose(reason: string): void
}

/** The two endings of one relayed connection. */
interface ConnectionEndings {
  /** Abnormal: the pipes fail, the runner closes its socket at once. */
  end(): void
  /**
   * The exchange completed: the input pipe already ended (EOF half-closes
   * the socket's write side) and the output pipe settles at its own EOF, so
   * nothing is torn down — only the relay's bookkeeping closes.
   */
  settle(): void
}

/**
 * The public relay (`expose.md` § The public relay): every request whose
 * `Host` header is `<id>.<expose domain>` — every path, every method, before
 * the product routes. Carries HTTP/1.1 and WebSocket bytes to the exposed
 * address over a network stream on the device; nothing about Demi is visible
 * to the page or its visitors.
 */
export class ExposeRelay {
  constructor(private readonly deps: {
    exposes: Exposes
    targets: ConversationTargets
    pipes: PipeBroker
    upgradeWebSocket: UpgradeWebSocket
    idleTimeoutMs?: number
  }) {}

  /** The expose id an expose hostname carries, or null. */
  idFromHostHeader(host: string | undefined): string | null {
    const domain = this.deps.exposes.domain
    if (!domain || !host)
      return null
    const name = host.split(':')[0]!.toLowerCase()
    const suffix = `.${domain.toLowerCase()}`
    if (!name.endsWith(suffix))
      return null
    const id = name.slice(0, -suffix.length)
    return id.length > 0 && !id.includes('.') ? id : null
  }

  async handle(c: Context): Promise<Response> {
    const record = await this.deps.exposes.liveRecord(
      this.idFromHostHeader(c.req.header('host'))!
    )
    if (!record)
      return notFound()
    const host = await this.deps.targets.deviceHost(
      record.userId,
      record.deviceId
    )
    if (!host)
      return badGateway('device_offline')
    const controller = new AbortController()
    const release = this.deps.exposes.begin(record.id, () =>
      controller.abort(new Error('expose destroyed'))
    )
    if (!release)
      return limitReached()
    let stream: RelayStream | null = null
    let idle: IdleWatch | null = null
    let done = false
    // One owner for the connection's whole life: the response may stream long
    // after `handle` returns, so nothing is released on the way out.
    const stop = (teardown: boolean) => () => {
      if (done)
        return
      done = true
      if (teardown)
        stream?.dispose('visitor connection ended')
      idle?.close()
      release()
    }
    const endings: ConnectionEndings = { end: stop(true), settle: stop(false) }
    try {
      stream = await this.openStream(record, host.net)
      idle = new IdleWatch(this.deps.idleTimeoutMs ?? IDLE_TIMEOUT_MS, () => {
        controller.abort(new Error('relay connection idle'))
        endings.end()
      })
      if (c.req.header('upgrade')?.toLowerCase() === 'websocket')
        return await this.relayWebSocket(c, record, stream, idle, endings)
      return await this.relayHttp(c, record, stream, idle, endings)
    } catch (error) {
      endings.end()
      if (controller.signal.aborted)
        return aborted()
      return badGateway(error instanceof RemoteNetError ? error.code : 'unreachable')
    }
  }

  /**
   * One network stream (`runner.md` § Network streams): the visitor's bytes
   * through the input pipe's writer, the service's from the output pipe's
   * stream, both ends fixed to the device before `net_open`.
   */
  private async openStream(
    record: { deviceId: string; address: string },
    net: RemoteNet
  ): Promise<RelayStream> {
    const [host, port] = splitAddress(record.address)
    const input = this.deps.pipes.open(undefined, { deviceId: record.deviceId })
    const output = this.deps.pipes.open({ deviceId: record.deviceId })
    // Disposal at the connection's abnormal end rejects settled pipes; a
    // rejection no one observes is an expected ending here.
    input.done.catch(() => {})
    output.done.catch(() => {})
    try {
      await net.open({ host, port, input: input.ref(), output: output.ref() })
    } catch (error) {
      this.deps.pipes.fail(input.id, 'net open failed')
      this.deps.pipes.fail(output.id, 'net open failed')
      throw error
    }
    return {
      writer: input.writer(),
      source: output.stream(),
      dispose: (reason) => {
        this.deps.pipes.fail(input.id, reason)
        this.deps.pipes.fail(output.id, reason)
      },
    }
  }

  /** Step 4 and the header rewriting of `expose.md` § The public relay. */
  private async relayHttp(
    c: Context,
    record: { address: string },
    stream: RelayStream,
    idle: IdleWatch,
    endings: ConnectionEndings
  ): Promise<Response> {
    const request = c.req.raw
    const url = new URL(request.url)
    const headers: Array<[string, string]> = []
    for (const [name, value] of request.headers) {
      const lower = name.toLowerCase()
      if (isHopByHop(lower) || lower === 'host' || lower === 'content-length')
        continue
      headers.push([name, value])
    }
    headers.push(['Host', record.address])
    headers.push(['X-Forwarded-For', visitorIp(c) ?? 'unknown'])
    headers.push(['X-Forwarded-Host', request.headers.get('host') ?? ''])
    headers.push(['X-Forwarded-Proto', url.protocol.replace(':', '')])
    headers.push(['Connection', 'close'])
    if (request.body)
      headers.push(['Transfer-Encoding', 'chunked'])
    const writer = new TouchingWriter(stream.writer, idle)
    await writer.write(
      serializeRequestHead(request.method, url.pathname + url.search, headers)
    )
    // The exchange's completion is the request's true end: the input pipe
    // ends here (EOF half-closes the service socket) once the response is
    // fully parsed, never while it may still be streaming.
    const settleExchange = () => {
      stream.writer.end()
      endings.settle()
    }
    const response = new Promise<Response>((resolve, reject) => {
      // The body is delivered only from inside the stream's own pulls: this
      // Bun serves no response whose stream is fed from the outside.
      const body = new BodyQueue()
      const parser = new HttpResponseParser(
        head => {
          const responseHeaders: Array<[string, string]> = []
          for (const [name, value] of head.headers) {
            const lower = name.toLowerCase()
            if (isHopByHop(lower) || lower === 'content-length')
              continue
            responseHeaders.push([name, value])
          }
          resolve(new Response(
            body.stream,
            { status: head.status, headers: responseHeaders }
          ))
        },
        chunk => {
          idle.touch()
          body.enqueue(chunk)
        },
        () => {
          body.close()
          settleExchange()
        }
      )
      void (async () => {
        try {
          for await (const chunk of stream.source) {
            parser.feed(chunk)
          }
          parser.end()
          settleExchange()
        } catch (error) {
          parser.fail(String(error))
          body.fail(error)
          reject(error)
          endings.end()
        }
      })()
    })
    if (request.body) {
      const requestBody = request.body
      void (async () => {
        try {
          for await (const chunk of requestBody) {
            await writer.write(chunkHeader(chunk.byteLength))
            await writer.write(chunk)
            await writer.write(CHUNK_END)
          }
          await writer.write(CHUNKED_EOF)
          // The request's last byte half-closes the service's socket: the
          // input pipe ending is the runner's EOF (`runner.md` § Network
          // streams). A bodiless request waits for the exchange's end: this
          // Bun aborts a still-streaming response on the client's FIN.
          stream.writer.end()
        } catch {
          // The stream's failure ends the exchange; the visitor sees the
          // response end rather than a hang.
          endings.end()
        }
      })()
    }
    return response
  }

  /** Step 5: the WebSocket upgrade, relayed message by message. */
  private async relayWebSocket(
    c: Context,
    record: { address: string },
    stream: RelayStream,
    idle: IdleWatch,
    endings: ConnectionEndings
  ): Promise<Response> {
    const url = new URL(c.req.url)
    const writer = new TouchingWriter(stream.writer, idle)
    await writer.write(
      wsHandshake(url.pathname + url.search, record.address, wsKey())
    )
    // The service's handshake answer arrives before any frame. The iterator
    // stays alive across it: returning it early would cancel the stream.
    const chunks = stream.source[Symbol.asyncIterator]()
    const nextChunk = async (): Promise<Uint8Array | null> => {
      const next = await chunks.next()
      return next.done ? null : next.value
    }
    let pending: Uint8Array = new Uint8Array(0)
    for (;;) {
      const chunk = await nextChunk()
      if (chunk === null) {
        endings.end()
        return badGateway('unreachable')
      }
      pending = concat(pending, chunk)
      const headEnd = indexOfAscii(pending, '\r\n\r\n')
      if (headEnd !== -1) {
        const head = ascii(pending.subarray(0, headEnd))
        if (!/^HTTP\/1\.[01] 101/.test(head)) {
          endings.end()
          return badGateway('refused')
        }
        pending = pending.subarray(headEnd + 4)
        break
      }
    }
    const parser = new WsFrameParser()
    let client: {
      send(data: string | ArrayBuffer | Uint8Array): void
      close(code: number, reason: string): void
    } | null = null
    let closed = false
    const finish = (code: number, reason: string, graceful: boolean) => {
      if (closed)
        return
      closed = true
      client?.close(code, reason)
      if (graceful)
        endings.settle()
      else
        endings.end()
    }
    // The service may already be sending while the visitor's upgrade is still
    // completing; frames wait for `onOpen`, then flush in order.
    const early: ServerFrame[] = []
    const deliver = (frame: ServerFrame) => {
      if (client === null && frame.kind !== 'close') {
        early.push(frame)
        return
      }
      if (frame.kind === 'message') {
        client?.send(frame.opcode === WS_TEXT
          ? new TextDecoder().decode(frame.data)
          : frame.data)
        idle.touch()
      } else if (frame.kind === 'close') {
        // The close handshake: answer the service's close, then EOF.
        void writer
          .write(encodeClientFrame(WS_CLOSE, encodeClosePayload(
            frame.code === 1005 ? 1000 : frame.code,
            frame.reason
          )))
          .then(() => stream.writer.end())
          .catch(() => {})
        finish(frame.code === 1005 ? 1000 : frame.code, frame.reason, true)
      } else if (frame.kind === 'ping') {
        void writer.write(encodeClientFrame(WS_PONG, frame.data))
          .catch(() => {})
      }
    }
    void (async () => {
      try {
        if (pending.length > 0)
          for (const frame of parser.feed(pending))
            deliver(frame)
        for (;;) {
          const chunk = await nextChunk()
          if (chunk === null)
            break
          for (const frame of parser.feed(chunk))
            deliver(frame)
        }
      } catch {
        // The stream's end closes the client; nothing else to deliver.
      }
      finish(1006, '', false)
    })()
    // The server runtime offers no socket after the upgrade, so the frames
    // are re-emitted through the instance's own WebSocket upgrade helper.
    const upgraded = await this.deps.upgradeWebSocket(() => ({
      onOpen(_event, ws) {
        client = ws
        for (const frame of early.splice(0))
          deliver(frame)
      },
      onMessage(event) {
        const data = event.data
        const payload = typeof data === 'string'
          ? new TextEncoder().encode(data)
          : new Uint8Array(data as ArrayBuffer)
        const frame = encodeClientFrame(
          typeof data === 'string' ? WS_TEXT : WS_BINARY,
          payload
        )
        void writer.write(frame).catch(() => finish(1006, '', false))
      },
      onClose(event) {
        const code = 'code' in event && typeof event.code === 'number'
          ? event.code
          : 1000
        const reason = 'reason' in event && typeof event.reason === 'string'
          ? event.reason
          : ''
        void writer
          .write(encodeClientFrame(WS_CLOSE, encodeClosePayload(code, reason)))
          .then(() => stream.writer.end())
          .catch(() => {})
        finish(code, reason, true)
      },
    }))(c, async () => {})
    return upgraded ?? badGateway('refused')
  }
}

/** The relayed response body: queued by the parser, drained by the visitor. */
class BodyQueue {
  private chunks: Uint8Array[] = []
  private state: 'open' | 'closed' | 'failed' = 'open'
  private failure: unknown = null
  private wake: (() => void) | null = null
  readonly stream = new ReadableStream<Uint8Array>({
    pull: async (controller) => {
      while (this.chunks.length === 0 && this.state === 'open')
        await new Promise<void>(resolve => {
          this.wake = resolve
        })
      this.wake = null
      const chunk = this.chunks.shift()
      if (chunk !== undefined) {
        controller.enqueue(chunk)
        return
      }
      if (this.state === 'failed')
        throw this.failure
      controller.close()
    },
    cancel: () => {
      this.state = 'closed'
      this.woken()
    },
  })

  enqueue(chunk: Uint8Array): void {
    if (this.state !== 'open')
      return
    this.chunks.push(chunk)
    this.woken()
  }

  close(): void {
    if (this.state !== 'open')
      return
    this.state = 'closed'
    this.woken()
  }

  fail(error: unknown): void {
    if (this.state !== 'open')
      return
    this.state = 'failed'
    this.failure = error
    this.woken()
  }

  private woken(): void {
    this.wake?.()
    this.wake = null
  }
}

/** Every write through the pipe writer awaits its backpressure (`expose.md`),
 * and concurrent writes are serialized: the channel behind the writer loses
 * whichever of two overlapping pushes loses the race. */
class TouchingWriter {
  private chain: Promise<void> = Promise.resolve()

  constructor(
    private readonly writer: PipeWriter,
    private readonly idle: IdleWatch
  ) {}

  write(chunk: Uint8Array): Promise<void> {
    this.chain = this.chain.then(async () => {
      this.idle.touch()
      await this.writer.write(chunk)
      this.idle.touch()
    })
    return this.chain
  }
}

/** No bytes in either direction for the timeout: the connection is closed. */
class IdleWatch {
  private timer: ReturnType<typeof setTimeout> | null = null
  constructor(
    private readonly timeoutMs: number,
    private readonly onFire: () => void
  ) {
    this.touch()
  }

  touch(): void {
    if (this.timer !== null)
      clearTimeout(this.timer)
    this.timer = setTimeout(this.onFire, this.timeoutMs)
  }

  close(): void {
    if (this.timer !== null)
      clearTimeout(this.timer)
    this.timer = null
  }
}

function splitAddress(address: string): [string, number] {
  const index = address.lastIndexOf(':')
  const port = Number(address.slice(index + 1))
  return [address.slice(0, index), port]
}

function visitorIp(c: Context): string | null {
  // Hono carries the Bun server as the fetch handler's second argument.
  const server = c.env as Server<unknown> | undefined
  try {
    return server?.requestIP(c.req.raw)?.address ?? null
  } catch {
    return null
  }
}

function notFound(): Response {
  return new Response(
    'This expose does not exist (anymore); its URL is gone or has expired.\n',
    { status: 404, headers: { 'content-type': 'text/plain; charset=utf-8' } }
  )
}

function badGateway(reason: string): Response {
  return new Response(
    `The exposed service is unreachable (${reason}).\n`,
    { status: 502, headers: { 'content-type': 'text/plain; charset=utf-8' } }
  )
}

function limitReached(): Response {
  return new Response(
    'This expose is at its concurrent-connection limit; retry shortly.\n',
    { status: 503, headers: { 'content-type': 'text/plain; charset=utf-8' } }
  )
}

function aborted(): Response {
  return new Response('This expose was removed while serving.\n', {
    status: 502,
    headers: { 'content-type': 'text/plain; charset=utf-8' },
  })
}

function concat(a: Uint8Array, b: Uint8Array): Uint8Array {
  const out = new Uint8Array(a.length + b.length)
  out.set(a, 0)
  out.set(b, a.length)
  return out
}

function indexOfAscii(haystack: Uint8Array, needle: string): number {
  outer:
  for (let i = 0; i <= haystack.length - needle.length; i++) {
    for (let j = 0; j < needle.length; j++) {
      if (haystack[i + j] !== needle.charCodeAt(j))
        continue outer
    }
    return i
  }
  return -1
}

function ascii(bytes: Uint8Array): string {
  let out = ''
  for (const byte of bytes)
    out += String.fromCharCode(byte)
  return out
}

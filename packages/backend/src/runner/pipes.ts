// The pipe broker (`runner.md` § Pipes): one primitive for every byte
// stream between processes. A pipe has a source and a sink; an end on a
// device is an HTTP exchange (`PUT` from the source device, `GET` by the
// sink device), an end in this process is a stream. The backend pipes one
// into the other and holds nothing beyond what is in flight. An end may be
// named later than the pipe is minted — the relay mints a call's pipes
// before the handler runs, and the handler decides whether it reads and
// writes them itself or attaches them to a job elsewhere.
import type { CommandIO } from '@demicodes/shell'
import { ByteChannel, deferred, errorMessage, noop, type Deferred } from '@demicodes/utils'
import { generateDeviceToken } from './claim-codes'

/** Where a pipe's bytes come from or go, as the wire knows it. */
export interface DeviceEnd {
  deviceId: string
}

export interface Pipe {
  id: string
  /** Origin-relative; runners resolve it against their backend URL. */
  url: string
  /** Resolves once the sink drained the body; rejects when the pipe failed. */
  done: Promise<void>
  /** The wire's name for this pipe. */
  ref(): { id: string; url: string }
  /** The sink is this process: the body, pulled as it is iterated. Fixes the sink on the first pull. */
  stream(): AsyncIterable<Uint8Array>
  /** The source is this process: a writer with backpressure. Fixes the source on the first write or `end`. */
  writer(): PipeWriter
  /** The sink is a device that will `GET`; refused once the sink is fixed. */
  sinkTo(deviceId: string): void
  /** The source is a device that will `PUT`; refused once the source is fixed. */
  sourceFrom(deviceId: string): void
}

export interface PipeWriter {
  write(chunk: Uint8Array): Promise<void>
  /** Ends the body. A no-op when the source turned out to be a device. */
  end(): void
  fail(error: unknown): void
}

type End = { kind: 'device'; deviceId: string } | { kind: 'process' } | null

interface PendingPipe {
  source: End
  sink: End
  /** The body, from the source's `PUT` or from the in-process writer. */
  body: Deferred<ReadableStream<Uint8Array>>
  drained: Deferred<void>
  timer: ReturnType<typeof setTimeout>
  channel: ByteChannel | null
}

export class PipeBroker {
  private readonly pipes = new Map<string, PendingPipe>()

  constructor(private readonly options: { timeoutMs?: number } = {}) {}

  /**
   * Mints a pipe. An end left out is fixed later by `stream` / `writer` (this
   * process) or `sinkTo` / `sourceFrom` (a device). An end that never arrives
   * times the pipe out.
   */
  open(source?: DeviceEnd, sink?: DeviceEnd): Pipe {
    const id = generateDeviceToken()
    const pending: PendingPipe = {
      source: source ? { kind: 'device', deviceId: source.deviceId } : null,
      sink: sink ? { kind: 'device', deviceId: sink.deviceId } : null,
      body: deferred(),
      drained: deferred(),
      timer: setTimeout(() => this.fail(id, 'an end never arrived'), this.options.timeoutMs ?? 120_000),
      channel: null,
    }
    // Unobserved rejections on the two internal promises are expected paths.
    pending.body.promise.catch(noop)
    pending.drained.promise.catch(noop)
    this.pipes.set(id, pending)
    const url = `/api/pipes/${id}`
    return {
      id,
      url,
      done: pending.drained.promise,
      ref: () => ({ id, url }),
      stream: () => this.processSink(id),
      writer: () => this.processSource(id),
      sinkTo: (deviceId) => this.fix(id, 'sink', { kind: 'device', deviceId }),
      sourceFrom: (deviceId) => this.fix(id, 'source', { kind: 'device', deviceId }),
    }
  }

  /** `PUT /api/pipes/:id` by `deviceId`: the body streams to the sink; resolves with the HTTP status once drained. */
  async put(id: string, deviceId: string, body: ReadableStream<Uint8Array> | null): Promise<{ status: number; message: string }> {
    const pipe = this.pipes.get(id)
    if (!pipe || pipe.source?.kind !== 'device' || pipe.source.deviceId !== deviceId) return { status: 404, message: 'no such pipe' }
    if (!body) return { status: 400, message: 'a body is required' }
    pipe.body.resolve(body)
    try {
      await pipe.drained.promise
      return { status: 200, message: 'drained' }
    } catch (error) {
      return { status: 409, message: errorMessage(error) }
    }
  }

  /** `GET /api/pipes/:id` by `deviceId`: the body once the source arrived. Its completion settles the pipe. */
  async get(id: string, deviceId: string): Promise<{ status: 200; body: ReadableStream<Uint8Array> } | { status: number; message: string }> {
    const pipe = this.pipes.get(id)
    if (!pipe || pipe.sink?.kind !== 'device' || pipe.sink.deviceId !== deviceId) return { status: 404, message: 'no such pipe' }
    let body: ReadableStream<Uint8Array>
    try {
      body = await pipe.body.promise
    } catch (error) {
      return { status: 409, message: errorMessage(error) }
    }
    return { status: 200, body: this.settling(id, body) }
  }

  /** Fails every pipe a device is an end of — its connection dropped. */
  deviceGone(deviceId: string): void {
    for (const [id, pipe] of this.pipes) {
      const isEnd = (end: End) => end?.kind === 'device' && end.deviceId === deviceId
      if (isEnd(pipe.source) || isEnd(pipe.sink)) this.fail(id, `device ${deviceId} disconnected`)
    }
  }

  fail(id: string, reason: string): void {
    const pipe = this.pipes.get(id)
    if (!pipe) return
    this.finish(id)
    const error = new Error(`pipe failed: ${reason}`)
    pipe.channel?.fail(error)
    pipe.body.reject(error)
    pipe.drained.reject(error)
  }

  close(): void {
    for (const id of [...this.pipes.keys()]) this.fail(id, 'backend shutting down')
  }

  private fix(id: string, which: 'source' | 'sink', end: NonNullable<End>): void {
    const pipe = this.pipes.get(id)
    if (!pipe) throw new Error('pipe: already settled')
    if (pipe[which] !== null) throw new Error(`pipe: the ${which} is already fixed`)
    pipe[which] = end
  }

  /** The in-process sink: pulls the body chunk by chunk; stopping early counts as drained, the way a closed pipe does. */
  private async *processSink(id: string): AsyncIterable<Uint8Array> {
    const pipe = this.pipes.get(id)
    if (!pipe) throw new Error('pipe: already settled')
    if (pipe.sink === null) pipe.sink = { kind: 'process' }
    else if (pipe.sink.kind !== 'process') throw new Error('pipe: the sink is a device')
    const reader = (await pipe.body.promise).getReader()
    let ended = false
    try {
      for (;;) {
        const next = await reader.read()
        if (next.done) break
        yield next.value
      }
      ended = true
    } finally {
      if (!ended) reader.cancel().catch(noop)
      this.finish(id)
      pipe.drained.resolve()
    }
  }

  /** The in-process source: a bounded channel behind a readable body; the writer waits for the sink's pull. */
  private processSource(id: string): PipeWriter {
    const fixed = (): PendingPipe | null => {
      const pipe = this.pipes.get(id)
      if (!pipe) return null
      if (pipe.source?.kind === 'device') return null
      if (pipe.source === null) {
        pipe.source = { kind: 'process' }
        const channel = new ByteChannel()
        pipe.channel = channel
        const chunks = channel.stream()[Symbol.asyncIterator]()
        pipe.body.resolve(
          new ReadableStream<Uint8Array>({
            pull: async (controller) => {
              const next = await chunks.next()
              if (next.done) controller.close()
              else controller.enqueue(next.value)
            },
            cancel: () => void chunks.return?.(),
          }, { highWaterMark: 0 }),
        )
      }
      return pipe
    }
    return {
      write: async (chunk) => {
        const pipe = fixed()
        if (!pipe) throw new Error('pipe: not writable from this process')
        if (chunk.byteLength > 0) await pipe.channel!.push(chunk)
      },
      end: () => fixed()?.channel?.close(),
      fail: (error) => fixed()?.channel?.fail(error),
    }
  }

  /** Wraps the source body so the pipe settles with the sink's read; a sink stopping early counts as drained. */
  private settling(id: string, body: ReadableStream<Uint8Array>): ReadableStream<Uint8Array> {
    const reader = body.getReader()
    const settle = (): void => {
      const pipe = this.pipes.get(id)
      this.finish(id)
      pipe?.drained.resolve()
    }
    return new ReadableStream<Uint8Array>({
      pull: async (controller) => {
        let next: Awaited<ReturnType<typeof reader.read>>
        try {
          next = await reader.read()
        } catch (error) {
          this.fail(id, errorMessage(error))
          controller.error(error)
          return
        }
        if (next.done) {
          controller.close()
          settle()
          return
        }
        controller.enqueue(next.value)
      },
      cancel: () => {
        reader.cancel().catch(noop)
        settle()
      },
    })
  }

  private finish(id: string): void {
    const pipe = this.pipes.get(id)
    if (!pipe) return
    clearTimeout(pipe.timer)
    this.pipes.delete(id)
  }
}

/**
 * The pipes of a relayed `rpc` call, carried on the handler's `io` so a
 * handler that attaches them to a job elsewhere (`demi host shell`) can
 * name the far ends instead of copying bytes through this process.
 */
export interface RelayedPipes {
  stdin: Pipe | null
  stdout: Pipe
}

const RELAYED_PIPES = Symbol('relayedPipes')

export function withRelayedPipes(io: CommandIO, pipes: RelayedPipes): CommandIO {
  return Object.assign(io, { [RELAYED_PIPES]: pipes })
}

export function relayedPipesOf(io: CommandIO): RelayedPipes | null {
  return (io as CommandIO & { [RELAYED_PIPES]?: RelayedPipes })[RELAYED_PIPES] ?? null
}

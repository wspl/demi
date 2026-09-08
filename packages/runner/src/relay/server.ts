import { listenUnix, type StreamSocket, type UnixListener } from '../machine'
import type { BackendToRunnerMessage, RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { LOCAL, localFrame, localFrames, localInvokeSchema, localWatchSchema, localManageSchema, type LocalInvoke } from '@demicodes/runner-protocol/local'
import { createLoader, type ManifestSource } from '@demicodes/command-loader'
import type { Host } from '@demicodes/shell'
import { decodeUtf8, encodeUtf8, errorMessage, noop, SerialQueue } from '@demicodes/utils'
import type { PipeEnds } from '../pipes'
import { ExecutionContexts, type ExecutionContext } from '../commands/contexts'
import { workerModule } from '../commands/execute'
import { BackendCalls } from './backend-calls'

export interface RelayServerOptions {
  send(message: RunnerToBackendMessage): void
  pipes: PipeEnds
  host: Host
  contexts: ExecutionContexts
  source(context: ExecutionContext): ManifestSource
  socketMode?: number
  manageSecret: string
  drain(): Promise<void>
  stop(): Promise<void>
}
interface Call {
  request: LocalInvoke
  context: ExecutionContext
  data: StreamSocket
  control: StreamSocket | null
  abort: AbortController
  writes: SerialQueue
  input: { resolve(value: IteratorResult<Uint8Array>): void; reject(error: unknown): void } | null
  ended: boolean
  completed: boolean
  watchTimer: ReturnType<typeof setTimeout>
}
export class RelayServer {
  private readonly calls = new Map<string, Call>()
  private readonly sockets = new Set<StreamSocket>()
  private readonly backend: BackendCalls
  private closed = false
  static async listen(path: string, options: RelayServerOptions): Promise<RelayServer> {
    const server = new RelayServer(await listenUnix(path, options.socketMode ?? 0o600), options)
    void server.acceptLoop()
    return server
  }
  private constructor(private readonly listener: UnixListener, private readonly options: RelayServerOptions) {
    this.backend = new BackendCalls(options.send, options.pipes)
  }
  close(): void {
    this.closed = true
    this.listener.close()
    this.connectionLost()
    for (const socket of this.sockets) socket.close()
  }
  connectionLost(): void {
    for (const call of this.calls.values()) this.cancel(call)
    this.backend.close()
  }
  cancelJob(jobId: string): void {
    for (const call of this.calls.values()) if (call.context.jobId === jobId) this.cancel(call)
  }
  cancelOwner(owner: string): void {
    for (const call of this.calls.values()) if (call.context.owner === owner) this.cancel(call)
  }
  handleReply(message: Extract<BackendToRunnerMessage, { type: 'rpc_pipes' | 'rpc_output' | 'rpc_exit' }>): void { this.backend.handleReply(message) }
  private cancel(call: Call): void {
    this.calls.delete(call.request.id)
    clearTimeout(call.watchTimer)
    call.abort.abort(new Error('command cancelled'))
    call.input?.reject(call.abort.signal.reason)
    call.input = null
    call.data.close(); call.control?.close()
  }
  private write(call: Call, type: number, body?: Uint8Array): Promise<void> {
    return call.writes.run(() => call.data.write(localFrame(type, body)))
  }
  private async output(call: Call, type: number, value: string | Uint8Array): Promise<void> {
    const bytes = typeof value === 'string' ? encodeUtf8(value) : value
    for (let offset = 0; offset < bytes.length; offset += LOCAL.chunkSize) await this.write(call, type, bytes.subarray(offset, offset + LOCAL.chunkSize))
  }
  private input(call: Call): AsyncIterable<Uint8Array> {
    return { [Symbol.asyncIterator]: () => ({
      next: async () => {
        if (call.abort.signal.aborted) throw call.abort.signal.reason
        if (call.ended) return { done: true, value: undefined }
        if (call.input) throw new Error('concurrent input readers')
        const pending = Promise.withResolvers<IteratorResult<Uint8Array>>()
        call.input = pending
        pending.promise.catch(noop)
        try { await this.write(call, LOCAL.frames.pull) } catch (error) { pending.reject(error); call.input = null }
        return pending.promise
      },
      return: async () => ({ done: true, value: undefined }),
    }) }
  }
  private async dispatch(call: Call): Promise<void> {
    const { request, context } = call
    try {
      const loader = await createLoader({ source: this.options.source(context), host: this.options.host, rpc: invocation => this.backend.invoke(context, invocation), importModule: workerModule })
      const input = this.input(call)
      const exitCode = await loader.dispatch(request.root, request.argv, {
        ...(request.live ? { stdinStream: input } : { stdin: input }),
        cwd: request.cwd, env: request.env, signal: call.abort.signal,
        stdout: value => this.output(call, LOCAL.frames.stdout, value),
        stderr: value => this.output(call, LOCAL.frames.stderr, value),
        onRunningHint: async hint => {
          if (context.jobId) this.options.send({ type: 'job_running_hint', jobId: context.jobId, invocationId: request.id, hint: hint ?? null })
        },
      })
      if (!Number.isInteger(exitCode) || exitCode < 0 || exitCode > 255) throw new Error('invalid command exit code')
      const body = new Uint8Array(4); new DataView(body.buffer).setUint32(0, exitCode)
      await this.write(call, LOCAL.frames.exit, body)
      call.completed = true
      // The client exits after consuming this frame and closes both sockets.
    } catch (error) {
      if (!call.abort.signal.aborted) await this.write(call, LOCAL.frames.error, encodeUtf8(errorMessage(error))).catch(noop)
      this.cancel(call)
    } finally {
      if (context.jobId) { try { this.options.send({ type: 'job_running_hint', jobId: context.jobId, invocationId: request.id, hint: null }) } catch { /* Offline. */ } }
    }
  }
  private async acceptLoop(): Promise<void> {
    while (!this.closed) {
      let socket: StreamSocket
      try { socket = await this.listener.accept() } catch { return }
      this.sockets.add(socket)
      void this.serve(socket)
    }
  }
  private async serve(socket: StreamSocket): Promise<void> {
    let call: Call | null = null
    let control = false
    try {
      for await (const frame of localFrames(socket.input)) {
        if (control) throw new Error('control connection only carries its handshake')
        if (!call) {
          if (frame.type === LOCAL.frames.manage) {
            const request = localManageSchema.parse(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(frame.body)))
            if (request.secret !== this.options.manageSecret) throw new Error('invalid management context')
            if (request.action === 'drain') await this.options.drain()
            await socket.write(localFrame(LOCAL.frames.ready))
            if (request.action === 'drain') void this.options.stop()
            return
          } else if (frame.type === LOCAL.frames.invoke) {
            const request = localInvokeSchema.parse(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(frame.body)))
            if (this.calls.has(request.id)) throw new Error('duplicate invocation')
            const context = this.options.contexts.get(request.context)
            call = { request, context, data: socket, control: null, abort: new AbortController(), writes: new SerialQueue(), input: null, ended: false, completed: false, watchTimer: setTimeout(() => { if (call && !call.control) this.cancel(call) }, 10_000) }
            this.calls.set(request.id, call)
            await this.write(call, LOCAL.frames.ready)
          } else if (frame.type === LOCAL.frames.watch) {
            const request = localWatchSchema.parse(JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(frame.body)))
            this.options.contexts.get(request.context)
            const candidate = this.calls.get(request.id)
            if (!candidate || candidate.control || candidate.request.context !== request.context) throw new Error('invalid invocation control context')
            call = candidate
            control = true; call.control = socket; clearTimeout(call.watchTimer)
            await socket.write(localFrame(LOCAL.frames.ready))
            void this.dispatch(call)
          } else throw new Error('invocation handshake required')
        } else if (frame.type === LOCAL.frames.input || frame.type === LOCAL.frames.inputEnd) {
          if (!call.control || !call.input || call.ended) throw new Error('unsolicited input')
          if (frame.body.length > LOCAL.chunkSize || (frame.type === LOCAL.frames.inputEnd && frame.body.length)) throw new Error('invalid input frame')
          call.ended = frame.type === LOCAL.frames.inputEnd
          call.input.resolve(call.ended ? { done: true, value: undefined } : { done: false, value: frame.body })
          call.input = null
        } else throw new Error('unexpected invocation frame')
      }
    } catch (error) {
      if (!call) await socket.write(localFrame(LOCAL.frames.error, encodeUtf8(errorMessage(error)))).catch(noop)
    } finally {
      this.sockets.delete(socket)
      if (call) this.cancel(call)
      socket.close()
    }
  }
}

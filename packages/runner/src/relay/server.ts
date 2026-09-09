import { listenUnix, type StreamSocket, type UnixListener } from '../machine'
import type {
  BackendToRunnerMessage,
  RunnerToBackendMessage
} from '@demicodes/runner-protocol'
import {
  LOCAL,
  localFrame,
  localFrames,
  localInvokeSchema,
  localWatchSchema,
  localManageSchema,
  type LocalInvoke
} from '@demicodes/runner-protocol/local'
import { createLoader, type ManifestSource } from '@demicodes/command-loader'
import type { Host } from '@demicodes/shell'
import { encodeUtf8, errorMessage, noop, SerialQueue } from '@demicodes/utils'
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
  pendingInput: {
    resolve(value: IteratorResult<Uint8Array>): void
    reject(error: unknown): void
  } | null
  inputEnded: boolean
  watchTimer: ReturnType<typeof setTimeout>
}

export class RelayServer {
  private readonly calls = new Map<string, Call>()
  private readonly sockets = new Set<StreamSocket>()
  private readonly backend: BackendCalls
  private closed = false

  static async listen(
    path: string,
    options: RelayServerOptions
  ): Promise<RelayServer> {
    const server = new RelayServer(
      await listenUnix(path, options.socketMode ?? 0o600),
      options
    )
    void server.acceptLoop()
    return server
  }

  private constructor(
    private readonly listener: UnixListener,
    private readonly options: RelayServerOptions,
  ) {
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
    for (const call of this.calls.values()) {
      if (call.context.jobId === jobId)
        this.cancel(call)
    }
  }

  cancelOwner(owner: string): void {
    for (const call of this.calls.values()) {
      if (call.context.owner === owner)
        this.cancel(call)
    }
  }

  handleReply(
    message: Extract<BackendToRunnerMessage, { type: 'rpc_pipes'
      | 'rpc_output'
      | 'rpc_exit' }>
  ): void {
    this.backend.handleReply(message)
  }

  private cancel(call: Call): void {
    this.calls.delete(call.request.id)
    clearTimeout(call.watchTimer)
    call.abort.abort(new Error('command cancelled'))
    call.pendingInput?.reject(call.abort.signal.reason)
    call.pendingInput = null
    call.data.close()
    call.control?.close()
  }

  private write(call: Call, type: number, body?: Uint8Array): Promise<void> {
    return call.writes.run(() => call.data.write(localFrame(type, body)))
  }

  private async output(
    call: Call,
    type: number,
    value: string | Uint8Array
  ): Promise<void> {
    const bytes = typeof value === 'string' ? encodeUtf8(value) : value
    for (let offset = 0; offset < bytes.length; offset += LOCAL.chunkSize) {
      await this.write(
        call,
        type,
        bytes.subarray(offset, offset + LOCAL.chunkSize)
      )
    }
  }

  private input(call: Call): AsyncIterable<Uint8Array> {
    return {
      [Symbol.asyncIterator]: () => ({
        next: () => this.pullInput(call),
        return: async () => ({ done: true, value: undefined }),
      }),
    }
  }

  private async pullInput(call: Call): Promise<IteratorResult<Uint8Array>> {
    if (call.abort.signal.aborted)
      throw call.abort.signal.reason
    if (call.inputEnded)
      return { done: true, value: undefined }
    if (call.pendingInput)
      throw new Error('concurrent input readers')

    const pending = Promise.withResolvers<IteratorResult<Uint8Array>>()
    call.pendingInput = pending
    // Cancellation can reject the input while the pull frame is still being written.
    pending.promise.catch(noop)
    try {
      await this.write(call, LOCAL.frames.pull)
    } catch (error) {
      pending.reject(error)
      call.pendingInput = null
    }
    return pending.promise
  }

  private async dispatch(call: Call): Promise<void> {
    const { request, context } = call
    try {
      const loader = await createLoader({
        source: this.options.source(context),
        host: this.options.host,
        rpc: invocation => this.backend.invoke(context, invocation),
        importModule: workerModule,
      })
      const input = this.input(call)
      const exitCode = await loader.dispatch(request.root, request.argv, {
        ...(request.live ? { stdinStream: input } : { stdin: input }),
        cwd: request.cwd,
        env: request.env,
        signal: call.abort.signal,
        stdout: value => this.output(call, LOCAL.frames.stdout, value),
        stderr: value => this.output(call, LOCAL.frames.stderr, value),
        onRunningHint: async hint => {
          this.sendRunningHint(call, hint ?? null)
        },
      })
      if (!Number.isInteger(exitCode) || exitCode < 0 || exitCode > 255)
        throw new Error('invalid command exit code')
      const body = new Uint8Array(4)
      new DataView(body.buffer).setUint32(0, exitCode)
      await this.write(call, LOCAL.frames.exit, body)
      // The client exits after consuming this frame and closes both sockets.
    } catch (error) {
      if (!call.abort.signal.aborted) {
        // The peer may already have disconnected; cancellation below still releases the call.
        await this.write(
          call,
          LOCAL.frames.error,
          encodeUtf8(errorMessage(error))
        ).catch(noop)
      }
      this.cancel(call)
    } finally {
      try {
        this.sendRunningHint(call, null)
      } catch {
        // A disconnected backend no longer has a live hint to clear.
      }
    }
  }

  private async acceptLoop(): Promise<void> {
    while (!this.closed) {
      let socket: StreamSocket
      try {
        socket = await this.listener.accept()
      } catch {
        // Closing the listener ends the accept loop.
        return
      }
      this.sockets.add(socket)
      void this.serve(socket)
    }
  }

  private sendRunningHint(call: Call, hint: string | null): void {
    if (!call.context.jobId)
      return
    this.options.send({
      type: 'job_running_hint',
      jobId: call.context.jobId,
      invocationId: call.request.id,
      hint,
    })
  }

  private async serve(socket: StreamSocket): Promise<void> {
    let call: Call | null = null
    let control = false
    try {
      for await (const frame of localFrames(socket.input)) {
        if (control)
          throw new Error('control connection only carries its handshake')

        if (call) {
          this.receiveInput(call, frame.type, frame.body)
          continue
        }

        switch (frame.type) {
          case LOCAL.frames.manage:
            await this.manage(socket, frame.body)
            return
          case LOCAL.frames.invoke:
            call = this.createCall(socket, frame.body)
            await this.write(call, LOCAL.frames.ready)
            break
          case LOCAL.frames.watch:
            call = this.attachControl(socket, frame.body)
            control = true
            await socket.write(localFrame(LOCAL.frames.ready))
            void this.dispatch(call)
            break
          default:
            throw new Error('invocation handshake required')
        }
      }
    } catch (error) {
      if (!call) {
        // Handshake errors can only be reported while the peer remains connected.
        await socket.write(localFrame(
          LOCAL.frames.error,
          encodeUtf8(errorMessage(error))
        )).catch(noop)
      }
    } finally {
      this.sockets.delete(socket)
      if (call)
        this.cancel(call)
      socket.close()
    }
  }

  private createCall(socket: StreamSocket, body: Uint8Array): Call {
    const request = localInvokeSchema.parse(decodeHandshake(body))
    if (this.calls.has(request.id))
      throw new Error('duplicate invocation')
    const context = this.options.contexts.get(request.context)
    const call: Call = {
      request,
      context,
      data: socket,
      control: null,
      abort: new AbortController(),
      writes: new SerialQueue(),
      pendingInput: null,
      inputEnded: false,
      watchTimer: setTimeout(() => {
        if (!call.control)
          this.cancel(call)
      }, 10_000),
    }
    this.calls.set(request.id, call)
    return call
  }

  private attachControl(socket: StreamSocket, body: Uint8Array): Call {
    const request = localWatchSchema.parse(decodeHandshake(body))
    this.options.contexts.get(request.context)
    const call = this.calls.get(request.id)
    if (!call || call.control || call.request.context !== request.context) {
      throw new Error('invalid invocation control context')
    }
    call.control = socket
    clearTimeout(call.watchTimer)
    return call
  }

  private receiveInput(call: Call, type: number, body: Uint8Array): void {
    if (type !== LOCAL.frames.input && type !== LOCAL.frames.inputEnd) {
      throw new Error('unexpected invocation frame')
    }
    if (!call.control || !call.pendingInput || call.inputEnded) {
      throw new Error('unsolicited input')
    }
    if (body.length > LOCAL.chunkSize
      || (type === LOCAL.frames.inputEnd && body.length > 0)) {
      throw new Error('invalid input frame')
    }

    call.inputEnded = type === LOCAL.frames.inputEnd
    call.pendingInput.resolve(call.inputEnded
      ? { done: true, value: undefined }
      : { done: false, value: body })
    call.pendingInput = null
  }

  private async manage(socket: StreamSocket, body: Uint8Array): Promise<void> {
    const request = localManageSchema.parse(decodeHandshake(body))
    if (request.secret !== this.options.manageSecret)
      throw new Error('invalid management context')
    if (request.action === 'drain')
      await this.options.drain()
    await socket.write(localFrame(LOCAL.frames.ready))
    if (request.action === 'drain')
      void this.options.stop()
  }
}

function decodeHandshake(body: Uint8Array): unknown {
  return JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(body))
}

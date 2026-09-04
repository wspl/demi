// The runner's end of the local relay (`runner.md` § The local relay): a
// Unix domain socket for command-mode processes — the manifest on a cache
// miss, and `rpc` invocations forwarded to the backend under the ids the
// job's environment carried. The process's pipe arrives as frames and
// leaves as the `PUT` the backend's `rpc_pipes` names; its stdout arrives
// as the `GET` body and goes back as frames; stderr and the exit ride the
// socket (`runner.md` § Pipes).
import { listenUnix, msgpackDecode, msgpackEncode, type StreamSocket, type UnixListener } from '../machine'
import type { BackendToRunnerMessage, PipeRef, RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { ByteChannel, createId, errorMessage, noop } from '@demicodes/utils'
import type { PipeEnds } from '../pipes'
import { frameOf, framesOf, relayRequestSchema, type RelayReply } from './protocol'

const codec = { encode: msgpackEncode, decode: msgpackDecode }

export interface RelayServerOptions {
  /** Forwards to the backend; throws while offline. */
  send(message: RunnerToBackendMessage): void
  manifest(): Promise<unknown | null>
  /** The device ends of the call's pipes. */
  pipes: PipeEnds
  /** The socket's mode (default 0600: the runner's own user); PID 1 opens it to the guest user it spawns jobs as. */
  socketMode?: number
}

interface Call {
  socket: StreamSocket
  /** The process's pipe, as its frames arrive; absent when the call declared none. */
  pipe: ByteChannel | null
}

export class RelayServer {
  private readonly calls = new Map<string, Call>()
  /** Per call, the replies still being written: the stdout body streams asynchronously and `rpc_exit` waits behind it. */
  private readonly writes = new Map<string, Promise<void>>()
  private closed = false

  static async listen(path: string, options: RelayServerOptions): Promise<RelayServer> {
    const listener = await listenUnix(path, options.socketMode ?? 0o600)
    const server = new RelayServer(listener, options)
    void server.acceptLoop()
    return server
  }

  private constructor(
    private readonly listener: UnixListener,
    private readonly options: RelayServerOptions,
  ) {}

  close(): void {
    this.closed = true
    this.listener.close()
    for (const call of this.calls.values()) call.socket.close()
    this.calls.clear()
  }

  /** The backend went away: every call in flight fails. */
  connectionLost(): void {
    for (const call of this.calls.values()) {
      call.pipe?.fail(new Error('runner disconnected'))
      void this.reply(call.socket, { type: 'error', message: 'runner disconnected' }).then(() => call.socket.close(), noop)
    }
    this.calls.clear()
  }

  handleReply(message: Extract<BackendToRunnerMessage, { type: 'rpc_pipes' | 'rpc_output' | 'rpc_exit' }>): void {
    const call = this.calls.get(message.callId)
    if (!call) return
    const { callId } = message
    if (message.type === 'rpc_pipes' && message.stdin) void this.sendPipe(call, message.stdin)
    const queued = this.writes.get(callId) ?? Promise.resolve()
    const next = queued.then(async () => {
      if (!this.calls.has(callId)) return
      switch (message.type) {
        case 'rpc_pipes':
          await this.receiveStdout(call, message.stdout)
          return
        case 'rpc_output':
          await this.reply(call.socket, { type: 'output', stream: 'stderr', bytes: message.bytes })
          return
        case 'rpc_exit':
          this.calls.delete(callId)
          this.writes.delete(callId)
          await this.reply(call.socket, { type: 'exit', exitCode: message.exitCode })
          call.socket.close()
          return
      }
    })
    this.writes.set(
      callId,
      next.catch(async (error: unknown) => {
        // A failed stdout pipe or a gone process ends the call on this side.
        this.calls.delete(callId)
        this.writes.delete(callId)
        await this.reply(call.socket, { type: 'error', message: errorMessage(error) }).catch(noop)
        call.socket.close()
      }),
    )
  }

  /** The process's pipe up to the backend; a failure is reported and is the far end's to judge. */
  private async sendPipe(call: Call, ref: PipeRef): Promise<void> {
    if (!call.pipe) {
      this.report(ref, new Error('the call declared no pipe'))
      return
    }
    try {
      await this.options.pipes.put(ref.url, call.pipe.stream())
      this.report(ref, null)
    } catch (error) {
      this.report(ref, error)
    }
  }

  /** The stdout body down into the process, in order with the other replies. */
  private async receiveStdout(call: Call, ref: PipeRef): Promise<void> {
    try {
      for await (const bytes of await this.options.pipes.get(ref.url)) {
        await this.reply(call.socket, { type: 'output', stream: 'stdout', bytes })
      }
      this.report(ref, null)
    } catch (error) {
      this.report(ref, error)
      throw error
    }
  }

  private report(ref: PipeRef, error: unknown | null): void {
    try {
      this.options.send(error === null ? { type: 'pipe_done', pipeId: ref.id, ok: true } : { type: 'pipe_done', pipeId: ref.id, ok: false, error: errorMessage(error) })
    } catch {
      // Offline: the backend already failed the pipe with the connection.
    }
  }

  private async acceptLoop(): Promise<void> {
    while (!this.closed) {
      let socket: StreamSocket
      try {
        socket = await this.listener.accept()
      } catch {
        return
      }
      void this.serve(socket)
    }
  }

  private async serve(socket: StreamSocket): Promise<void> {
    let callId: string | null = null
    let call: Call | null = null
    try {
      for await (const request of framesOf(socket.input, codec, relayRequestSchema)) {
        switch (request.type) {
          case 'manifest':
            await this.reply(socket, { type: 'manifest', manifest: await this.options.manifest() })
            socket.close()
            return
          case 'rpc': {
            callId = createId()
            const { type: _type, ...rest } = request
            call = { socket, pipe: request.stdin ? new ByteChannel() : null }
            this.calls.set(callId, call)
            this.options.send({ type: 'rpc_call', callId, ...rest })
            break
          }
          case 'pipe':
            // Backpressure: the frame is taken once the PUT consumed it.
            await call?.pipe?.push(request.bytes).catch(noop)
            break
          case 'pipe_end':
            call?.pipe?.close()
            break
          case 'stdin':
            if (callId) this.options.send({ type: 'rpc_stdin', callId, bytes: request.bytes })
            break
          case 'stdin_end':
            if (callId) this.options.send({ type: 'rpc_stdin_end', callId })
            break
        }
      }
      // The process went away before `pipe_end`: what it sent is all there is.
      call?.pipe?.fail(new Error('the command-mode process closed its pipe early'))
    } catch (error) {
      if (callId) this.calls.delete(callId)
      call?.pipe?.fail(error)
      await this.reply(socket, { type: 'error', message: errorMessage(error) }).catch(noop)
      socket.close()
    }
  }

  private reply(socket: StreamSocket, reply: RelayReply): Promise<void> {
    return socket.write(frameOf(codec, reply))
  }
}

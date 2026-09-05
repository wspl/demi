// The runner's end of the local relay (`runner.md` § The local relay): a
// Unix domain socket for command-mode processes — the manifest on a cache
// miss, and `rpc` invocations forwarded to the backend under the ids the
// job's environment carried. The process's pipe arrives as frames and
// leaves as the `PUT` the backend's `rpc_pipes` names; its stdout arrives
// as the `GET` body and goes back as frames; stderr and the exit ride the
// socket (`runner.md` § Pipes).
import { listenUnix, msgpackDecode, msgpackEncode, type StreamSocket, type UnixListener } from '../machine'
import type { BackendToRunnerMessage, PipeRef, RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { ByteChannel, createId, errorMessage, noop, SerialQueue } from '@demicodes/utils'
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
  jobId?: string
  socket: StreamSocket
  control: StreamSocket | null
  request: Extract<RunnerToBackendMessage, { type: 'rpc_call' }>
  /** The process's pipe, as its frames arrive; absent when the call declared none. */
  pipe: ByteChannel | null
  writes: SerialQueue
  stdout: Promise<void>
}

export class RelayServer {
  private readonly calls = new Map<string, Call>()
  private readonly hints = new Map<string, { jobId: string; socket: StreamSocket }>()
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
    for (const [callId, call] of this.calls) this.cancel(callId, call)
    for (const invocationId of this.hints.keys()) this.clearHint(invocationId)
  }

  /** The backend went away: every call in flight fails. */
  connectionLost(): void {
    for (const call of this.calls.values()) {
      call.pipe?.fail(new Error('runner disconnected'))
      call.control?.close()
      void this.replyCall(call, { type: 'error', message: 'runner disconnected' }).finally(() => call.socket.close()).catch(noop)
    }
    this.calls.clear()
    for (const invocationId of this.hints.keys()) this.clearHint(invocationId)
  }

  /** Job termination also cancels calls whose data connection is backpressured. */
  cancelJob(jobId: string): void {
    for (const [callId, call] of this.calls) {
      if (call.jobId === jobId) this.cancel(callId, call)
    }
    for (const [invocationId, hint] of this.hints) {
      if (hint.jobId === jobId) this.clearHint(invocationId)
    }
  }

  handleReply(message: Extract<BackendToRunnerMessage, { type: 'rpc_pipes' | 'rpc_output' | 'rpc_exit' }>): void {
    const call = this.calls.get(message.callId)
    if (!call) return
    const { callId } = message
    const failed = async (error: unknown) => {
      await this.replyCall(call, { type: 'error', message: errorMessage(error) }).catch(noop)
      this.cancel(callId, call)
    }
    switch (message.type) {
      case 'rpc_pipes':
        if (message.stdin) void this.sendPipe(call, message.stdin)
        call.stdout = this.receiveStdout(call, message.stdout)
        void call.stdout.catch(failed)
        break
      case 'rpc_output':
        void this.replyCall(call, { type: 'output', stream: 'stderr', bytes: message.bytes }).catch(failed)
        break
      case 'rpc_exit':
        void call.stdout.then(async () => {
          if (this.calls.get(callId) !== call) return
          await this.replyCall(call, { type: 'exit', exitCode: message.exitCode })
          this.calls.delete(callId)
          call.pipe?.close()
          call.control?.close()
          call.socket.close()
        }).catch(failed)
        break
    }
  }

  /** A process that leaves before its exit reply cancels the backend handler. */
  private cancel(callId: string, call: Call): void {
    if (this.calls.get(callId) !== call) return
    this.calls.delete(callId)
    call.pipe?.fail(new Error('rpc call cancelled'))
    try {
      this.options.send({ type: 'rpc_cancel', callId })
    } catch {
      // The backend already cancels calls when the runner connection drops.
    }
    call.socket.close()
    call.control?.close()
  }

  private clearHint(invocationId: string): void {
    const hint = this.hints.get(invocationId)
    if (!hint) return
    this.hints.delete(invocationId)
    try {
      this.options.send({ type: 'job_running_hint', jobId: hint.jobId, invocationId, hint: null })
    } catch {
      // Disconnect already cleared the backend's active jobs.
    }
    hint.socket.close()
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
        await this.replyCall(call, { type: 'output', stream: 'stdout', bytes })
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
    let watched: Call | null = null
    let hintId: string | null = null
    try {
      for await (const request of framesOf(socket.input, codec, relayRequestSchema)) {
        if (watched || hintId) throw new Error('a lifetime connection carries only one request')
        switch (request.type) {
          case 'running_hint': {
            if (call) throw new Error('a hint lifetime connection carries no data')
            hintId = createId()
            this.hints.set(hintId, { jobId: request.jobId, socket })
            this.options.send({ type: 'job_running_hint', jobId: request.jobId, invocationId: hintId, hint: request.hint })
            await this.reply(socket, { type: 'ready' })
            break
          }
          case 'watch': {
            if (call) throw new Error('a lifetime connection carries no data')
            watched = this.calls.get(request.callId) ?? null
            if (!watched || watched.control) throw new Error('no unbound invocation for this lifetime connection')
            watched.control = socket
            if (this.calls.get(request.callId) === watched) this.options.send(watched.request)
            await this.reply(socket, { type: 'ready' })
            break
          }
          case 'manifest':
            await this.reply(socket, { type: 'manifest', manifest: await this.options.manifest() })
            socket.close()
            return
          case 'rpc': {
            if (call) throw new Error('one rpc invocation per relay connection')
            if (this.calls.has(request.callId)) throw new Error('duplicate rpc call id')
            callId = request.callId
            if (!request.jobId) throw new Error('rpc requires a backend-dispatched job')
            const { type: _type, jobId, ...rest } = request
            call = { jobId, socket, control: null, request: { type: 'rpc_call', jobId, ...rest }, pipe: request.stdin ? new ByteChannel() : null, writes: new SerialQueue(), stdout: Promise.resolve() }
            this.calls.set(callId, call)
            await this.replyCall(call, { type: 'ready' })
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
      call?.pipe?.fail(error)
      if (call) await this.replyCall(call, { type: 'error', message: errorMessage(error) }).catch(noop)
      else await this.reply(socket, { type: 'error', message: errorMessage(error) }).catch(noop)
      socket.close()
    } finally {
      if (callId && call) this.cancel(callId, call)
      if (watched) this.cancel(watched.request.callId, watched)
      if (hintId) this.clearHint(hintId)
    }
  }

  private replyCall(call: Call, reply: RelayReply): Promise<void> {
    return call.writes.run(() => this.reply(call.socket, reply))
  }

  private reply(socket: StreamSocket, reply: RelayReply): Promise<void> {
    return socket.write(frameOf(codec, reply))
  }
}

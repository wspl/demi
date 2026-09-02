// The runner's end of the local relay (`runner.md` § The local relay): a
// Unix domain socket for command-mode processes — the manifest on a cache
// miss, and `rpc` invocations forwarded to the backend under the ids the
// job's environment carried, with stdin streamed in and stdout/stderr
// streamed back.
import { listenUnix, msgpackDecode, msgpackEncode, type StreamSocket, type UnixListener } from '@demicodes/host-runner'
import type { BackendToRunnerMessage, RunnerToBackendMessage } from '@demicodes/runner-protocol'
import { createId, errorMessage, noop } from '@demicodes/utils'
import { frameOf, framesOf, relayRequestSchema, type RelayReply } from './relay-protocol'

const codec = { encode: msgpackEncode, decode: msgpackDecode }

export interface RelayServerOptions {
  /** Forwards to the backend; throws while offline. */
  send(message: RunnerToBackendMessage): void
  manifest(): Promise<unknown | null>
}

export class RelayServer {
  private readonly calls = new Map<string, StreamSocket>()
  private closed = false

  static async listen(path: string, options: RelayServerOptions): Promise<RelayServer> {
    const listener = await listenUnix(path, 0o600)
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
    for (const socket of this.calls.values()) socket.close()
    this.calls.clear()
  }

  /** The backend went away: every call in flight fails. */
  connectionLost(): void {
    for (const socket of this.calls.values()) {
      void this.reply(socket, { type: 'error', message: 'runner disconnected' }).then(() => socket.close(), noop)
    }
    this.calls.clear()
  }

  handleReply(message: Extract<BackendToRunnerMessage, { type: 'rpc_output' | 'rpc_exit' }>): void {
    const socket = this.calls.get(message.callId)
    if (!socket) return
    if (message.type === 'rpc_output') {
      void this.reply(socket, { type: 'output', stream: message.stream, bytes: message.bytes }).catch(noop)
      return
    }
    this.calls.delete(message.callId)
    void this.reply(socket, { type: 'exit', exitCode: message.exitCode }).then(() => socket.close(), noop)
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
    try {
      for await (const request of framesOf(socket.input, codec, relayRequestSchema)) {
        switch (request.type) {
          case 'manifest':
            await this.reply(socket, { type: 'manifest', manifest: await this.options.manifest() })
            socket.close()
            return
          case 'rpc': {
            callId = createId()
            const { type: _type, ...call } = request
            this.calls.set(callId, socket)
            this.options.send({ type: 'rpc_call', callId, ...call })
            break
          }
          case 'stdin':
            if (callId) this.options.send({ type: 'rpc_stdin', callId, bytes: request.bytes })
            break
          case 'stdin_end':
            if (callId) this.options.send({ type: 'rpc_stdin_end', callId })
            break
        }
      }
    } catch (error) {
      if (callId) this.calls.delete(callId)
      await this.reply(socket, { type: 'error', message: errorMessage(error) }).catch(noop)
      socket.close()
    }
  }

  private reply(socket: StreamSocket, reply: RelayReply): Promise<void> {
    return socket.write(frameOf(codec, reply))
  }
}

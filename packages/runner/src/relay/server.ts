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
  /** `GET`s a brokered transfer with the device token (`rpc_transfer`). */
  download(url: string): Promise<AsyncIterable<Uint8Array>>
}

export class RelayServer {
  private readonly calls = new Map<string, StreamSocket>()
  /** Per call, the replies still being written: a transfer body streams asynchronously and `rpc_exit` waits behind it. */
  private readonly writes = new Map<string, Promise<void>>()
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

  handleReply(message: Extract<BackendToRunnerMessage, { type: 'rpc_output' | 'rpc_transfer' | 'rpc_exit' }>): void {
    const socket = this.calls.get(message.callId)
    if (!socket) return
    const { callId } = message
    const queued = this.writes.get(callId) ?? Promise.resolve()
    const next = queued.then(async () => {
      if (!this.calls.has(callId)) return
      switch (message.type) {
        case 'rpc_output':
          await this.reply(socket, { type: 'output', stream: message.stream, bytes: message.bytes })
          return
        case 'rpc_transfer':
          for await (const bytes of await this.options.download(message.url)) {
            await this.reply(socket, { type: 'output', stream: 'stdout', bytes })
          }
          return
        case 'rpc_exit':
          this.calls.delete(callId)
          this.writes.delete(callId)
          await this.reply(socket, { type: 'exit', exitCode: message.exitCode })
          socket.close()
          return
      }
    })
    this.writes.set(
      callId,
      next.catch(async (error: unknown) => {
        // A failed transfer or a gone process ends the call on this side.
        this.calls.delete(callId)
        this.writes.delete(callId)
        await this.reply(socket, { type: 'error', message: errorMessage(error) }).catch(noop)
        socket.close()
      }),
    )
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

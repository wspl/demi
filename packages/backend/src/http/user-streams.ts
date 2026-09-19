import type { ServerWebSocket, WebSocketHandler } from 'bun'
import { Hono, type Context } from 'hono'
import type { UpgradeWebSocket, WSContext, WSEvents } from 'hono/ws'
import type { ArtifactResolver, NativePackage } from '@demicodes/command-protocol'
import {
  RemoteServiceError,
  type Pipe,
  type PipeBroker,
  type RemoteServiceStream
} from '@demicodes/host-remote'
import { errorCode, errorMessage } from '@demicodes/utils'
import type { AuthEnv } from '../auth/identity'
import {
  HostAccessRefused,
  type ConversationTargets,
  type UserStreamAccess
} from '../conversation/target'
import { buildCommandContext } from '../runner/command-context'
import type { ControlService } from '../storage/control'

/**
 * A declared user stream (`native-runtime.md` § User streams): the operation
 * it opens in the package the conversation's jobs use, fixed for the
 * program's lifetime.
 */
export interface UserStreamDeclaration {
  package: NativePackage
  operation: string
  resolveArtifact: ArtifactResolver
}

/**
 * The page stops being sent pictures while its socket holds this much: the
 * backend then pulls nothing more from the Host (`browser-live-view.md`
 * § Backpressure).
 */
const SEND_BUFFER_BYTES = 256 * 1024

/**
 * `WS /api/conversations/:id/streams/:name` and
 * `POST /api/conversations/:id/activity` (`web-api.md` § User streams).
 */
export function userStreamRoutes(options: {
  control: ControlService
  targets: Pick<ConversationTargets, 'stream' | 'withHost'>
  pipes: PipeBroker
  upgradeWebSocket: UpgradeWebSocket
  streams: ReadonlyMap<string, UserStreamDeclaration>
  /** The product's origin when it is served under another host name. */
  publicOrigin?: string
}): Hono<AuthEnv> {
  const { control, targets, pipes } = options
  const app = new Hono<AuthEnv>()

  app.get('/:id/streams/:name', async (c, next) => {
    // The stream operates a browser signed in to the user's sites.
    if (!fromProduct(c, options.publicOrigin))
      return c.json({ code: 'forbidden_origin', message: 'A user stream opens only from the product' }, 403)
    const declaration = options.streams.get(c.req.param('name'))
    if (!declaration)
      return c.json({ code: 'unknown_stream', message: 'No stream has that name' }, 404)
    const id = c.req.param('id')
    const conversation = await control.getConversation(id)
    if (!conversation || conversation.userId !== c.get('user').id)
      return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    if (c.req.header('upgrade')?.toLowerCase() !== 'websocket')
      return c.json({ code: 'upgrade_required', message: 'A user stream is a WebSocket' }, 426)
    let access: UserStreamAccess
    try {
      access = await targets.stream(id, c.req.raw.signal)
    } catch (error) {
      if (error instanceof HostAccessRefused)
        return c.json({ code: error.code, message: error.message }, 409)
      throw error
    }
    const input = pipes.open(undefined, { deviceId: access.deviceId })
    const output = pipes.open({ deviceId: access.deviceId })
    // Settled pipes reject at the stream's end; the relay observes the ends it uses.
    input.done.catch(() => {})
    output.done.catch(() => {})
    let service: RemoteServiceStream
    try {
      service = await access.host.services.open({
        context: await buildCommandContext(control, id, { kind: 'user' }),
        package: declaration.package,
        operation: declaration.operation,
        cwd: access.cwd,
        input: input.ref(),
        output: output.ref(),
        resolveArtifact: declaration.resolveArtifact,
      })
    } catch (error) {
      pipes.fail(input.id, 'stream never opened')
      pipes.fail(output.id, 'stream never opened')
      access.release()
      if (error instanceof RemoteServiceError)
        return c.json({ code: 'stream_failed', message: error.message }, 502)
      if (errorCode(error) === 'ERUNNEROFFLINE')
        return c.json({ code: 'device_offline', message: 'The device has no live runner' }, 409)
      throw error
    }
    const relay = new StreamRelay(pipes, input, output, service, access)
    const response = await options.upgradeWebSocket(() => relay.events())(c, next)
    // The upgrade itself failed: nothing will ever end the stream but this.
    if (!response)
      relay.end(1011, 'upgrade_failed')
    return response
  })

  app.post('/:id/activity', async (c) => {
    const id = c.req.param('id')
    const conversation = await control.getConversation(id)
    if (!conversation || conversation.userId !== c.get('user').id)
      return c.json({ code: 'conversation_not_found', message: 'No such conversation' }, 404)
    try {
      // One user operation is admitted and ends at once, which restarts the
      // idle window (`resource-lifecycle.md` § Activity).
      await targets.withHost(id, async () => {}, { signal: c.req.raw.signal })
    } catch (error) {
      if (error instanceof HostAccessRefused)
        return c.json({ code: error.code, message: error.message }, error.code === 'host_not_attached' ? 404 : 409)
      if (errorCode(error) === 'ERUNNEROFFLINE')
        return c.json({ code: 'device_offline', message: 'The device has no live runner' }, 409)
      throw error
    }
    return c.body(null, 204)
  })

  return app
}

/**
 * Adds the send-buffer drain Hono's Bun adapter leaves out: it reaches the
 * socket's own `onDrain`, where a relay waits to send more.
 */
export function withDrain<T extends { events: unknown }>(
  websocket: WebSocketHandler<T>
): WebSocketHandler<T> {
  return {
    ...websocket,
    drain(ws) {
      (ws.data.events as { onDrain?: () => void }).onDrain?.()
    },
  }
}

/** The upgrade comes from a page on the product's own origin. */
function fromProduct(c: Context, publicOrigin: string | undefined): boolean {
  const origin = c.req.header('origin')
  if (!origin)
    return false
  if (origin === publicOrigin)
    return true
  try {
    return new URL(origin).host === c.req.header('host')
  } catch {
    return false
  }
}

/**
 * One open stream between the page's socket and the Host's two pipes: the
 * page's binary messages go into `input` in order, `output` goes to the page
 * only as fast as the page takes it. Whichever side ends first ends both.
 */
class StreamRelay {
  private socket: WSContext | null = null
  private writes = Promise.resolve()
  private drained: (() => void) | null = null
  /** How the stream ended, once it has. */
  private closing: { code: number; reason: string } | null = null
  private readonly writer

  constructor(
    private readonly pipes: PipeBroker,
    private readonly input: Pipe,
    private readonly output: Pipe,
    private readonly service: RemoteServiceStream,
    private readonly access: UserStreamAccess,
  ) {
    this.writer = input.writer()
    access.signal.addEventListener(
      'abort',
      () => this.end(4000, 'conversation_changed'),
      { once: true }
    )
    if (access.signal.aborted)
      this.end(4000, 'conversation_changed')
  }

  events(): WSEvents & { onDrain(): void } {
    return {
      onOpen: (_event, ws) => {
        this.socket = ws
        if (this.closing) {
          ws.close(this.closing.code, this.closing.reason)
          return
        }
        void this.deliver()
      },
      onMessage: (event, ws) => {
        if (typeof event.data === 'string') {
          ws.close(1003, 'binary_only')
          return
        }
        const bytes = event.data instanceof ArrayBuffer
          ? new Uint8Array(event.data)
          : new Uint8Array(event.data as ArrayBufferLike)
        // In order, each after the pipe took the one before.
        this.writes = this.writes
          .then(() => this.writer.write(bytes))
          .catch(() => this.end(1011, 'host_unreachable'))
      },
      onClose: () => {
        this.socket = null
        this.end(1000, 'page_closed')
      },
      onDrain: () => {
        const drained = this.drained
        this.drained = null
        drained?.()
      },
    }
  }

  /** The Host's bytes to the page; the invocation's completion ends them. */
  private async deliver(): Promise<void> {
    try {
      for await (const chunk of this.output.stream()) {
        const socket = this.socket
        if (!socket || this.closing)
          return
        const raw = socket.raw as ServerWebSocket<unknown>
        raw.send(chunk)
        while (!this.closing && raw.getBufferedAmount() > SEND_BUFFER_BYTES)
          await this.drain()
      }
      this.end(1000, 'completed')
    } catch (error) {
      if (!this.closing)
        console.warn(`user stream output failed: ${errorMessage(error)}`)
      this.end(1011, 'host_unreachable')
    }
  }

  /** Waits for the page to take buffered bytes, or looks again shortly. */
  private drain(): Promise<void> {
    return new Promise((resolve) => {
      const timer = setTimeout(done, 50)
      function done() {
        clearTimeout(timer)
        resolve()
      }
      this.drained = done
    })
  }

  /** Ends the stream once: the socket closes with the reason, the pipes end, the access is released. */
  end(code: number, reason: string): void {
    if (this.closing)
      return
    this.closing = { code, reason }
    this.drained?.()
    this.socket?.close(code, reason)
    this.pipes.fail(this.input.id, `user stream ended: ${reason}`)
    if (reason !== 'completed')
      this.pipes.fail(this.output.id, `user stream ended: ${reason}`)
    this.service.close()
    this.access.release()
  }
}

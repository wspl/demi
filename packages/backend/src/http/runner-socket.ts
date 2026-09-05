import { Hono } from 'hono'
import type { UpgradeWebSocket } from 'hono/ws'
import type { RunnerRegistry, RunnerSocketHandle } from '../runner/registry'

/**
 * `WS /api/runner` — the one outbound socket every runner holds. All protocol
 * logic (hello/claim/liveness/Host RPC routing) lives in the registry; this
 * route only adapts the WebSocket events.
 */
export function runnerSocketRoutes(options: { registry: RunnerRegistry; upgradeWebSocket: UpgradeWebSocket }): Hono {
  const { registry, upgradeWebSocket } = options
  const app = new Hono()

  app.get(
    '/',
    upgradeWebSocket(() => {
      let handle: RunnerSocketHandle | null = null
      return {
        onOpen(_event, ws) {
          handle = registry.openSocket({
            send: (frame) => {
              ws.send(frame as Uint8Array<ArrayBuffer>)
            },
            close: () => {
              ws.close()
            },
          })
        },
        onMessage(event) {
          // Frames are binary MessagePack; a text frame is malformed and the
          // registry closes the socket over it.
          const data = event.data
          handle?.handleMessage(data instanceof ArrayBuffer ? new Uint8Array(data) : new Uint8Array(0))
        },
        onClose() {
          handle?.handleClose()
        },
      }
    }),
  )

  return app
}

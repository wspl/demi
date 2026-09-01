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
            send: (text) => {
              ws.send(text)
            },
            close: () => {
              ws.close()
            },
          })
        },
        onMessage(event) {
          handle?.handleMessage(typeof event.data === 'string' ? event.data : String(event.data))
        },
        onClose() {
          handle?.handleClose()
        },
      }
    }),
  )

  return app
}

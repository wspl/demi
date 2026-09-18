import type { Server } from 'bun'
import type { Context } from 'hono'

/**
 * The Bun server a request arrived on. Hono hands the fetch handler's second
 * argument over as `c.env`, which it types only through an app's `Bindings`;
 * this app runs under `Bun.serve`, and without it in route tests that call
 * `app.request`, so the server may be absent.
 */
export function bunServerOf(c: Context): Server<unknown> | undefined {
  return c.env as Server<unknown> | undefined
}

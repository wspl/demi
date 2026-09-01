import { Hono } from 'hono'
import type { AnthropicPassthrough } from '../llm/passthrough'

/** `/api/passthrough/anthropic/*` — the Claude Code CLI's `ANTHROPIC_BASE_URL`. */
export function passthroughRoutes(options: { anthropic: AnthropicPassthrough }): Hono {
  const app = new Hono()
  app.all('/anthropic/*', (c) => {
    const subpath = c.req.path.replace(/^.*\/api\/passthrough\/anthropic/, '')
    return options.anthropic.handle(c.req.raw, subpath)
  })
  return app
}

import { Hono } from 'hono'
import { STUB_USER } from '../auth/identity'

/** `/api/auth/*` — stub identity until the real login surface lands (M7). */
export function authRoutes(): Hono {
  const app = new Hono()
  app.get('/me', (c) => c.json({ user: STUB_USER }))
  app.post('/login', (c) => c.json({ user: STUB_USER }))
  app.post('/logout', (c) => c.body(null, 204))
  return app
}

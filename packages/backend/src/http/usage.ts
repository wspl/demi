import { Hono } from 'hono'
import type { AuthEnv, InstanceMode } from '../auth/identity'
import { requireAdmin } from './authenticate'
import type { ControlService, UsageRow } from '../storage/control'

/**
 * `GET /api/usage` — the caller's ledger, aggregated at query time
 * (`provider × model`): request count and token sums. `GET
 * /api/usage/instance` — in shared mode, the admin's view of the instance's
 * ledger, the same aggregation per user.
 */
export function usageRoutes(options: { control: ControlService; mode: InstanceMode }): Hono<AuthEnv> {
  const { control, mode } = options
  const app = new Hono<AuthEnv>()

  app.get('/', async (c) => {
    const rows = await control.listUsage(c.get('user').id)
    return c.json({ totals: aggregate(rows) })
  })

  app.get('/instance', requireAdmin, async (c) => {
    if (mode !== 'shared') return c.json({ code: 'forbidden', message: 'The instance ledger is a shared-mode view' }, 403)
    const users = await control.listUsers()
    const byUser = new Map<string, UsageRow[]>()
    for (const row of await control.listAllUsage()) byUser.set(row.userId, [...(byUser.get(row.userId) ?? []), row])
    return c.json({
      users: users.map((user) => ({ userId: user.id, username: user.username, totals: aggregate(byUser.get(user.id) ?? []) })),
    })
  })

  return app
}

function aggregate(rows: UsageRow[]) {
  const groups = new Map<string, ReturnType<typeof emptyGroup>>()
  for (const row of rows) {
    const key = `${row.providerId} ${row.modelId}`
    let group = groups.get(key)
    if (!group) {
      group = emptyGroup(row.providerId, row.modelId)
      groups.set(key, group)
    }
    group.requests += 1
    group.inputTokens += row.inputTokens
    group.outputTokens += row.outputTokens
    group.cacheReadTokens += row.cacheReadTokens
    group.cacheWriteTokens += row.cacheWriteTokens
  }
  return [...groups.values()]
}

function emptyGroup(providerId: string, modelId: string) {
  return {
    providerId,
    modelId,
    requests: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
  }
}

import { Hono } from 'hono'
import { STUB_USER } from '../auth/identity'
import type { ControlService, UsageRow } from '../storage/control'

/**
 * `GET /api/usage` — the caller's ledger, aggregated at query time
 * (`connection × model`): request count and token sums, plus the raw total.
 */
export function usageRoutes(options: { control: ControlService }): Hono {
  const app = new Hono()

  app.get('/', async (c) => {
    const rows = await options.control.listUsage(STUB_USER.id)
    return c.json({ totals: aggregate(rows) })
  })

  return app
}

function aggregate(rows: UsageRow[]) {
  const groups = new Map<string, ReturnType<typeof emptyGroup>>()
  for (const row of rows) {
    const key = `${row.connectionId} ${row.modelId}`
    let group = groups.get(key)
    if (!group) {
      group = emptyGroup(row.connectionId, row.modelId)
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

function emptyGroup(connectionId: string, modelId: string) {
  return {
    connectionId,
    modelId,
    requests: 0,
    inputTokens: 0,
    outputTokens: 0,
    cacheReadTokens: 0,
    cacheWriteTokens: 0,
  }
}

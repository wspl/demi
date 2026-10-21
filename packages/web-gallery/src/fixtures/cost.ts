import { ref } from 'vue'
import type { CacheChart, CacheMiss, ChartPoint } from '@demicodes/protocol'
import type { AgentPrice, ConversationCostView, CostShown } from '@demicodes/web-ui/agent/context-cost'
import { type GlobalProfiles, workflowProfiles } from '@demicodes/web-ui/agent/workflow-menu'
import { demoSubagentProfiles } from './subagent-profiles'
import { MINUTE } from './time'

/** A deterministic sequence in [0, 1), so every load draws the same conversation. */
function sequence(seed: number): () => number {
  let state = seed
  return () => {
    state = (state * 1_664_525 + 1_013_904_223) >>> 0
    return state / 4_294_967_296
  }
}

/** Claude Opus's prices, dollars per million tokens. */
const RATE = { input: 5, cacheWrite: 6.25, cacheRead: 0.5, output: 25 }

/**
 * The calls a compaction comes before, and the misses with their causes;
 * past the first 180, a giant conversation compacts every 105 calls and
 * misses every 97th.
 */
const COMPACTED_BEFORE = new Set([31, 68, 131])
const MISSES = new Map<number, CacheMiss>([
  [7, 'not_cached'],
  [75, 'cache_expired'],
  [137, 'system_prompt_changed'],
])
const LATER_COMPACTIONS = 105
const LATER_MISSES = 97
function compactedBefore(number: number): boolean {
  return COMPACTED_BEFORE.has(number) || (number > 180 && number % LATER_COMPACTIONS === 0)
}
function missAt(number: number): CacheMiss | null {
  return MISSES.get(number) ?? (number > 180 && number % LATER_MISSES === 0 ? 'not_cached' : null)
}
/** How long before the miss its pause was: past the vendor's five minutes. */
const PAUSE_BEFORE = new Map([[75, 9 * MINUTE]])

/**
 * The user's subagent settings as Settings › Subagents shows them, Subagents
 * on; archivist is disabled.
 */
export const GLOBAL_PROFILES: GlobalProfiles = { enabled: true, profiles: demoSubagentProfiles() }

/** The profiles Global Workflow offers the card specimens. */
export const OFFERED_PROFILES: readonly string[] = workflowProfiles({ type: 'default' }, { state: 'nowhere' }, GLOBAL_PROFILES)

/** Eight profiles a workflow offers, more than the Profiles part shows. */
export const MANY_PROFILES: readonly string[] = ['explore', 'reviewer', 'scout', 'planner', 'tester', 'writer', 'translator', 'designer']

/**
 * A subagent's price as the fixture states it: with profiles, one of them
 * since disabled, which has no row; with each of `MANY_PROFILES`; or without
 * any.
 */
const SUBAGENTS: Record<'profiles' | 'many' | 'plain', AgentPrice[]> = {
  profiles: [
    { agent: 1, profile: 'explore', price: 1.32 },
    { agent: 2, profile: 'explore', price: 1.32 },
    { agent: 3, profile: 'reviewer', price: 1.32 },
    { agent: 4, profile: 'archivist', price: 0.88 },
    { agent: 5, profile: null, price: 0.56 },
  ],
  many: MANY_PROFILES.map((profile, index) => ({ agent: index + 1, profile, price: 0.24 + ((index * 5) % 8) * 0.31 })),
  plain: [
    { agent: 1, profile: null, price: 1.32 },
    { agent: 2, profile: null, price: 1.32 },
    { agent: 3, profile: null, price: 0.88 },
  ],
}

/**
 * How a specimen's chart stands, as the backend would send it for a
 * conversation whose main agent made `calls` turn calls: each point of `size`
 * calls, from the call `first`.
 */
export interface ChartShape {
  calls: number
  size: number
  first?: number
}

/** The shapes of the conversations the specimens show. */
export const SHAPES = {
  /** Three hours and a half, five calls a point. */
  long: { calls: 180, size: 5 },
  /** Just started: the rate alone. */
  young: { calls: 2, size: 1 },
  short: { calls: 12, size: 1 },
  /** One call a point to the last. */
  forty: { calls: 40, size: 1 },
  thirty: { calls: 30, size: 1 },
  sixty: { calls: 60, size: 2 },
  /** The latest 400 of a thousand calls, ten a point. */
  giant: { calls: 1_000, size: 10, first: 601 },
} satisfies Record<string, ChartShape>

/**
 * A long conversation's cost and cache chart, as the agents' metadata and the
 * backend's chart bring them: calls about a minute apart, compactions and
 * misses of several causes at fixed calls, and subagents started with
 * profiles, without them, or none.
 *
 * The chart's points are made up whole, as the backend would answer: a point
 * that holds the first call, a miss or the first call after a compaction
 * reads less from the cache.
 */
export function costFixture(subagents: keyof typeof SUBAGENTS | 'none', shape: ChartShape = SHAPES.long, priced = true): { cost: ConversationCostView, chart: CacheChart } {
  const random = sequence(7)
  const { calls, size } = shape
  const first = shape.first ?? 1
  // A giant conversation started long enough ago for its calls to end by now.
  const start = Date.now() - Math.max(212, calls * 1.25) * MINUTE
  /** When the call `number` started: about 65 seconds after the one before, or a pause. */
  const startOf = (number: number) =>
    start + number * 65_000 + [...PAUSE_BEFORE].reduce((sum, [at, pause]) => sum + (number >= at ? pause : 0), 0)
  const points: ChartPoint[] = []
  let main = 0
  for (let from = first; from <= calls && calls >= 4; from += size) {
    const to = Math.min(calls, from + size - 1)
    const count = to - from + 1
    const numbers = Array.from({ length: count }, (_, index) => from + index)
    const misses = [...new Set(numbers.flatMap((number) => missAt(number) ?? []))]
    const afterCompaction = numbers.some(compactedBefore)
    // A call that starts a prompt or misses reads almost nothing.
    const fresh = numbers.filter((number) => number === 1 || compactedBefore(number) || missAt(number) !== null).length
    const rate = (0.9 + random() * 0.08) * (count - fresh) / count + 0.02 * fresh / count
    const price = count * (0.02 + random() * 0.03) + fresh * 0.18
    main += price
    points.push({
      first: from,
      last: to,
      rate,
      from: new Date(startOf(from)).toISOString(),
      to: new Date(startOf(to)).toISOString(),
      misses,
      afterCompaction,
      price: priced ? price : null,
      rewritten: count === 1 ? Math.round(fresh ? 24_000 + random() * 12_000 : 400 + random() * 2_000) : null,
    })
  }
  // The calls before the window, which the chart no longer draws.
  main += (first - 1) * 0.04
  if (calls < 4)
    main = calls * 0.12
  const others = subagents === 'none' ? [] : SUBAGENTS[subagents]
  const elsewhere = others.reduce((total, agent) => total + agent.price, 0)
  const total = main + elsewhere
  // The price by kind, in the shares of a conversation that reads its cache
  // well.
  const share = { input: 0.08, cacheWrite: 0.34, cacheRead: 0.18, output: 0.4 }
  const tokens = (kind: keyof typeof share) => Math.round((total * share[kind] * 1e6) / RATE[kind])
  const unpriced = { total: null, input: null, cacheWrite: null, cacheRead: null, output: null }
  const cost: ConversationCostView = {
    calls: calls + others.length,
    totals: {
      inputTokens: tokens('input'),
      cacheWriteTokens: tokens('cacheWrite'),
      cacheReadTokens: tokens('cacheRead'),
      outputTokens: tokens('output'),
      price: !priced ? unpriced : {
        total,
        input: total * share.input,
        cacheWrite: total * share.cacheWrite,
        cacheRead: total * share.cacheRead,
        output: total * share.output,
      },
    },
    agents: priced ? [{ agent: 0, profile: null, price: main }, ...others] : [],
  }
  // The rate over the window, the drawn points' calls; a conversation of
  // fewer than four calls has no points, and the first call read nothing.
  const window = points.reduce((sum, point) => sum + (point.last - point.first + 1), 0)
  const rate = points.length === 0
    ? (calls > 1 ? 0.46 : 0)
    : points.reduce((sum, point) => sum + point.rate * (point.last - point.first + 1), 0) / window
  return { cost, chart: { rate, size, points } }
}

/**
 * What a card specimen holds of its conversation's cost, as the product's
 * store does: the cost from the agents' metadata, and the cache chart its
 * node brought, so the card opens whole.
 */
export function useCardCost(subagents: keyof typeof SUBAGENTS | 'none', { shape = SHAPES.long, priced = true }: { shape?: ChartShape, priced?: boolean } = {}) {
  const fixture = costFixture(subagents, shape, priced)
  const cost = ref<CostShown>({ cost: fixture.cost, chart: fixture.chart })
  return { cost }
}

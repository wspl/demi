import type { AgentCost, CostCall, CostPrice, CostTotals } from '@demicodes/protocol'
import type { ShownError } from '../infra/errors'
import { t } from './locales/text'

/**
 * A conversation's cost as the context card shows it (`usage-and-quota.md`
 * § A conversation's cost): the backend's `GET /conversations/:id/cost`,
 * which the page reads with the conversation as it opens and to which it
 * adds each `cost` frame while the conversation is open.
 */
export interface ConversationCostView {
  /** The number of the conversation's ledger rows the cost includes. */
  rows: number
  totals: CostTotals
  agents: AgentCost[]
  calls: CostCall[]
}

/** The cost of a conversation that has no ledger row yet, as one the page just created. */
export const NO_COST: ConversationCostView = {
  rows: 0,
  totals: {
    inputTokens: 0,
    cacheWriteTokens: 0,
    cacheReadTokens: 0,
    outputTokens: 0,
    price: { total: null, input: null, cacheWrite: null, cacheRead: null, output: null },
  },
  agents: [],
  calls: [],
}

/**
 * The cost as the page holds it while the conversation is open: the read's
 * answer with each frame added, or the failure of the read, which the
 * section says with Retry.
 */
export type CostState =
  | { status: 'read', cost: ConversationCostView }
  | { status: 'failed', error: ShownError }

/** What one new ledger row adds, as the `cost` frame carries it. */
export interface CostAddition {
  /** The number of the conversation's ledger rows up to the frame's own. */
  rows: number
  totals: CostTotals
  agent: AgentCost | null
  call: CostCall | null
}

/** A sum of prices that may have none: null while no part has one. */
function addPrice(a: number | null, b: number | null): number | null {
  return a === null ? b : b === null ? a : a + b
}

function addPrices(a: CostPrice, b: CostPrice): CostPrice {
  return {
    total: addPrice(a.total, b.total),
    input: addPrice(a.input, b.input),
    cacheWrite: addPrice(a.cacheWrite, b.cacheWrite),
    cacheRead: addPrice(a.cacheRead, b.cacheRead),
    output: addPrice(a.output, b.output),
  }
}

/** `cost` with the row `addition` adds: its totals, its agent's price and its call. */
export function addCost(cost: ConversationCostView, addition: CostAddition): ConversationCostView {
  const totals: CostTotals = {
    inputTokens: cost.totals.inputTokens + addition.totals.inputTokens,
    cacheWriteTokens: cost.totals.cacheWriteTokens + addition.totals.cacheWriteTokens,
    cacheReadTokens: cost.totals.cacheReadTokens + addition.totals.cacheReadTokens,
    outputTokens: cost.totals.outputTokens + addition.totals.outputTokens,
    price: addPrices(cost.totals.price, addition.totals.price),
  }
  const added = addition.agent
  const agents = added === null
    ? cost.agents
    : cost.agents.some((agent) => agent.agent === added.agent)
      ? cost.agents.map((agent) => (agent.agent === added.agent ? { ...agent, price: agent.price + added.price } : agent))
      : [...cost.agents, added].sort((a, b) => a.agent - b.agent)
  return {
    rows: addition.rows,
    totals,
    agents,
    calls: addition.call === null ? cost.calls : [...cost.calls, addition.call],
  }
}

/**
 * What a `cost` frame is to the cost the page holds, by the rows each
 * counts (`usage-and-quota.md` § Reading it): `next` when its row follows
 * the held ones and the page adds it, `held` when the cost includes it
 * already, as a frame that crossed the read, and `gap` when rows before it
 * are missing, as frames lost while the connection was down, and the page
 * reads the cost again.
 */
export function costStep(cost: ConversationCostView, addition: CostAddition): 'next' | 'held' | 'gap' {
  if (addition.rows <= cost.rows)
    return 'held'
  return addition.rows === cost.rows + 1 ? 'next' : 'gap'
}

/** The tokens of a call's prompt: uncached, written to the cache and read from it. */
function prompt(call: Pick<CostCall, 'inputTokens' | 'cacheWriteTokens' | 'cacheReadTokens'>): number {
  return call.inputTokens + call.cacheWriteTokens + call.cacheReadTokens
}

/** The share of a call's prompt the vendor read from its cache; 0 for an empty prompt. */
export function callHitRate(call: CostCall): number {
  const whole = prompt(call)
  return whole === 0 ? 0 : call.cacheReadTokens / whole
}

/** The hit rate over `calls`; null without a call that sent a prompt. */
export function hitRate(calls: readonly CostCall[]): number | null {
  const whole = calls.reduce((sum, call) => sum + prompt(call), 0)
  return whole === 0 ? null : calls.reduce((sum, call) => sum + call.cacheReadTokens, 0) / whole
}

/**
 * The turn calls of the main conversation from which the cache chart, and
 * its switch, shows: with fewer, a line says nothing a user can read and
 * the rate alone says it.
 */
export const CHART_CALLS = 4

/**
 * The most recent turn calls the chart draws, and the cache hit rate counts:
 * a long conversation's whole run would crowd the plot past reading, and
 * what a user checks is whether the cache serves the conversation now.
 */
export const RECENT_CALLS = 50

/**
 * The main conversation's last `RECENT_CALLS` turn calls, and how many came
 * before them, so each keeps its number in the conversation.
 */
export function recentCalls(calls: readonly CostCall[]): { calls: readonly CostCall[], before: number } {
  const before = Math.max(0, calls.length - RECENT_CALLS)
  return { calls: calls.slice(before), before }
}

/** Whether the conversation made any call: a conversation with none has no Cost section. */
export function hasCost(cost: ConversationCostView | null): cost is ConversationCostView {
  if (cost === null)
    return false
  const { totals } = cost
  return cost.calls.length > 0
    || totals.price.total !== null
    || totals.inputTokens + totals.cacheWriteTokens + totals.cacheReadTokens + totals.outputTokens > 0
}

/** One row of the Profiles part. */
export interface ProfileRow {
  key: string
  label: string
  price: number
}

/**
 * The Profiles part's rows: the main conversation first, for scale, then
 * the sum of each enabled profile's subagents, largest first. A subagent
 * started with no profile, or with one not in `enabled` (since disabled or
 * deleted), has no row; with no profile row there is no part, and no row.
 */
export function profileRows(agents: readonly AgentCost[], enabled: readonly string[]): ProfileRow[] {
  const sums = new Map<string, number>()
  for (const agent of agents) {
    if (agent.agent !== 0 && agent.profile !== null && enabled.includes(agent.profile))
      sums.set(agent.profile, (sums.get(agent.profile) ?? 0) + agent.price)
  }
  if (sums.size === 0)
    return []
  const main = agents.find((agent) => agent.agent === 0)
  const profiles = [...sums]
    .map(([profile, price]) => ({ key: `profile:${profile}`, label: profile, price }))
    .sort((a, b) => b.price - a.price)
  return [...(main ? [{ key: 'main', label: t('context.cost.mainConversation'), price: main.price }] : []), ...profiles]
}
